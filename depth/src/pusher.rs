use {
	crate::{
		consts::{DEPTH_PUSH_BATCH_SIZE, DEPTH_PUSH_INTERVAL_SECS},
		storage::DepthStorage,
	},
	common::{
		consts::{WEBSOCKET_MSG_KEY, WEBSOCKET_STREAM},
		depth_types::{CacheMarketPriceInfo, CacheTokenPriceInfo},
		key::{DEPTH_CACHE_KEY, PRICE_CACHE_KEY, market_field},
		websocket_types::WebSocketDepth,
	},
	std::{collections::HashMap, sync::Arc},
	tokio::{
		sync::RwLock,
		time::{Duration, interval},
	},
	tracing::{error, info},
};

/// 启动定时推送任务
pub async fn start_pusher_task(storage: Arc<RwLock<DepthStorage>>) {
	let mut shutdown_receiver = crate::consumer::get_shutdown_receiver();

	info!("Starting depth pusher task with interval: {}s", DEPTH_PUSH_INTERVAL_SECS);

	let mut tick_interval = interval(Duration::from_secs(DEPTH_PUSH_INTERVAL_SECS));
	tick_interval.tick().await;

	loop {
		tokio::select! {
			_ = tick_interval.tick() => {
				if let Err(e) = push_depths(&storage).await {
					error!("Failed to push depths: {}", e);
				}
			}
			_ = shutdown_receiver.recv() => {
				info!("Pusher received shutdown signal");
				break;
			}
		}
	}
}

/// 推送深度快照到 WebSocket Stream 和缓存 Redis
async fn push_depths(storage: &Arc<RwLock<DepthStorage>>) -> anyhow::Result<()> {
	// 获取所有深度快照（market级别）
	let depths = storage.read().await.get_all_depths();

	if depths.is_empty() {
		return Ok(());
	}

	info!("Pushing {} depth snapshots", depths.len());

	// 1. 分批次推送到 WebSocket Stream
	push_to_websocket_stream(&depths).await?;

	// 2. 批量更新缓存 Redis
	update_cache_redis(&depths).await?;

	Ok(())
}

/// 分批次推送到 WebSocket Stream
async fn push_to_websocket_stream(depths: &[WebSocketDepth]) -> anyhow::Result<()> {
	let mut conn = common::redis_pool::get_websocket_mq_connection().await?;

	for chunk in depths.chunks(DEPTH_PUSH_BATCH_SIZE) {
		let mut pipe = redis::pipe();

		for depth in chunk {
			let depth_json = serde_json::to_string(depth)?;
			pipe.xadd_maxlen(WEBSOCKET_STREAM, redis::streams::StreamMaxlen::Approx(common::consts::WEBSOCKET_STREAM_MAXLEN), "*", &[(WEBSOCKET_MSG_KEY, depth_json.as_str())]);
		}

		// 批量执行
		let _: () = pipe.query_async(&mut conn).await?;
	}

	Ok(())
}

/// 批量更新缓存 Redis (使用 HSET)
/// hash key: "depth", field: event_id::market_id, value: WebSocketDepth JSON
/// hash key: "price", field: event_id::market_id, value: CacheMarketPriceInfo JSON
async fn update_cache_redis(depths: &[WebSocketDepth]) -> anyhow::Result<()> {
	let mut conn = common::redis_pool::get_cache_redis_connection().await?;

	let mut pipe = redis::pipe();

	for depth in depths {
		let field = market_field(depth.event_id, depth.market_id);

		// HSET depth field value
		let depth_value = serde_json::to_string(depth)?;
		pipe.hset(DEPTH_CACHE_KEY, &field, depth_value);

		// 构建 CacheMarketPriceInfo
		let mut prices = HashMap::new();
		for (token_id, token_info) in &depth.depths {
			let best_bid = token_info.bids.first().map(|p| p.price.clone()).unwrap_or_default();
			let best_ask = token_info.asks.first().map(|p| p.price.clone()).unwrap_or_default();
			prices.insert(token_id.clone(), CacheTokenPriceInfo { best_bid, best_ask, latest_trade_price: token_info.latest_trade_price.clone() });
		}
		let price_info = CacheMarketPriceInfo { update_id: depth.update_id, timestamp: depth.timestamp, prices };

		// HSET price field value
		let price_value = serde_json::to_string(&price_info)?;
		pipe.hset(PRICE_CACHE_KEY, &field, price_value);
	}

	// 批量执行
	let _: () = pipe.query_async(&mut conn).await?;

	Ok(())
}
