use {
	crate::cache,
	common::{
		consts::{ONCHAIN_EVENT_MSG_KEY, ONCHAIN_EVENT_STREAM},
		event_types::OnchainEventMessage,
		redis_pool,
	},
	redis::AsyncCommands,
	std::time::Duration,
	tracing::{error, info},
};

pub async fn start_consumer() -> anyhow::Result<()> {
	tokio::spawn(async {
		if let Err(e) = consumer_task().await {
			error!("Event consumer error: {}", e);
		}
	});

	info!("Started event consumer");
	Ok(())
}

async fn consumer_task() -> anyhow::Result<()> {
	let mut shutdown_receiver = crate::init::get_shutdown_receiver();
	let mut conn = redis_pool::get_common_mq_connection().await?;

	//其实是可以多个副本的 另外两个消费者组跟其他进程都是消费者组 然后各自删除
	let mut last_id = "$".to_string();

	loop {
		tokio::select! {
			result = read_messages(&mut conn, &last_id) => {
				match result {
					Ok(new_last_id) => {
						// 更新 last_id
						if let Some(id) = new_last_id {
							last_id = id;
						}
					}
					Err(e) => {
						error!("Consumer read error: {}, reconnecting...", e);
						// 重新获取连接
						match redis_pool::get_common_mq_connection().await {
							Ok(new_conn) => {
								conn = new_conn;
								info!("Redis connection reestablished");
							}
							Err(e) => {
								error!("Failed to reconnect: {}", e);
								tokio::time::sleep(Duration::from_secs(1)).await;
							}
						}
					}
				}
			}
			_ = shutdown_receiver.recv() => {
				info!("Event consumer received shutdown signal");
				break;
			}
		}
	}

	Ok(())
}

async fn read_messages(conn: &mut impl AsyncCommands, last_id: &str) -> anyhow::Result<Option<String>> {
	use redis::streams::{StreamReadOptions, StreamReadReply};

	// block(0) 表示永久阻塞直到有新消息
	let opts = StreamReadOptions::default().count(1).block(0);

	let result: StreamReadReply = conn.xread_options(&[ONCHAIN_EVENT_STREAM], &[last_id], &opts).await?;

	let mut new_last_id = None;

	for stream_key in result.keys {
		for stream_id in stream_key.ids {
			let msg_id = stream_id.id.clone();
			new_last_id = Some(msg_id.clone());

			if let Some(value) = stream_id.map.get(ONCHAIN_EVENT_MSG_KEY) {
				match redis::from_redis_value::<String>(value) {
					Ok(json_str) => {
						info!("Received event message {}: {}", msg_id, json_str);
						if let Err(e) = process_message(&json_str).await {
							error!("Failed to process event message {}: {}", msg_id, e);
						}
					}
					Err(e) => {
						error!("Failed to parse redis value for message {}: {}", msg_id, e);
					}
				}
			}
		}
	}

	Ok(new_last_id)
}

async fn process_message(json_str: &str) -> anyhow::Result<()> {
	let message: OnchainEventMessage = serde_json::from_str(json_str)?;

	match message {
		OnchainEventMessage::Create(event) => handle_event_create(event).await?,
		OnchainEventMessage::Close(close) => handle_event_close(close).await?,
		OnchainEventMessage::MarketAdd(market_add) => handle_market_add(market_add).await?,
		OnchainEventMessage::MarketClose(market_close) => handle_market_close(market_close).await?,
	}

	Ok(())
}

async fn handle_event_create(event: common::event_types::OnchainMQEventCreate) -> anyhow::Result<()> {
	info!("Handling EventCreate: event_id={}, market_count={}", event.event_id, event.markets.len());
	info!("TEST_EVENT: Onchain_msg service received EventCreate from onchain_event_stream, event_id: {}", event.event_id);

	// Update cache with event information
	cache::update_event_cache(event.event_id, &event.markets).await;

	Ok(())
}

async fn handle_event_close(close: common::event_types::MQEventClose) -> anyhow::Result<()> {
	info!("Handling EventClose: event_id={}", close.event_id);
	info!("TEST_EVENT: Onchain_msg service received EventClose from onchain_event_stream, event_id: {}", close.event_id);

	// No special action needed for event close in onchain_msg service
	// Cache remains valid for historical lookups

	Ok(())
}

async fn handle_market_add(market_add: common::event_types::OnchainMQMarketAdd) -> anyhow::Result<()> {
	info!("Handling MarketAdd: event_id={}, market_id={}", market_add.event_id, market_add.market.market_id);

	// Update cache with new market information
	let token_cache = cache::get_token_id_cache();
	let condition_cache = cache::get_condition_id_cache();

	let mut token_write = token_cache.write().await;
	let mut condition_write = condition_cache.write().await;

	let market = &market_add.market;

	// Update condition_id cache
	condition_write.insert(market.condition_id.clone(), (market_add.event_id, market.market_id, market.token_ids.clone()));

	// Update token_id cache with outcome_name
	for (idx, token_id) in market.token_ids.iter().enumerate() {
		let outcome_name = market.outcomes.get(idx).cloned().unwrap_or_default();
		token_write.insert(token_id.clone(), (market_add.event_id, market.market_id, outcome_name));
	}

	info!("Updated cache for market_add: event_id={}, market_id={}", market_add.event_id, market.market_id);

	Ok(())
}

async fn handle_market_close(market_close: common::event_types::OnchainMQMarketClose) -> anyhow::Result<()> {
	info!("Handling MarketClose: event_id={}, market_id={}, win_outcome={}", market_close.event_id, market_close.market_id, market_close.win_outcome_name);

	// No special action needed for market close in onchain_msg service
	// Cache remains valid for historical lookups

	Ok(())
}
