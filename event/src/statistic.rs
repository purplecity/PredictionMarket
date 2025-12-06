use {
	crate::{consts::STATISTIC_INTERVAL_SECS, db::get_db_pool},
	common::{
		event_types::{CacheEventVolume, CacheMarketVolume},
		key::VOLUME_CACHE_KEY,
	},
	rust_decimal::Decimal,
	std::collections::HashMap,
	tokio::time::{Duration, interval},
	tracing::{error, info},
};

/// 启动统计任务
pub async fn start_statistic_task() {
	info!("Starting event volume statistic task with interval: {}s", STATISTIC_INTERVAL_SECS);

	let mut tick_interval = interval(Duration::from_secs(STATISTIC_INTERVAL_SECS));
	tick_interval.tick().await;

	loop {
		tick_interval.tick().await;

		if let Err(e) = run_statistic().await {
			error!("Failed to run statistic: {}", e);
		}
	}
}

/// 执行统计任务
async fn run_statistic() -> anyhow::Result<()> {
	let pool = get_db_pool()?;

	// 1. 获取所有未关闭的市场 ID
	let active_event_ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM events WHERE closed = false ORDER BY id").fetch_all(&pool).await?;

	if active_event_ids.is_empty() {
		info!("No active events to update volume");
		return Ok(());
	}

	info!("Updating volume for {} active events", active_event_ids.len());

	// 2. 使用 SQL 聚合统计每个市场和选项的交易量
	// 只统计 taker = true 的记录
	let volume_stats: Vec<(i64, i16, Decimal)> = sqlx::query_as(
		"SELECT event_id, market_id, COALESCE(SUM(trade_volume), 0) as total_volume
		 FROM trades
		 WHERE taker = true AND event_id = ANY($1)
		 GROUP BY event_id, market_id
		 ORDER BY event_id, market_id",
	)
	.bind(&active_event_ids)
	.fetch_all(&pool)
	.await?;

	// 3. 组织数据：event_id -> CacheEventVolume
	let mut event_volumes: HashMap<i64, CacheEventVolume> = HashMap::new();

	for (event_id, market_id, volume) in volume_stats {
		let event_volume = event_volumes.entry(event_id).or_insert(CacheEventVolume { event_id, total_volume: Decimal::ZERO, market_volumes: Vec::new() });

		event_volume.market_volumes.push(CacheMarketVolume { market_id, volume });
		event_volume.total_volume += volume;
	}

	// 确保所有活跃市场都有记录（即使交易量为 0）
	for event_id in &active_event_ids {
		event_volumes.entry(*event_id).or_insert(CacheEventVolume { event_id: *event_id, total_volume: Decimal::ZERO, market_volumes: Vec::new() });
	}

	// 4. 批量更新 event 表
	if !event_volumes.is_empty() {
		let event_ids: Vec<i64> = event_volumes.values().map(|mv| mv.event_id).collect();
		let volumes: Vec<Decimal> = event_volumes.values().map(|mv| mv.total_volume).collect();

		sqlx::query(
			"UPDATE events
			 SET volume = updates.volume
			 FROM (
			   SELECT UNNEST($1::bigint[]) as id,
			          UNNEST($2::decimal[]) as volume
			 ) AS updates
			 WHERE events.id = updates.id",
		)
		.bind(&event_ids)
		.bind(&volumes)
		.execute(&pool)
		.await?;
	}

	// 5. 批量更新 Redis 缓存（使用 pipeline）
	update_redis_cache(&event_volumes).await?;

	info!("Successfully updated volume for {} events", event_volumes.len());

	Ok(())
}

/// 批量更新 Redis 缓存
/// key: "volume", field: event_id, value: CacheEventVolume JSON
async fn update_redis_cache(event_volumes: &HashMap<i64, CacheEventVolume>) -> anyhow::Result<()> {
	let mut conn = common::redis_pool::get_cache_redis_connection().await?;

	// 创建 pipeline
	let mut pipe = redis::pipe();

	for event_volume in event_volumes.values() {
		let field = event_volume.event_id.to_string();
		let value = serde_json::to_string(event_volume)?;
		pipe.hset(VOLUME_CACHE_KEY, &field, value);
	}

	// 批量执行
	let _: () = pipe.query_async(&mut conn).await?;

	Ok(())
}
