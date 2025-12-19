use {
	crate::{
		config::get_config,
		engine::{EventManager, MatchEngine, get_manager},
		output::{publish_order_change_to_store, publish_to_processor},
		types::OrderBookControl,
	},
	chrono::Utc,
	common::{
		consts::{EVENT_INPUT_MSG_KEY, EVENT_INPUT_STREAM, ORDER_INPUT_MSG_KEY, ORDER_INPUT_STREAM},
		engine_types::{CancelOrderMessage, EventInputMessage, OrderInputMessage, SubmitOrderMessage},
		event_types::EngineMQEventCreate,
		processor_types::{OrderRejected, ProcessorMessage},
		store_types::OrderChangeEvent,
	},
	redis::AsyncCommands,
	serde_json,
	std::collections::HashMap,
	tokio::sync::broadcast,
	tracing::{error, info},
	uuid::Uuid,
};
/*
“100% 可靠” 是理论理想，而 “99.9999% + 人工兜底” 才是生产实践。
系统设计的目标不是“永不犯错”，而是“错误可发现、可追溯、可修复”。
做“生产级”系统，而不是“论文级”系统
就用wal(启动时不重复)+运行消费中去下重(防止重复消费 helper.rs中SlidingWindowDedup结构体)来保障不重复消息
每一次撮合处理就fs.sync_all确保持久化 可以用最好的磁盘来快点 比如6us级别的ssd 再配合多机器撮合 能够承担起整个系统百万tps的消息处理了
但是wal在我们这里没啥用 因为撮合处理之后是内存发送到各个task的channel中去 然后外部去消费
wal能一定程度上保障了撮合的处理(实际还是有可能会因为你是不同task发送到一个专门用于写wal文件的task的channel 这个task还是有可能没被写入)
下游能不能消费取决于mq的可靠性
增加了复杂性 这是个复杂问题 我们这里就是如果出现了那么就把pending消息手动处理确认 正常都是优雅关机ackdel 除非遇到机器宕机或者程序panic 程序panic我们要百分百避免
*/

//https://docs.rs/redis/0.32.7/redis/trait.FromRedisValue.html#tymethod.from_redis_value
//redis的返回值redis::value可以转化为其他rust类型
/// NOTE: 启动的时候不删除stream中消息 因为删除消息一样的要走后续processor订单拒绝 所以统一走正常流程消费
pub async fn start_order_input_consumer_group(consumer_count: usize, batch_size: usize) -> anyhow::Result<()> {
	let group_name = crate::consts::ORDER_INPUT_CONSUMER_GROUP;
	let consumer_name_prefix = crate::consts::ORDER_INPUT_CONSUMER_NAME_PREFIX;
	// 确保消费者组存在
	// 初始化消费者组（如果不存在）
	let mut conn = common::redis_pool::get_engine_input_mq_connection().await?;
	// Redis XGROUP CREATE returns an error if the group already exists, which we can ignore
	let res: redis::RedisResult<bool> = conn.xgroup_create_mkstream(ORDER_INPUT_STREAM, group_name, "0").await;
	if let Err(e) = res {
		error!("Failed to create order input consumer group: {}", e);
		// 忽略错误，因为组可能已经存在
	}

	// 在启动消费者之前，先认领所有 pending 消息
	let pending_messages = claim_all_pending_messages(&mut conn, group_name).await?;
	info!("Claimed {} pending messages on startup", pending_messages.len());

	// 明确创建消费者 启动多个消费者任务
	let consumer_id = Uuid::new_v4().to_string();
	let mut started_consumers = 0;
	for i in 0..consumer_count {
		let consumer_id = format!("{}-{}-{}", consumer_name_prefix, consumer_id, i);

		let res: redis::RedisResult<bool> = conn.xgroup_createconsumer(ORDER_INPUT_STREAM, group_name, &consumer_id).await;
		if let Err(e) = res {
			error!("Failed to create order input consumer: {}", e);
			continue;
		}

		started_consumers += 1;

		let group_name = group_name.to_string();

		tokio::spawn(async move {
			order_input_consumer_task(group_name, consumer_id, batch_size).await;
		});
	}

	info!("Started {} order input consumers in group {}", started_consumers, group_name);
	Ok(())
}

/// 认领所有 pending 消息并存储到数组中
async fn claim_all_pending_messages<C: redis::AsyncCommands>(conn: &mut C, group_name: &str) -> anyhow::Result<Vec<(String, OrderInputMessage)>> {
	let mut pending_messages = Vec::new();

	// 使用 XPENDING 查看所有 pending 消息
	// XPENDING key group [start] [end] [count] [consumer]
	let pending_result: redis::RedisResult<redis::streams::StreamPendingReply> = conn.xpending_count(ORDER_INPUT_STREAM, group_name, "-", "+", 10000).await;

	match pending_result {
		Ok(redis::streams::StreamPendingReply::Data(data)) => {
			// 如果有 pending 消息，使用 XAUTOCLAIM 自动认领所有 pending 消息
			if data.count > 0 {
				// 使用 XAUTOCLAIM 自动认领所有 pending 消息
				let temp_consumer = format!("temp-claim-{}", Uuid::new_v4());
				let options = redis::streams::StreamAutoClaimOptions::default().count(data.count.min(10000)); // 最多认领 10000 条
				let autoclaim_result: redis::RedisResult<redis::streams::StreamAutoClaimReply> = conn
					.xautoclaim_options(
						ORDER_INPUT_STREAM,
						group_name,
						&temp_consumer,
						0u64,  // min-idle-time: 0 毫秒，立即认领
						"0-0", // start: 从最早开始
						options,
					)
					.await;

				match autoclaim_result {
					Ok(reply) => {
						let mut message_ids = Vec::new();

						// 解析认领的消息
						for stream_id in reply.claimed {
							let msg_id = stream_id.id.clone();
							message_ids.push(msg_id.clone());

							// 查找消息内容
							if let Some(fields) = stream_id.map.get(ORDER_INPUT_MSG_KEY) {
								match redis::from_redis_value::<String>(fields) {
									Ok(value) => {
										match serde_json::from_str::<OrderInputMessage>(&value) {
											Ok(msg) => {
												pending_messages.push((msg_id, msg));
											}
											Err(e) => {
												error!("Failed to deserialize pending order message: {}", e);
											}
										}
									}
									Err(e) => {
										error!("Failed to convert redis value to string for pending message: {}", e);
									}
								}
							}
						}

						// 使用 XACKDEL 确认并删除所有认领的消息，保持 pending 队列干净
						if !message_ids.is_empty() {
							let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
							// XACKDEL key group DELREF IDS numids id [id ...]
							// 使用 DELREF 选项删除所有引用，保持队列干净
							let result: redis::RedisResult<Vec<redis::streams::XAckDelStatusCode>> =
								conn.xack_del(ORDER_INPUT_STREAM, group_name, &ids, redis::streams::StreamDeletionPolicy::DelRef).await;
							match result {
								Ok(results) => {
									// results 是一个数组，每个 ID 对应一个状态码
									let deleted_count = results.iter().filter(|r| matches!(r, redis::streams::XAckDelStatusCode::AcknowledgedAndDeleted)).count();
									info!("XACKDEL {} claimed pending messages ({} deleted)", message_ids.len(), deleted_count);
								}
								Err(e) => {
									error!("Failed to XACKDEL claimed pending messages: {}", e);
								}
							}
						}
					}
					Err(e) => {
						error!("Failed to autoclaim pending messages: {}", e);
					}
				}
			}
		}
		Ok(redis::streams::StreamPendingReply::Empty) => {
			// 没有 pending 消息
		}
		Err(e) => {
			// XPENDING 可能返回错误（例如组不存在），忽略
			info!("XPENDING returned error (may be expected): {}", e);
		}
	}

	Ok(pending_messages)
}

/// 订单输入流的单个消费者任务
async fn order_input_consumer_task(group_name: String, consumer_id: String, batch_size: usize) {
	let mut shutdown_receiver = crate::engine::get_shutdown_receiver();

	// 在 loop 外部获取连接
	let mut conn = match common::redis_pool::get_engine_input_mq_connection().await {
		Ok(conn) => conn,
		Err(e) => {
			error!("Failed to get redis connection for consumer {}: {}", consumer_id, e);
			return;
		}
	};

	loop {
		tokio::select! {
			// 没有 future 交叉同时 await，所以借用是安全的
			result = read_order_messages(&mut conn, &group_name, &consumer_id, batch_size) => {
				match result {
					Ok(_) => {
						// 成功处理一批消息，继续下一批
					}
					Err(e) => {
						error!("Error reading messages for consumer {}: {}, reconnecting...", consumer_id, e);
						// 只有可能 redis 命令连接可能失败，重新获取连接
						match common::redis_pool::get_engine_input_mq_connection().await {
							Ok(new_conn) => {
								conn = new_conn;
								info!("Redis connection reestablished for consumer {}", consumer_id);
							}
							Err(e) => {
								error!("Failed to reconnect for consumer {}: {}", consumer_id, e);
								// 等待一段时间后重试
								tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
							}
						}
					}
				}
			}
			_ = shutdown_receiver.recv() => {
				info!("Order input consumer {} received shutdown signal", consumer_id);
				break;
			}
		}
	}
}

/// 读取并处理订单输入消息
async fn read_order_messages<C: redis::AsyncCommands>(conn: &mut C, group_name: &str, consumer_id: &str, batch_size: usize) -> anyhow::Result<()> {
	// 使用 XREADGROUP 读取消息
	let options = redis::streams::StreamReadOptions::default().group(group_name, consumer_id).count(batch_size).block(0); // 一直阻塞直到收到消息
	let keys = vec![ORDER_INPUT_STREAM];
	let ids = vec![">"];
	let result: redis::RedisResult<Option<redis::streams::StreamReadReply>> = conn.xread_options(&keys, &ids, &options).await;

	match result {
		Ok(Some(reply)) => {
			let mut message_ids = Vec::new();

			// 解析消息
			for stream_key in reply.keys {
				if stream_key.key == ORDER_INPUT_STREAM {
					for stream_id in stream_key.ids {
						let msg_id = stream_id.id.clone();
						message_ids.push(msg_id.clone());

						// 查找消息内容
						if let Some(fields) = stream_id.map.get(ORDER_INPUT_MSG_KEY) {
							match redis::from_redis_value::<String>(fields) {
								Ok(value) => {
									match serde_json::from_str::<OrderInputMessage>(&value) {
										Ok(msg) => {
											info!("Consumer {} received order message: {:?}", consumer_id, msg);
											process_order_input_message(msg).await;
										}
										Err(e) => {
											error!("Failed to deserialize order message: {}", e);
										}
									}
								}
								Err(e) => {
									error!("Failed to convert redis value to string: {}", e);
								}
							}
						}
					}
				}
			}

			// 使用 XACKDEL 确认并删除消息
			if !message_ids.is_empty() {
				let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
				// XACKDEL key group DELREF IDS numids id [id ...]
				// 使用 DELREF 选项删除所有引用，保持队列干净
				let result: redis::RedisResult<Vec<redis::streams::XAckDelStatusCode>> = conn.xack_del(ORDER_INPUT_STREAM, group_name, &ids, redis::streams::StreamDeletionPolicy::DelRef).await;
				if let Err(e) = result {
					error!("Failed to XACKDEL order messages: {}", e);
				}
			}
		}
		Ok(None) => {
			// 没有消息，继续等待
		}
		Err(e) => {
			return Err(anyhow::anyhow!("Redis XREADGROUP error: {}", e));
		}
	}

	Ok(())
}

pub async fn process_order_input_message(msg: OrderInputMessage) {
	match msg {
		OrderInputMessage::SubmitOrder(submit_msg) => {
			if let Err(e) = submit_order(submit_msg.clone()).await {
				// 提交订单失败，发送拒绝消息
				let rejected = OrderRejected { order_id: submit_msg.order_id, symbol: submit_msg.symbol, user_id: submit_msg.user_id, privy_id: submit_msg.privy_id, reason: e.to_string() };
				if let Err(send_err) = publish_to_processor(ProcessorMessage::OrderRejected(rejected)) {
					error!("Failed to publish order rejected message: {}", send_err);
				}
			}
		}
		OrderInputMessage::CancelOrder(msg) => {
			// 取消订单失败不需要发送 Processor 消息（只有取消成功才发送）
			if let Err(e) = cancel_order(msg).await {
				error!("Failed to cancel order: {}", e);
			}
		}
	}
}

pub async fn submit_order(submit_order_message: SubmitOrderMessage) -> anyhow::Result<()> {
	let manager = get_manager().read().await;
	if manager.stop {
		return Err(anyhow::anyhow!("All events stopped"));
	}
	let event_managers = manager.event_managers.read().await;
	let event_manager = event_managers.get(&submit_order_message.symbol.event_id).ok_or(anyhow::anyhow!("Event not found"))?;
	if let Some(end_timestamp) = event_manager.end_timestamp
		&& end_timestamp.timestamp_millis() < Utc::now().timestamp_millis()
	{
		return Err(anyhow::anyhow!("Event {} expired", submit_order_message.symbol.event_id));
	}
	// 使用event_id和market_id的组合来查找order_sender（不区分token_0/token_1）
	let market_key = format!("{}{}{}", submit_order_message.symbol.event_id, common::consts::SYMBOL_SEPARATOR, submit_order_message.symbol.market_id);
	let order_sender = event_manager.order_senders.get(&market_key).ok_or(anyhow::anyhow!("Order sender not found for market: {}", market_key))?;

	// 直接发送 SubmitOrderMessage，不再构造 Order（Order 只在 submit_order 中构造限价单时使用）
	if (order_sender.send(OrderBookControl::SubmitOrder(submit_order_message)).await).is_err() {
		//error!("System busy: MatchEngine channel full for market {}, unable to submit order", market_key);
		return Err(anyhow::anyhow!("System busy: MatchEngine for market {} is at full capacity", market_key));
	}
	Ok(())
}

pub async fn cancel_order(cancel_order_message: CancelOrderMessage) -> anyhow::Result<()> {
	let manager = get_manager().read().await;
	if manager.stop {
		return Err(anyhow::anyhow!("All events stopped"));
	}
	let event_managers = manager.event_managers.read().await;
	let event_manager = event_managers.get(&cancel_order_message.symbol.event_id).ok_or(anyhow::anyhow!("Event not found"))?;
	if let Some(end_timestamp) = event_manager.end_timestamp
		&& end_timestamp.timestamp_millis() < Utc::now().timestamp_millis()
	{
		return Err(anyhow::anyhow!("Event expired {}", cancel_order_message.symbol.event_id));
	}
	// 使用event_id和market_id的组合来查找order_sender（不区分token_0/token_1）
	let market_key = format!("{}{}{}", cancel_order_message.symbol.event_id, common::consts::SYMBOL_SEPARATOR, cancel_order_message.symbol.market_id);
	let order_sender = event_manager.order_senders.get(&market_key).ok_or(anyhow::anyhow!("Order sender not found for market: {}", market_key))?;
	if (order_sender.send(OrderBookControl::CancelOrder(cancel_order_message.order_id)).await).is_err() {
		//error!("System busy: MatchEngine channel full for market {}, unable to cancel order", market_key);
		return Err(anyhow::anyhow!("System busy: MatchEngine for market {} is at full capacity", market_key));
	}
	Ok(())
}

/// NOTE: 删不删除消息重新来都没影响
pub fn start_event_input_consumer(batch_size: usize) -> anyhow::Result<()> {
	tokio::spawn(async move {
		event_input_consumer_task(batch_size).await;
	});

	info!("Started event input consumer");
	Ok(())
}

/// 市场输入流的消费者任务
async fn event_input_consumer_task(batch_size: usize) {
	let mut shutdown_receiver = crate::engine::get_shutdown_receiver();

	// 在 loop 外部获取连接
	let mut conn = match common::redis_pool::get_engine_input_mq_connection().await {
		Ok(conn) => conn,
		Err(e) => {
			error!("Failed to get redis connection for event input consumer: {}", e);
			return;
		}
	};

	//撮合单副本 从0开始 需要删除
	let mut last_id = "0".to_string(); // 从最早的消息开始读取

	loop {
		tokio::select! {
			// 没有 future 交叉同时 await，所以借用是安全的
			result = read_event_messages(&mut conn, &mut last_id, batch_size) => {
				match result {
					Ok(_) => {
						// 成功处理一批消息，继续下一批
					}
					Err(e) => {
						error!("Error reading messages: {}, reconnecting...", e);
						// 只有可能 redis 命令连接可能失败，重新获取连接
						match common::redis_pool::get_engine_input_mq_connection().await {
							Ok(new_conn) => {
								conn = new_conn;
								info!("Redis connection reestablished for event input consumer");
							}
							Err(e) => {
								error!("Failed to reconnect: {}", e);
								// 等待一段时间后重试
								tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
							}
						}
					}
				}
			}
			_ = shutdown_receiver.recv() => {
				info!("Event input consumer received shutdown signal");
				break;
			}
		}
	}
}

/// 读取并处理市场输入消息
async fn read_event_messages<C: redis::AsyncCommands>(conn: &mut C, last_id: &mut String, batch_size: usize) -> anyhow::Result<()> {
	// 使用 XREAD 读取消息，使用上次记录的最新消息ID，避免漏消息
	let options = redis::streams::StreamReadOptions::default().count(batch_size).block(0); // 一直阻塞直到收到消息
	let keys = vec![EVENT_INPUT_STREAM];
	let ids = vec![last_id.as_str()];
	let result: redis::RedisResult<Option<redis::streams::StreamReadReply>> = conn.xread_options(&keys, &ids, &options).await;

	match result {
		Ok(Some(reply)) => {
			let mut message_ids = Vec::new();

			// 解析消息
			for stream_key in reply.keys {
				if stream_key.key == EVENT_INPUT_STREAM {
					for stream_id in stream_key.ids {
						let msg_id = stream_id.id.clone();
						message_ids.push(msg_id.clone());

						// 查找消息内容
						if let Some(fields) = stream_id.map.get(EVENT_INPUT_MSG_KEY) {
							match redis::from_redis_value::<String>(fields) {
								Ok(value) => {
									match serde_json::from_str::<EventInputMessage>(&value) {
										Ok(msg) => {
											process_event_input_message(msg).await;
										}
										Err(e) => error!("Failed to deserialize event message: {}", e),
									}
								}
								Err(e) => error!("Failed to convert redis value to string: {}", e),
							}
						}
					}
				}
			}

			// 使用 XDEL 删除消息
			if !message_ids.is_empty() {
				// 更新最新的消息ID为最后一个消息ID，下次读取时从这个ID之后开始
				if let Some(last_msg_id) = message_ids.last() {
					*last_id = last_msg_id.clone();
				}

				let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
				let _: redis::RedisResult<usize> = conn.xdel(EVENT_INPUT_STREAM, &ids).await;
			}
		}
		Ok(None) => {
			// 没有消息，继续等待
		}
		Err(e) => {
			return Err(anyhow::anyhow!("Redis XREAD error: {}", e));
		}
	}

	Ok(())
}

pub async fn process_event_input_message(msg: EventInputMessage) {
	match msg {
		EventInputMessage::AddOneEvent(msg) => {
			let msg_id = msg.event_id;
			if let Err(e) = add_event(msg).await {
				error!("Failed to add event: {}, error: {}", msg_id, e);
			}
		}
		EventInputMessage::RemoveOneEvent(msg) => {
			let msg_id = msg.event_id;
			if let Err(e) = remove_event(msg_id).await {
				error!("Failed to remove event: {}, error: {}", msg_id, e);
			}
		}
		EventInputMessage::AddOneMarket(msg) => {
			if let Err(e) = add_one_market(msg).await {
				error!("Failed to add market: error: {}", e);
			}
		}
		EventInputMessage::RemoveOneMarket(msg) => {
			if let Err(e) = remove_one_market(msg).await {
				error!("Failed to remove market: error: {}", e);
			}
		}
		EventInputMessage::StopAllEvents(msg) => stop_all_events(msg.stop).await,
		EventInputMessage::ResumeAllEvents(msg) => resume_all_events(msg.resume).await,
	}
}

pub async fn add_event(msg: EngineMQEventCreate) -> anyhow::Result<()> {
	info!("TEST_EVENT: Match engine received EventCreate from event_input_stream, event_id: {}", msg.event_id);

	// 检查市场是否已存在
	{
		let manager = get_manager().read().await;
		let event_managers = manager.event_managers.read().await;
		if event_managers.contains_key(&msg.event_id) {
			info!("Event {} already exists, skipping", msg.event_id);
			return Ok(());
		}
	}

	let config = get_config();
	let max_order_count = config.engine.engine_max_order_count;

	// 创建退出信号的broadcast channel
	let (exit_sender, _) = broadcast::channel(1);

	let mut order_senders = HashMap::new();
	let mut market_exit_signals = HashMap::new();

	// 为每个market创建一个MatchEngine（处理token_0和token_1两个结果）
	for (market_id_str, market) in &msg.markets {
		// 解析 market_id 从 String 到 i16
		let market_id: i16 = market_id_str.parse().map_err(|e| anyhow::anyhow!("Invalid market_id '{}': {}", market_id_str, e))?;

		// 为每个market创建独立的退出信号（oneshot）
		let (market_exit_sender, market_exit_receiver) = tokio::sync::oneshot::channel();
		let global_exit_receiver = exit_sender.subscribe();

		// 从market中获取token_ids
		let token_ids = if market.token_ids.len() >= 2 {
			(market.token_ids[0].clone(), market.token_ids[1].clone())
		} else {
			return Err(anyhow::anyhow!("Option {} must have at least 2 token_ids", market_id));
		};

		// 创建MatchEngine（使用global_exit_receiver）
		// 创建MatchEngine（使用global_exit_receiver和market_exit_receiver）
		let (mut engine, order_sender) = MatchEngine::new(msg.event_id, market_id, token_ids, max_order_count, global_exit_receiver, market_exit_receiver);

		// 启动MatchEngine的run任务
		tokio::spawn(async move {
			engine.run().await;
		});

		// 使用event_id和market_id的组合作为key
		let market_key = format!("{}{}{}", msg.event_id, common::consts::SYMBOL_SEPARATOR, market_id);
		order_senders.insert(market_key.clone(), order_sender);
		market_exit_signals.insert(market_key, market_exit_sender);
	}

	// 创建EventManager
	let event_manager = EventManager { event_id: msg.event_id, order_senders, exit_signal: exit_sender, market_exit_signals, end_timestamp: msg.end_date };

	// 将EventManager存储到全局Manager中

	let manager = get_manager().write().await;
	let mut event_managers = manager.event_managers.write().await;
	event_managers.insert(msg.event_id, event_manager);

	// 通知 store 添加市场信息和选项
	if let Err(e) = publish_order_change_to_store(OrderChangeEvent::EventAdded(msg.clone())) {
		error!("Failed to publish event added event to store: {}", e);
	}

	info!("Event {} added with {} markets", msg.event_id, msg.markets.len());
	info!("TEST_EVENT: Match engine successfully initialized event and created orderbooks, event_id: {}, markets_count: {}", msg.event_id, msg.markets.len());
	Ok(())
}

pub async fn remove_event(event_id: i64) -> anyhow::Result<()> {
	info!("TEST_EVENT: Match engine received EventClose from event_input_stream, event_id: {}", event_id);

	let manager = get_manager().write().await;
	let mut event_managers = manager.event_managers.write().await;

	// 获取EventManager并发送退出信号
	match event_managers.remove(&event_id) {
		Some(event_manager) => {
			// 发送退出信号给所有MatchEngine
			let _ = event_manager.exit_signal.send(());
			info!("Sent exit signal to all MatchEngines for event {}", event_id);

			// 通知 store 移除市场并清理订单
			if let Err(e) = publish_order_change_to_store(OrderChangeEvent::EventRemoved(event_id)) {
				error!("Failed to publish event removed event to store: {}", e);
			}
		}
		None => {
			return Err(anyhow::anyhow!("Event {} not found", event_id));
		}
	}

	info!("Event {} removed", event_id);
	Ok(())
}

pub async fn stop_all_events(stop: bool) {
	if !stop {
		return;
	}
	let mut manager = get_manager().write().await;
	manager.stop = true;
	info!("All events stopped");
}

pub async fn resume_all_events(resume: bool) {
	if !resume {
		return;
	}
	let mut manager = get_manager().write().await;
	manager.stop = false;
	info!("All events resumed");
}

/// 在已有Event下动态添加单个Market
pub async fn add_one_market(msg: common::engine_types::AddOneMarketMessage) -> anyhow::Result<()> {
	info!("Match engine adding market: event_id={}, market_id={}", msg.event_id, msg.market.market_id);

	let manager = get_manager().read().await;
	let mut event_managers = manager.event_managers.write().await;

	// 获取EventManager
	let event_manager = event_managers.get_mut(&msg.event_id).ok_or(anyhow::anyhow!("Event {} not found", msg.event_id))?;

	// 检查market是否已存在
	let market_key = format!("{}{}{}", msg.event_id, common::consts::SYMBOL_SEPARATOR, msg.market.market_id);
	if event_manager.order_senders.contains_key(&market_key) {
		info!("Market {} already exists in event {}, skipping", msg.market.market_id, msg.event_id);
		return Ok(());
	}

	let config = crate::config::get_config();
	let max_order_count = config.engine.engine_max_order_count;

	// 为该market创建独立的退出信号（oneshot）
	let (market_exit_sender, market_exit_receiver) = tokio::sync::oneshot::channel();
	let global_exit_receiver = event_manager.exit_signal.subscribe();

	// 从market中获取token_ids
	let token_ids = if msg.market.token_ids.len() >= 2 {
		(msg.market.token_ids[0].clone(), msg.market.token_ids[1].clone())
	} else {
		return Err(anyhow::anyhow!("Market {} must have at least 2 token_ids", msg.market.market_id));
	};

	// 创建MatchEngine
	// 创建MatchEngine（使用global_exit_receiver和market_exit_receiver）
	let (mut engine, order_sender) = crate::engine::MatchEngine::new(msg.event_id, msg.market.market_id, token_ids, max_order_count, global_exit_receiver, market_exit_receiver);

	// 启动MatchEngine的run任务
	tokio::spawn(async move {
		engine.run().await;
	});

	// 添加到order_senders和market_exit_signals
	event_manager.order_senders.insert(market_key.clone(), order_sender);
	event_manager.market_exit_signals.insert(market_key, market_exit_sender);

	// 通知 store 添加市场
	if let Err(e) = crate::output::publish_order_change_to_store(common::store_types::OrderChangeEvent::MarketAdded { event_id: msg.event_id, market: msg.market.clone() }) {
		error!("Failed to publish market added event to store: {}", e);
	}

	info!("Market {} added to event {}", msg.market.market_id, msg.event_id);
	Ok(())
}

/// 移除Event下的单个Market（关闭并取消所有订单）
pub async fn remove_one_market(msg: common::engine_types::RemoveOneMarketMessage) -> anyhow::Result<()> {
	info!("Match engine removing market: event_id={}, market_id={}", msg.event_id, msg.market_id);

	let manager = get_manager().read().await;
	let mut event_managers = manager.event_managers.write().await;

	// 获取EventManager
	let event_manager = event_managers.get_mut(&msg.event_id).ok_or(anyhow::anyhow!("Event {} not found", msg.event_id))?;

	// 移除market的order_sender（这会导致该market的MatchEngine无法接收新订单）
	let market_key = format!("{}{}{}", msg.event_id, common::consts::SYMBOL_SEPARATOR, msg.market_id);
	if event_manager.order_senders.remove(&market_key).is_none() {
		return Err(anyhow::anyhow!("Market {} not found in event {}", msg.market_id, msg.event_id));
	}

	// 发送market级别的退出信号给该market的MatchEngine task
	if let Some(market_exit_sender) = event_manager.market_exit_signals.remove(&market_key) {
		let _ = market_exit_sender.send(());
		info!("Sent exit signal to market {} in event {}", msg.market_id, msg.event_id);
	}

	// join_set的监控任务会自动检测到task完成

	// 通知 store 移除市场
	if let Err(e) = crate::output::publish_order_change_to_store(common::store_types::OrderChangeEvent::MarketRemoved { event_id: msg.event_id, market_id: msg.market_id }) {
		error!("Failed to publish market removed event to store: {}", e);
	}

	info!("Market {} removed from event {} (order_sender removed, exit signal sent)", msg.market_id, msg.event_id);
	Ok(())
}

pub async fn start_input_consumer() -> anyhow::Result<()> {
	let config = crate::config::get_config();
	start_order_input_consumer_group(config.engine_input_mq.order_input_consumer_count, config.engine_input_mq.order_input_batch_size).await?;
	start_event_input_consumer(config.engine_input_mq.event_input_batch_size)?;
	Ok(())
}
