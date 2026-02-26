use {
	crate::cache,
	common::{
		consts::{API_MQ_MSG_KEY, API_MQ_STREAM},
		event_types::ApiEventMqMessage,
		redis_pool,
	},
	redis::from_redis_value,
	tokio::sync::{OnceCell, broadcast},
	tracing::{error, info},
};

static SHUTDOWN: OnceCell<broadcast::Sender<()>> = OnceCell::const_new();

/// 初始化 shutdown sender
pub fn init_shutdown() {
	let (tx, _) = broadcast::channel(1);
	let _ = SHUTDOWN.set(tx);
}

/// 获取 shutdown sender
fn get_shutdown() -> &'static broadcast::Sender<()> {
	SHUTDOWN.get().expect("SHUTDOWN not initialized")
}

/// 获取 shutdown receiver
pub fn get_shutdown_receiver() -> broadcast::Receiver<()> {
	get_shutdown().subscribe()
}

/// 发送 shutdown 信号
pub fn send_shutdown() {
	let _ = get_shutdown().send(());
}

/// 消费者任务
pub async fn consumer_task() {
	let mut shutdown_receiver = get_shutdown_receiver();

	// 在 loop 外部获取连接
	let mut conn = match redis_pool::get_common_mq_connection().await {
		Ok(conn) => conn,
		Err(e) => {
			error!("Failed to get redis connection: {}", e);
			return;
		}
	};

	//多副本 从最新开始读 不删除消息
	let mut last_id = "$".to_string();

	loop {
		tokio::select! {
			//没有future交叉同时await 所以借用是安全的
			result = read_messages(&mut conn, &mut last_id) => {
				match result {
					Ok(_) => {
						// 成功处理一批消息，继续下一批
					}
					Err(e) => {
						error!("Error reading messages: {}, reconnecting...", e);
						// 只有可能redis命令连接可能失败，重新获取连接
						// 不在不可控边界过度防御，而在可控范围内保持简单高效。
						match redis_pool::get_common_mq_connection().await {
							Ok(new_conn) => {
								conn = new_conn;
								info!("Redis connection reestablished");
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
				info!("Received shutdown signal, exiting consumer task");
				break;
			}
		}
	}
}

/// 读取并处理消息
async fn read_messages<C: redis::AsyncCommands>(conn: &mut C, last_id: &mut String) -> anyhow::Result<()> {
	// 使用 XREAD 读取消息，每次只读取一条
	let options = redis::streams::StreamReadOptions::default().count(1).block(5000); // 一直阻塞直到收到消息
	let keys = vec![API_MQ_STREAM];
	let ids = vec![last_id.as_str()];
	let result: redis::RedisResult<Option<redis::streams::StreamReadReply>> = conn.xread_options(&keys, &ids, &options).await;

	match result {
		Ok(Some(reply)) => {
			let mut message_ids = Vec::new();

			// 解析消息
			for stream_key in reply.keys {
				if stream_key.key == API_MQ_STREAM {
					for stream_id in stream_key.ids {
						let msg_id = stream_id.id.clone();
						message_ids.push(msg_id.clone());

						// 查找消息内容
						if let Some(fields) = stream_id.map.get(API_MQ_MSG_KEY) {
							match from_redis_value::<String>(fields) {
								Ok(value) => {
									if let Err(e) = process_message(value.as_str()).await {
										error!("Failed to handle message {}: {}", msg_id, e);
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

			// 使用 XDEL 删除消息
			if !message_ids.is_empty() {
				// 更新最新的消息ID
				if let Some(last_msg_id) = message_ids.last() {
					*last_id = last_msg_id.clone();
				}

				// 多副本
				// let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
				// let _: redis::RedisResult<usize> = conn.xdel(API_MQ_STREAM, &ids).await;
			}
		}
		Ok(None) => {
			// 没有消息，继续等待
		}
		Err(e) => {
			// 返回错误让外层重连（如 Redis 重启后 broken pipe）
			error!("Redis XREAD error: {}", e);
			return Err(e.into());
		}
	}

	Ok(())
}

/// 处理单条消息
async fn process_message(value: &str) -> anyhow::Result<()> {
	// 解析消息
	let message: ApiEventMqMessage = serde_json::from_str(value)?;

	// 处理不同类型的消息
	match message {
		ApiEventMqMessage::EventCreate(event) => {
			info!("Received event create: event_id={}", event.event_id);
			info!("TEST_EVENT: API service received EventCreate from api_mq_stream, event_id: {}, event_identifier: {}", event.event_id, event.event_identifier);
			cache::insert_event(*event).await;
		}
		ApiEventMqMessage::EventClose(close) => {
			info!("Received event close: event_id={}", close.event_id);
			info!("TEST_EVENT: API service received EventClose from api_mq_stream, event_id: {}", close.event_id);
			cache::remove_event(close.event_id).await;
		}
		ApiEventMqMessage::MarketAdd(market_add) => {
			info!("Received market add: event_id={}, market_id={}", market_add.event_id, market_add.market_id);
			cache::add_market_to_event(market_add.event_id, market_add.market_id, market_add.market).await;
		}
		ApiEventMqMessage::MarketClose(market_close) => {
			info!("Received market close: event_id={}, market_id={}", market_close.event_id, market_close.market_id);
			cache::remove_market_from_event(market_close.event_id, market_close.market_id).await;
		}
	}

	Ok(())
}
