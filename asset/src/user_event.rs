use {
	crate::consts::USER_EVENT_BATCH_SIZE,
	common::{
		consts::{USER_EVENT_MSG_KEY, USER_EVENT_STREAM},
		redis_pool,
		websocket_types::{PositionChangeEvent, UserEvent, UserPositions},
	},
	redis::pipe,
	rust_decimal::Decimal,
	tokio::sync::mpsc,
	tracing::{error, info},
};

/// 仓位事件类型
pub enum PositionEventType {
	/// 创建（首次获得仓位）
	Created,
	/// 更新（仓位变化，如果 quantity 为 0 会自动转为 Removed）
	Updated,
	/// 移除（仓位清零）
	Removed,
}

/// 仓位事件信息
pub struct PositionEvent {
	pub event_type: PositionEventType,
	pub privy_id: String,
	pub event_id: i64,
	pub market_id: i16,
	pub outcome_name: String,
	pub token_id: String,
	pub quantity: Decimal,
	pub avg_price: Decimal,
	pub update_id: i64,
}

// 全局 channel sender
static EVENT_SENDER: tokio::sync::OnceCell<mpsc::UnboundedSender<PositionEvent>> = tokio::sync::OnceCell::const_new();

/// 初始化 event channel 并启动消费者 task
pub fn init_event_channel() {
	let (tx, rx) = mpsc::unbounded_channel::<PositionEvent>();
	let _ = EVENT_SENDER.set(tx);

	tokio::spawn(event_consumer_task(rx));
	info!("User event channel initialized");
}

/// 发送仓位创建事件到 channel
pub fn send_position_created(privy_id: String, event_id: i64, market_id: i16, outcome_name: String, token_id: String, avg_price: Decimal, quantity: Decimal, update_id: i64) {
	send_event(PositionEvent { event_type: PositionEventType::Created, privy_id, event_id, market_id, outcome_name, token_id, quantity, avg_price, update_id });
}

/// 发送仓位更新事件到 channel（如果 quantity 为 0，自动转为移除事件）
pub fn send_position_updated(privy_id: String, event_id: i64, market_id: i16, outcome_name: String, token_id: String, avg_price: Decimal, quantity: Decimal, update_id: i64) {
	let event_type = if quantity.is_zero() { PositionEventType::Removed } else { PositionEventType::Updated };
	send_event(PositionEvent { event_type, privy_id, event_id, market_id, outcome_name, token_id, quantity, avg_price, update_id });
}

/// 发送仓位移除事件到 channel
pub fn send_position_removed(privy_id: String, event_id: i64, market_id: i16, token_id: String, update_id: i64) {
	send_event(PositionEvent {
		event_type: PositionEventType::Removed,
		privy_id,
		event_id,
		market_id,
		outcome_name: String::new(),
		token_id,
		quantity: Decimal::ZERO,
		avg_price: Decimal::ZERO,
		update_id,
	});
}

/// 发送事件到 channel（内部函数）
fn send_event(event: PositionEvent) {
	if let Some(sender) = EVENT_SENDER.get() {
		let _ = sender.send(event);
	}
}

/// 批量发送仓位更新事件到 channel（如果 quantity 为 0，自动转为移除事件）
pub fn send_events(events: Vec<PositionEvent>) {
	if let Some(sender) = EVENT_SENDER.get() {
		for mut event in events {
			// 如果是 Updated 事件且 quantity 为 0，自动转为 Removed 事件
			if matches!(event.event_type, PositionEventType::Updated) && event.quantity.is_zero() {
				event.event_type = PositionEventType::Removed;
			}
			let _ = sender.send(event);
		}
	}
}

/// 消费者 task：从 channel 读取事件，批量发送到 Redis stream
async fn event_consumer_task(mut rx: mpsc::UnboundedReceiver<PositionEvent>) {
	let mut batch: Vec<PositionEvent> = Vec::with_capacity(USER_EVENT_BATCH_SIZE);

	loop {
		// recv_many 会阻塞直到有消息，然后尽可能多地读取（最多 BATCH_SIZE 个）
		let count = rx.recv_many(&mut batch, USER_EVENT_BATCH_SIZE).await;
		if count == 0 {
			// channel 关闭
			break;
		}

		if let Err(e) = send_batch_to_redis(&batch).await {
			error!("Failed to send user events to redis: {}", e);
		}
		batch.clear();
	}

	info!("User event consumer task exited");
}

/// 批量发送事件到 Redis stream（使用 pipeline）
async fn send_batch_to_redis(events: &[PositionEvent]) -> anyhow::Result<()> {
	let mut conn = redis_pool::get_websocket_mq_connection().await?;

	let mut event_jsons: Vec<String> = Vec::with_capacity(events.len());
	for event in events {
		let user_event = match event.event_type {
			PositionEventType::Created => {
				UserEvent::PositionChange(PositionChangeEvent::PositionCreated(UserPositions {
					privy_id: event.privy_id.clone(),
					event_id: event.event_id,
					market_id: event.market_id,
					outcome_name: event.outcome_name.clone(),
					token_id: event.token_id.clone(),
					avg_price: event.avg_price.normalize().to_string(),
					quantity: event.quantity.normalize().to_string(),
					update_id: event.update_id,
				}))
			}
			PositionEventType::Updated => {
				UserEvent::PositionChange(PositionChangeEvent::PositionUpdated(UserPositions {
					privy_id: event.privy_id.clone(),
					event_id: event.event_id,
					market_id: event.market_id,
					outcome_name: event.outcome_name.clone(),
					token_id: event.token_id.clone(),
					avg_price: event.avg_price.normalize().to_string(),
					quantity: event.quantity.normalize().to_string(),
					update_id: event.update_id,
				}))
			}
			PositionEventType::Removed => {
				UserEvent::PositionChange(PositionChangeEvent::PositionRemoved {
					event_id: event.event_id,
					market_id: event.market_id,
					token_id: event.token_id.clone(),
					privy_id: event.privy_id.clone(),
					update_id: event.update_id,
				})
			}
		};
		event_jsons.push(serde_json::to_string(&user_event)?);
	}

	let mut pipeline = pipe();
	for json in &event_jsons {
		pipeline.xadd_maxlen(USER_EVENT_STREAM, redis::streams::StreamMaxlen::Approx(common::consts::USER_EVENT_STREAM_MAXLEN), "*", &[(USER_EVENT_MSG_KEY, json.as_str())]);
	}
	pipeline.query_async::<Vec<String>>(&mut conn).await?;

	Ok(())
}
