use {
	crate::{
		consts::EVENT_RESOLUTION_CHECK_INTERVAL_SECS,
		db,
		types::{EventClose, EventCreate, MarketAdd, MarketClose},
	},
	chrono::Utc,
	common::{
		consts::{API_MQ_MSG_KEY, API_MQ_STREAM, EVENT_INPUT_MSG_KEY, EVENT_INPUT_STREAM, EVENT_MQ_MSG_KEY, EVENT_MQ_STREAM, ONCHAIN_EVENT_MSG_KEY, ONCHAIN_EVENT_STREAM},
		engine_types::EventInputMessage,
		event_types::{ApiEventMqMessage, ApiMQEventCreate, ApiMQEventMarket, EngineMQEventCreate, EngineMQEventMarket, MQEventClose, OnchainEventMessage, OnchainMQEventCreate, OnchainMQEventMarket},
		model::{EventMarket as EventMarketModel, Events as EventModel},
		redis_pool,
	},
	redis::{AsyncCommands, from_redis_value},
	serde::{Deserialize, Serialize},
	std::collections::HashMap,
	tokio::sync::{OnceCell, broadcast, mpsc},
	tracing::{error, info},
};

/// 类型别名：用于简化复杂类型定义
type EventQueryResult = (i64, bool, sqlx::types::Json<HashMap<String, EventMarketModel>>);

/// Event MQ 消息类型枚举（用于统一处理消息）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "types")]
enum EventMqMessage {
	#[serde(rename = "EventCreate")]
	EventCreate(EventCreate),
	#[serde(rename = "EventClose")]
	EventClose(EventClose),
	#[serde(rename = "MarketAdd")]
	MarketAdd(MarketAdd),
	#[serde(rename = "MarketClose")]
	MarketClose(MarketClose),
}

static SHUTDOWN: OnceCell<broadcast::Sender<()>> = OnceCell::const_new();

// 市场关闭检测任务的 channel
static RESOLUTION_CHECK_SENDER: OnceCell<mpsc::UnboundedSender<i64>> = OnceCell::const_new();

/// 初始化 shutdown sender 和 resolution check channel
pub fn init_shutdown() {
	let (tx, _) = broadcast::channel(1);
	let _ = SHUTDOWN.set(tx);

	// 初始化 resolution check channel 并启动任务
	let (sender, receiver) = mpsc::unbounded_channel();
	let _ = RESOLUTION_CHECK_SENDER.set(sender);
	tokio::spawn(resolution_check_task(receiver));
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

	//单副本 从0开始 需要删除
	let mut last_id = "0".to_string();

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
				info!("Consumer received shutdown signal");
				break;
			}
		}
	}
}

/// 读取并处理消息
async fn read_messages<C: redis::AsyncCommands>(conn: &mut C, last_id: &mut String) -> anyhow::Result<()> {
	// 使用 XREAD 读取消息，每次只读取一条
	let options = redis::streams::StreamReadOptions::default().count(1).block(0); // 一直阻塞直到收到消息
	let keys = vec![EVENT_MQ_STREAM];
	let ids = vec![last_id.as_str()];
	let result: redis::RedisResult<Option<redis::streams::StreamReadReply>> = conn.xread_options(&keys, &ids, &options).await;

	match result {
		Ok(Some(reply)) => {
			let mut message_ids = Vec::new();

			// 解析消息
			for stream_key in reply.keys {
				if stream_key.key == EVENT_MQ_STREAM {
					for stream_id in stream_key.ids {
						let msg_id = stream_id.id.clone();
						message_ids.push(msg_id.clone());

						// 查找消息内容
						if let Some(fields) = stream_id.map.get(EVENT_MQ_MSG_KEY) {
							match from_redis_value::<String>(fields) {
								Ok(value) => {
									if let Err(e) = handle_message(value.as_str()).await {
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

				let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
				let _: redis::RedisResult<usize> = conn.xdel(EVENT_MQ_STREAM, &ids).await;
			}
		}
		Ok(None) => {
			// 没有消息，继续等待
		}
		Err(e) => {
			// 打印错误，继续循环
			error!("Redis XREAD error: {}", e);
		}
	}

	Ok(())
}

/// 处理消息
async fn handle_message(value: &str) -> anyhow::Result<()> {
	let message: EventMqMessage = serde_json::from_str(value)?;

	// 完整克隆消息并打印日志
	let message_clone = message.clone();
	info!("Received message: {:?}", message_clone);

	match message {
		EventMqMessage::EventCreate(event_create) => {
			handle_event_create(event_create).await?;
		}
		EventMqMessage::EventClose(event_close) => {
			handle_event_close(event_close).await?;
		}
		EventMqMessage::MarketAdd(market_add) => {
			handle_market_add(market_add).await?;
		}
		EventMqMessage::MarketClose(market_close) => {
			handle_market_close(market_close).await?;
		}
	}

	Ok(())
}

/// 发送消息到 Redis Stream (使用 common_mq 连接, DB 3)
async fn publish_to_stream(stream: &str, msg_key: &str, message: &impl serde::Serialize) -> anyhow::Result<()> {
	let mut conn = redis_pool::get_common_mq_connection().await?;
	let message_json = serde_json::to_string(message)?;
	info!("TEST_EVENT: Publishing to stream '{}', JSON: {}", stream, message_json);

	let items: Vec<(&str, &str)> = vec![(msg_key, &message_json)];

	// 根据 stream 类型决定是否使用 MAXLEN
	match stream {
		common::consts::API_MQ_STREAM => {
			let _: Option<String> = conn.xadd_maxlen(stream, redis::streams::StreamMaxlen::Approx(common::consts::API_MQ_STREAM_MAXLEN), "*", &items).await?;
		}
		common::consts::ONCHAIN_EVENT_STREAM => {
			let _: Option<String> = conn.xadd_maxlen(stream, redis::streams::StreamMaxlen::Approx(common::consts::ONCHAIN_EVENT_STREAM_MAXLEN), "*", &items).await?;
		}
		_ => {
			let _: Option<String> = conn.xadd(stream, "*", &items).await?;
		}
	}

	Ok(())
}

/// 发送消息到 match_engine 的 Redis Stream (使用 engine_input_mq 连接, DB 0)
async fn publish_to_engine_stream(stream: &str, msg_key: &str, message: &impl serde::Serialize) -> anyhow::Result<()> {
	let mut conn = redis_pool::get_engine_input_mq_connection().await?;
	let message_json = serde_json::to_string(message)?;
	info!("TEST_EVENT: Publishing to engine stream '{}', JSON: {}", stream, message_json);

	let items: Vec<(&str, &str)> = vec![(msg_key, &message_json)];
	let _: Option<String> = conn.xadd(stream, "*", &items).await?;

	Ok(())
}

/// 处理 EventCreate 消息
async fn handle_event_create(event_create: EventCreate) -> anyhow::Result<()> {
	info!("TEST_EVENT: Event service received EventCreate message, event_identifier: {}", event_create.event_identifier);

	// 1. 验证 topic 是否存在且为 active
	if !db::check_topic_exists(&event_create.topic).await? {
		error!("EventCreate: topic '{}' does not exist or is not active for event_identifier: {}", event_create.topic, event_create.event_identifier);
		return Err(anyhow::anyhow!("Topic '{}' does not exist or is not active", event_create.topic));
	}

	// 2. 验证 markets 不为空
	if event_create.markets.is_empty() {
		error!("EventCreate: markets is empty for event_identifier: {}", event_create.event_identifier);
		return Err(anyhow::anyhow!("Markets is empty"));
	}

	// 2. 验证并构造 EventMarket
	let mut db_markets: HashMap<String, EventMarketModel> = HashMap::new();
	let mut api_markets: HashMap<String, ApiMQEventMarket> = HashMap::new();
	let mut engine_markets: HashMap<String, EngineMQEventMarket> = HashMap::new();
	let mut onchain_markets: HashMap<String, OnchainMQEventMarket> = HashMap::new();

	for (idx, market) in event_create.markets.iter().enumerate() {
		let market_id = (idx + 1) as i16;
		let market_id_str = market_id.to_string();

		// 验证 market
		if let Err(e) = market.validate() {
			error!("EventCreate: market {} validation failed: {}, event_identifier: {}", market.market_identifier, e, event_create.event_identifier);
			return Err(anyhow::anyhow!("Market validation failed: {}", e));
		}

		// 排序 outcomes 和 token_ids (Yes 在前，No 在后，否则字典序)
		let (outcomes, token_ids) = sort_outcomes_and_token_ids(&market.outcome_names, &market.token_ids);

		// 构造 outcome_info: token_id -> outcome_name 的映射
		let mut outcome_info = HashMap::new();
		for (token_id, outcome_name) in token_ids.iter().zip(outcomes.iter()) {
			outcome_info.insert(token_id.clone(), outcome_name.clone());
		}

		// 构造 EventMarketModel
		let db_market = EventMarketModel {
			parent_collection_id: market.parent_collection_id.clone(),
			condition_id: market.condition_id.clone(),
			id: market_id,
			market_identifier: market.market_identifier.clone(),
			question: market.question.clone(),
			slug: market.slug.clone(),
			title: market.title.clone(),
			image: market.image.clone().unwrap_or_default(),
			outcomes: outcomes.clone(),
			token_ids: token_ids.clone(),
			win_outcome_name: String::new(),     // 初始为空
			win_outcome_token_id: String::new(), // 初始为空
			closed: false,                       // 初始未关闭
			closed_at: None,                     // 初始为空
			volume: rust_decimal::Decimal::ZERO, // 初始交易量为0
		};

		// 构造 ApiMQEventMarket
		let api_market = ApiMQEventMarket {
			parent_collection_id: market.parent_collection_id.clone(),
			market_id,
			condition_id: market.condition_id.clone(),
			market_identifier: market.market_identifier.clone(),
			question: market.question.clone(),
			slug: market.slug.clone(),
			title: market.title.clone(),
			image: market.image.clone().unwrap_or_default(),
			outcome_info,
			outcome_names: outcomes.clone(),
			outcome_token_ids: token_ids.clone(),
		};

		// 构造 EngineMQEventMarket
		let engine_market = EngineMQEventMarket { market_id, outcomes: outcomes.clone(), token_ids: token_ids.clone() };

		// 构造 OnchainMQEventMarket
		let onchain_market = OnchainMQEventMarket { market_id, condition_id: market.condition_id.clone(), token_ids: token_ids.clone(), outcomes: outcomes.clone() };

		db_markets.insert(market_id_str.clone(), db_market);
		api_markets.insert(market_id_str.clone(), api_market);
		engine_markets.insert(market_id_str.clone(), engine_market);
		onchain_markets.insert(market_id_str, onchain_market);
	}

	// 3. 构造 Event 并写入数据库
	let now = Utc::now();
	let current_timestamp = now.timestamp();

	let end_date = event_create
		.end_date
		.map(|ts| {
			// 检查时间戳是否大于当前时间戳
			if ts <= current_timestamp {
				error!("EventCreate: end_date timestamp {} is not in the future (current: {}) for event_identifier: {}", ts, current_timestamp, event_create.event_identifier);
				return Err(anyhow::anyhow!("end_date timestamp must be in the future"));
			}

			// 转换时间戳
			match chrono::DateTime::from_timestamp(ts, 0) {
				Some(dt) => Ok(dt),
				None => {
					error!("EventCreate: Invalid end_date timestamp {} for event_identifier: {}", ts, event_create.event_identifier);
					Err(anyhow::anyhow!("Invalid end_date timestamp: {}", ts))
				}
			}
		})
		.transpose()?;
	let event = EventModel {
		id: 0, // 数据库会自动生成
		event_identifier: event_create.event_identifier.clone(),
		slug: event_create.slug.clone(),
		title: event_create.title.clone(),
		description: event_create.description.clone(),
		image: event_create.image.clone(),
		end_date,
		closed: false,
		closed_at: None, // 初始值，实际会在 close 时更新
		resolved: false,
		resolved_at: None, // 初始值，实际会在 resolved 时更新
		topic: event_create.topic.clone(),
		volume: rust_decimal::Decimal::ZERO,
		markets: sqlx::types::Json(db_markets),
		recommended: false,
		created_at: now,
	};

	let event_id = db::insert_event(&event).await.map_err(|e| {
		error!("EventCreate: Failed to insert event to database: {}, event_identifier: {}", e, event_create.event_identifier);
		e
	})?;
	info!("Event created: id={}, identifier={}", event_id, event_create.event_identifier);

	// 4. 发送到 stream
	// 发送 ApiEventMqMessage::EventCreate 到 api stream
	let api_create = ApiMQEventCreate {
		event_id,
		event_identifier: event_create.event_identifier.clone(),
		slug: event_create.slug.clone(),
		title: event_create.title.clone(),
		description: event_create.description.clone(),
		image: event_create.image.clone(),
		end_date,
		topic: event_create.topic.clone(),
		markets: api_markets,
		created_at: now,
	};
	let api_msg = ApiEventMqMessage::EventCreate(Box::new(api_create));
	publish_to_stream(API_MQ_STREAM, API_MQ_MSG_KEY, &api_msg).await.map_err(|e| {
		error!("EventCreate: Failed to publish to API stream: {}, event_id: {}, event_identifier: {}", e, event_id, event_create.event_identifier);
		e
	})?;

	// 发送 EventInputMessage::AddOneEvent 到 engine stream (DB 0)
	let engine_create = EngineMQEventCreate { event_id, markets: engine_markets, end_date };
	let engine_msg = EventInputMessage::AddOneEvent(engine_create);

	publish_to_engine_stream(EVENT_INPUT_STREAM, EVENT_INPUT_MSG_KEY, &engine_msg).await.map_err(|e| {
		error!("EventCreate: Failed to publish to ENGINE stream: {}, event_id: {}, event_identifier: {}", e, event_id, event_create.event_identifier);
		e
	})?;

	// 发送 OnchainEventMessage::Create 到 onchain_event_stream
	let onchain_create_msg = OnchainMQEventCreate { event_id, markets: onchain_markets };
	let onchain_msg = OnchainEventMessage::Create(onchain_create_msg);

	publish_to_stream(ONCHAIN_EVENT_STREAM, ONCHAIN_EVENT_MSG_KEY, &onchain_msg).await.map_err(|e| {
		error!("EventCreate: Failed to publish to ONCHAIN stream: {}, event_id: {}, event_identifier: {}", e, event_id, event_create.event_identifier);
		e
	})?;

	info!(
		"TEST_EVENT: Event service successfully pushed event to all streams (api_mq_stream, event_input_stream, onchain_event_stream), event_id: {}, event_identifier: {}",
		event_id, event_create.event_identifier
	);

	Ok(())
}

/// 构造 outcomes 和 token_ids 数组
/// 如果是 Yes/No，Yes 在前，No 在后；否则按 value 字典序排序
/// 排序 outcome_names 和 token_ids (保持对应关系)
/// Yes 在前，No 在后，否则按字典序
fn sort_outcomes_and_token_ids(outcome_names: &[String], token_ids: &[String]) -> (Vec<String>, Vec<String>) {
	// 创建索引配对
	let mut pairs: Vec<(usize, &String, &String)> = outcome_names.iter().zip(token_ids.iter()).enumerate().map(|(idx, (name, id))| (idx, name, id)).collect();

	// 检查是否是 Yes/No
	let is_yes_no = pairs.len() == 2 && pairs.iter().any(|(_, name, _)| name.eq_ignore_ascii_case("Yes")) && pairs.iter().any(|(_, name, _)| name.eq_ignore_ascii_case("No"));

	if is_yes_no {
		// Yes 在前，No 在后
		pairs.sort_by(|(_, name1, _), (_, name2, _)| {
			match (name1.eq_ignore_ascii_case("Yes"), name2.eq_ignore_ascii_case("Yes")) {
				(true, false) => std::cmp::Ordering::Less,
				(false, true) => std::cmp::Ordering::Greater,
				_ => std::cmp::Ordering::Equal,
			}
		});
	} else {
		// 按 outcome_name 字典序排序
		pairs.sort_by(|(_, name1, _), (_, name2, _)| name1.cmp(name2));
	}

	let sorted_outcomes: Vec<String> = pairs.iter().map(|(_, name, _)| (*name).clone()).collect();
	let sorted_token_ids: Vec<String> = pairs.iter().map(|(_, _, id)| (*id).clone()).collect();

	(sorted_outcomes, sorted_token_ids)
}

/// 处理 EventClose 消息
async fn handle_event_close(event_close: EventClose) -> anyhow::Result<()> {
	info!("TEST_EVENT: Event service received EventClose message, event_identifier: {}", event_close.event_identifier);

	// 1. 检查是否所有市场都已结束
	// 从数据库查询当前 event 的所有 markets
	let pool = db::get_db_pool()?;
	let markets_json: serde_json::Value = sqlx::query_scalar("SELECT markets FROM events WHERE event_identifier = $1").bind(&event_close.event_identifier).fetch_one(&pool).await.map_err(|e| {
		error!("EventClose: Failed to query event markets: {}, event_identifier: {}", e, event_close.event_identifier);
		e
	})?;

	let markets_map: std::collections::HashMap<String, common::model::EventMarket> = serde_json::from_value(markets_json).map_err(|e| {
		error!("EventClose: Failed to parse markets JSON: {}, event_identifier: {}", e, event_close.event_identifier);
		e
	})?;

	// 收集所有已经关闭的 market_identifier
	let mut closed_market_identifiers: std::collections::HashSet<String> = markets_map.values().filter(|m| m.closed).map(|m| m.market_identifier.clone()).collect();

	// 将请求中的 market_identifier 加入集合
	for result in &event_close.markets_result {
		closed_market_identifiers.insert(result.market_identifier.clone());
	}

	// 收集所有 market 的 market_identifier
	let all_market_identifiers: std::collections::HashSet<String> = markets_map.values().map(|m| m.market_identifier.clone()).collect();

	// 检查：已关闭的 + 请求中的 是否覆盖了所有 markets
	if closed_market_identifiers != all_market_identifiers {
		let missing: Vec<_> = all_market_identifiers.difference(&closed_market_identifiers).collect();
		error!(
			"EventClose: Not all markets are closed. Total markets: {}, closed+request: {}, missing: {:?}, event_identifier: {}",
			all_market_identifiers.len(),
			closed_market_identifiers.len(),
			missing,
			event_close.event_identifier
		);
		return Err(anyhow::anyhow!("Not all markets are closed"));
	}

	// 2. 更新数据库（同时更新 closed 和 markets)
	let markets_results: Vec<(String, String, String)> =
		event_close.markets_result.iter().map(|result| (result.market_identifier.clone(), result.win_outcome_token_id.clone(), result.win_outcome_name.clone())).collect();

	let event_id = db::update_event_closed(&event_close.event_identifier, &markets_results).await.map_err(|e| {
		error!("EventClose: Failed to update event closed: {}, event_identifier: {}", e, event_close.event_identifier);
		e
	})?;
	info!("Event closed: id={}, identifier={}", event_id, event_close.event_identifier);

	// 2. 发送到 stream
	let close_msg = MQEventClose { event_id };

	// 发送 EventInputMessage::RemoveOneEvent 到 engine stream (DB 0)
	let engine_close_msg = EventInputMessage::RemoveOneEvent(close_msg.clone());
	publish_to_engine_stream(EVENT_INPUT_STREAM, EVENT_INPUT_MSG_KEY, &engine_close_msg).await.map_err(|e| {
		error!("EventClose: Failed to publish to ENGINE stream: {}, event_id: {}, event_identifier: {}", e, event_id, event_close.event_identifier);
		e
	})?;

	// 发送 ApiEventMqMessage::EventClose 到 api stream
	let api_close_msg = ApiEventMqMessage::EventClose(close_msg.clone());
	publish_to_stream(API_MQ_STREAM, API_MQ_MSG_KEY, &api_close_msg).await.map_err(|e| {
		error!("EventClose: Failed to publish to API stream: {}, event_id: {}, event_identifier: {}", e, event_id, event_close.event_identifier);
		e
	})?;

	// 发送 OnchainEventMessage::Close 到 onchain_event_stream
	let onchain_msg = OnchainEventMessage::Close(close_msg.clone());
	publish_to_stream(ONCHAIN_EVENT_STREAM, ONCHAIN_EVENT_MSG_KEY, &onchain_msg).await.map_err(|e| {
		error!("EventClose: Failed to publish to ONCHAIN stream: {}, event_id: {}, event_identifier: {}", e, event_id, event_close.event_identifier);
		e
	})?;

	info!("TEST_EVENT: Event service successfully pushed EventClose to all streams (event_input_stream, api_mq_stream, onchain_event_stream), event_id: {}", event_id);

	// 3. 发送 event_id 到定期检测任务
	if let Some(sender) = RESOLUTION_CHECK_SENDER.get() {
		if let Err(e) = sender.send(event_id) {
			error!("Failed to send event_id to resolution check task: {}", e);
		}
	} else {
		error!("Resolution check sender not initialized");
	}

	Ok(())
}

/// 定期检测任务：接收 event_id，每5分钟检查订单状态
async fn resolution_check_task(mut receiver: mpsc::UnboundedReceiver<i64>) {
	use std::collections::HashSet;
	let mut pending_events: HashSet<i64> = HashSet::new();

	// 启动时先从数据库加载所有 closed 但未 resolved 的市场
	match db::get_pending_resolved_events().await {
		Ok(event_ids) => {
			for event_id in event_ids {
				pending_events.insert(event_id);
			}
			info!("Loaded {} pending events from database", pending_events.len());
		}
		Err(e) => {
			error!("Failed to load pending events from database: {}", e);
		}
	}

	let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(EVENT_RESOLUTION_CHECK_INTERVAL_SECS)); // 5分钟
	interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
	interval.tick().await; // 第一次立即执行

	loop {
		tokio::select! {
			// 接收新的 event_id
			event_id = receiver.recv() => {
				match event_id {
					Some(id) => {
						info!("Received event close notification: event_id={}", id);
						pending_events.insert(id);
					}
					None => {
						info!("Resolution check receiver closed");
						break;
					}
				}
			}
			// 每5分钟检查一次
			_ = interval.tick() => {
				if pending_events.is_empty() {
					continue;
				}

				let events_to_check: Vec<i64> = pending_events.iter().copied().collect();
				for event_id in events_to_check {
					match db::check_event_orders_resolved(event_id).await {
						Ok(true) => {
							// 所有订单都已处理完，更新为 resolved
							if let Err(e) = db::update_event_resolved(event_id).await {
								error!("Failed to update event resolved: event_id={}, error={}", event_id, e);
							} else {
								info!("Event resolved: event_id={}", event_id);
								pending_events.remove(&event_id);
								// 删除 event 相关缓存 (price/depth/volume)
								if let Err(e) = db::delete_event_cache(event_id).await {
									error!("Failed to delete event cache: event_id={}, error={}", event_id, e);
								}
							}
						}
						Ok(false) => {
							// 还有订单未处理完，继续等待
							info!("Event {} still has pending orders, will check again in 5 minutes", event_id);
						}
						Err(e) => {
							error!("Error checking event orders: event_id={}, error={}", event_id, e);
						}
					}
				}
			}
		}
	}
}

/// 处理单个 Market 添加
async fn handle_market_add(market_add: MarketAdd) -> anyhow::Result<()> {
	info!("Event service received MarketAdd message, event_identifier: {}, market_identifier: {}", market_add.event_identifier, market_add.market.market_identifier);

	// 1. 验证 market
	if let Err(e) = market_add.market.validate() {
		error!("MarketAdd: market {} validation failed: {}, event_identifier: {}", market_add.market.market_identifier, e, market_add.event_identifier);
		return Err(anyhow::anyhow!("Market validation failed: {}", e));
	}

	// 2. 从数据库查询 event_id、closed 和当前最大 market_id
	let pool = db::get_db_pool()?;
	let result: Option<EventQueryResult> =
		sqlx::query_as("SELECT id, closed, markets FROM events WHERE event_identifier = $1").bind(&market_add.event_identifier).fetch_optional(&pool).await.map_err(|e| {
			error!("MarketAdd: Failed to query event: {}, event_identifier: {}", e, market_add.event_identifier);
			e
		})?;

	let (event_id, event_closed, markets_json) = result.ok_or_else(|| {
		error!("MarketAdd: Event not found, event_identifier: {}", market_add.event_identifier);
		anyhow::anyhow!("Event not found: {}", market_add.event_identifier)
	})?;

	// 3. 检查 event 是否已经关闭
	if event_closed {
		error!("MarketAdd: Event already closed, event_identifier: {}", market_add.event_identifier);
		return Err(anyhow::anyhow!("Event already closed"));
	}

	// 4. 计算新的 market_id（现有最大 market_id + 1）
	let max_market_id: i16 = markets_json.0.keys().filter_map(|k| k.parse::<i16>().ok()).max().unwrap_or(0);
	let new_market_id = max_market_id + 1;
	let market_id_str = new_market_id.to_string();

	// 5. 排序 outcomes 和 token_ids
	let (outcomes, token_ids) = sort_outcomes_and_token_ids(&market_add.market.outcome_names, &market_add.market.token_ids);

	// 6. 构造 outcome_info
	let mut outcome_info = HashMap::new();
	for (token_id, outcome_name) in token_ids.iter().zip(outcomes.iter()) {
		outcome_info.insert(token_id.clone(), outcome_name.clone());
	}

	// 7. 构造 EventMarketModel
	let db_market = EventMarketModel {
		parent_collection_id: market_add.market.parent_collection_id.clone(),
		condition_id: market_add.market.condition_id.clone(),
		id: new_market_id,
		market_identifier: market_add.market.market_identifier.clone(),
		question: market_add.market.question.clone(),
		slug: market_add.market.slug.clone(),
		title: market_add.market.title.clone(),
		image: market_add.market.image.clone().unwrap_or_default(),
		outcomes: outcomes.clone(),
		token_ids: token_ids.clone(),
		win_outcome_name: String::new(),
		win_outcome_token_id: String::new(),
		closed: false,
		closed_at: None,
		volume: rust_decimal::Decimal::ZERO, // 初始交易量为0
	};

	// 8. 使用 JSONB 操作符直接在数据库中插入单个 market
	sqlx::query("UPDATE events SET markets = jsonb_set(markets, $1::text[], $2::jsonb) WHERE id = $3")
		.bind(format!("{{{}}}", market_id_str))
		.bind(sqlx::types::Json(&db_market))
		.bind(event_id)
		.execute(&pool)
		.await
		.map_err(|e| {
			error!("MarketAdd: Failed to insert market into event: {}, event_id: {}, market_id: {}", e, event_id, new_market_id);
			e
		})?;

	info!("Market added to event: event_id={}, market_id={}, market_identifier={}", event_id, new_market_id, market_add.market.market_identifier);

	// 9. 推送到 ENGINE stream
	let engine_market = EngineMQEventMarket { market_id: new_market_id, outcomes: outcomes.clone(), token_ids: token_ids.clone() };
	let engine_msg = EventInputMessage::AddOneMarket(common::engine_types::AddOneMarketMessage { event_id, market: engine_market });
	publish_to_engine_stream(EVENT_INPUT_STREAM, EVENT_INPUT_MSG_KEY, &engine_msg).await.map_err(|e| {
		error!("MarketAdd: Failed to publish to ENGINE stream: {}, event_id: {}, market_id: {}", e, event_id, new_market_id);
		e
	})?;

	// 10. 推送到 ONCHAIN stream
	let onchain_market = OnchainMQEventMarket { market_id: new_market_id, condition_id: market_add.market.condition_id.clone(), token_ids: token_ids.clone(), outcomes: outcomes.clone() };
	let onchain_msg = OnchainEventMessage::MarketAdd(common::event_types::OnchainMQMarketAdd { event_id, market: onchain_market });
	publish_to_stream(ONCHAIN_EVENT_STREAM, ONCHAIN_EVENT_MSG_KEY, &onchain_msg).await.map_err(|e| {
		error!("MarketAdd: Failed to publish to ONCHAIN stream: {}, event_id: {}, market_id: {}", e, event_id, new_market_id);
		e
	})?;

	// 11. 推送到 API stream
	let api_market = ApiMQEventMarket {
		parent_collection_id: market_add.market.parent_collection_id.clone(),
		market_id: new_market_id,
		condition_id: market_add.market.condition_id.clone(),
		market_identifier: market_add.market.market_identifier.clone(),
		question: market_add.market.question.clone(),
		slug: market_add.market.slug.clone(),
		title: market_add.market.title.clone(),
		image: market_add.market.image.clone().unwrap_or_default(),
		outcome_info: outcome_info.clone(),
		outcome_names: outcomes.clone(),
		outcome_token_ids: token_ids.clone(),
	};
	let api_msg = ApiEventMqMessage::MarketAdd(Box::new(common::event_types::ApiMQMarketAdd { event_id, market_id: new_market_id, market: api_market }));
	publish_to_stream(API_MQ_STREAM, API_MQ_MSG_KEY, &api_msg).await.map_err(|e| {
		error!("MarketAdd: Failed to publish to API stream: {}, event_id: {}, market_id: {}", e, event_id, new_market_id);
		e
	})?;

	info!("MarketAdd: Successfully added market and pushed to all streams, event_id: {}, market_id: {}", event_id, new_market_id);

	Ok(())
}

/// 处理单个 Market 关闭
async fn handle_market_close(market_close: MarketClose) -> anyhow::Result<()> {
	info!("Event service received MarketClose message, event_identifier: {}, market_identifier: {}", market_close.event_identifier, market_close.market_result.market_identifier);

	// 1. 从数据库查询 event_id 和 markets
	let pool = db::get_db_pool()?;
	let result: Option<(i64, sqlx::types::Json<HashMap<String, EventMarketModel>>)> =
		sqlx::query_as("SELECT id, markets FROM events WHERE event_identifier = $1").bind(&market_close.event_identifier).fetch_optional(&pool).await.map_err(|e| {
			error!("MarketClose: Failed to query event: {}, event_identifier: {}", e, market_close.event_identifier);
			e
		})?;

	let (event_id, markets_json) = result.ok_or_else(|| {
		error!("MarketClose: Event not found, event_identifier: {}", market_close.event_identifier);
		anyhow::anyhow!("Event not found: {}", market_close.event_identifier)
	})?;

	// 2. 查找 market_id
	let market_id = markets_json.0.iter().find(|(_, m)| m.market_identifier == market_close.market_result.market_identifier).and_then(|(id_str, _)| id_str.parse::<i16>().ok()).ok_or_else(|| {
		error!("MarketClose: Market not found, market_identifier: {}", market_close.market_result.market_identifier);
		anyhow::anyhow!("Market not found: {}", market_close.market_result.market_identifier)
	})?;

	let market_id_str = market_id.to_string();
	let closed_at = Utc::now();

	// 3. 使用 JSONB 操作符直接在数据库中更新 market 的多个字段
	sqlx::query(
		r#"
		UPDATE events
		SET markets = jsonb_set(
			jsonb_set(
				jsonb_set(
					jsonb_set(
						markets,
						ARRAY[$1, 'win_outcome_name']::text[],
						to_jsonb($2::text)
					),
					ARRAY[$1, 'win_outcome_token_id']::text[],
					to_jsonb($3::text)
				),
				ARRAY[$1, 'closed']::text[],
				to_jsonb($4::boolean)
			),
			ARRAY[$1, 'closed_at']::text[],
			to_jsonb($5::timestamptz)
		)
		WHERE id = $6
		"#,
	)
	.bind(&market_id_str)
	.bind(&market_close.market_result.win_outcome_name)
	.bind(&market_close.market_result.win_outcome_token_id)
	.bind(true)
	.bind(closed_at)
	.bind(event_id)
	.execute(&pool)
	.await
	.map_err(|e| {
		error!("MarketClose: Failed to update market: {}, event_id: {}, market_id: {}", e, event_id, market_id);
		e
	})?;

	info!("Market closed: event_id={}, market_id={}, market_identifier={}", event_id, market_id, market_close.market_result.market_identifier);

	// 5. 推送到 ENGINE stream
	let engine_msg = EventInputMessage::RemoveOneMarket(common::engine_types::RemoveOneMarketMessage { event_id, market_id });
	publish_to_engine_stream(EVENT_INPUT_STREAM, EVENT_INPUT_MSG_KEY, &engine_msg).await.map_err(|e| {
		error!("MarketClose: Failed to publish to ENGINE stream: {}, event_id: {}, market_id: {}", e, event_id, market_id);
		e
	})?;

	// 6. 推送到 ONCHAIN stream
	let onchain_msg = OnchainEventMessage::MarketClose(common::event_types::OnchainMQMarketClose {
		event_id,
		market_id,
		win_outcome_token_id: market_close.market_result.win_outcome_token_id.clone(),
		win_outcome_name: market_close.market_result.win_outcome_name.clone(),
	});
	publish_to_stream(ONCHAIN_EVENT_STREAM, ONCHAIN_EVENT_MSG_KEY, &onchain_msg).await.map_err(|e| {
		error!("MarketClose: Failed to publish to ONCHAIN stream: {}, event_id: {}, market_id: {}", e, event_id, market_id);
		e
	})?;

	// 7. 推送到 API stream
	let api_msg = ApiEventMqMessage::MarketClose(common::event_types::ApiMQMarketClose { event_id, market_id });
	publish_to_stream(API_MQ_STREAM, API_MQ_MSG_KEY, &api_msg).await.map_err(|e| {
		error!("MarketClose: Failed to publish to API stream: {}, event_id: {}, market_id: {}", e, event_id, market_id);
		e
	})?;

	info!("MarketClose: Successfully closed market and pushed to all streams, event_id: {}, market_id: {}", event_id, market_id);

	Ok(())
}
