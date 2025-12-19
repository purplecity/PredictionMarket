use {
	common::{
		common_env,
		engine_types::OrderStatus,
		key::{DEPTH_CACHE_KEY, PRICE_CACHE_KEY, VOLUME_CACHE_KEY, market_field},
		model::{EventMarket, Events},
		postgres_pool::{create_default_postgres_pool_config, init_postgres_pool},
		redis_pool,
	},
	sqlx::PgPool,
	tokio::sync::OnceCell,
};

static DB_POOL: OnceCell<PgPool> = OnceCell::const_new();

/// 初始化数据库连接池
pub async fn init_db_pool() -> anyhow::Result<()> {
	let env = common_env::get_common_env();
	let config = create_default_postgres_pool_config();

	let pool = init_postgres_pool(&env.postgres_write_host, env.postgres_write_port, &env.postgres_write_user, &env.postgres_write_password, &env.postgres_write_database, config).await?;
	DB_POOL.set(pool)?;
	Ok(())
}

/// 获取数据库连接池
pub fn get_db_pool() -> anyhow::Result<PgPool> {
	DB_POOL.get().cloned().ok_or_else(|| anyhow::anyhow!("DB pool not initialized"))
}

/*
sqlx::query! 是一个 “编译期检查 SQL” 的宏，它需要：
连接到真实的数据库（通过 DATABASE_URL），或者
使用预先生成的查询缓存文件（sqlx-data.json）

*/
/// 插入市场
pub async fn insert_event(event: &Events) -> anyhow::Result<i64> {
	let pool = get_db_pool()?;
	let markets_json = serde_json::to_value(&event.markets.0)?;
	let id: i64 = sqlx::query_scalar(
		r#"
		INSERT INTO events (
			event_identifier, slug, title, description, image,
			end_date, closed, closed_at, resolved, resolved_at,
			topic, volume, markets, recommended, created_at
		)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
		RETURNING id
		"#,
	)
	.bind(&event.event_identifier)
	.bind(&event.slug)
	.bind(&event.title)
	.bind(&event.description)
	.bind(&event.image)
	.bind(event.end_date)
	.bind(event.closed)
	.bind(event.closed_at)
	.bind(event.resolved)
	.bind(event.resolved_at)
	.bind(&event.topic)
	.bind(event.volume)
	.bind(&markets_json)
	.bind(event.recommended)
	.bind(event.created_at)
	.fetch_one(&pool)
	.await?;

	Ok(id)
}

/*
query!	查询多列或多行	匿名 struct（带字段名）	row.id, row.name 但是是在编译期检查的
query_as!	查询多列，映射到自定义 struct	你的 struct 类型	event.id
query_scalar!	只查一个值（单列）	直接是那个值的类型	i64, String, serde_json::Value
否则就是query获得一个row 然后get0 get1去拿多列值
*/

/// 根据 event_identifier 更新市场的 closed、closed_at 和 markets 字段，并返回 event_id
/// markets_results: Vec<(market_identifier, win_outcome_token_id, win_outcome_name)>
pub async fn update_event_closed(event_identifier: &str, markets_results: &[(String, String, String)]) -> anyhow::Result<i64> {
	let pool = get_db_pool()?;
	let now = chrono::Utc::now();

	// 1. 先查询获取 event_id 和当前 markets，用于找到每个 market 的 key
	let result: Option<(i64, serde_json::Value)> = sqlx::query_as("SELECT id, markets FROM events WHERE event_identifier = $1").bind(event_identifier).fetch_optional(&pool).await?;

	let (event_id, markets_json) = result.ok_or_else(|| anyhow::anyhow!("Event not found: {}", event_identifier))?;

	// 解析 markets 以找到 market_identifier 对应的 key（market_id字符串）
	let markets_map: std::collections::HashMap<String, EventMarket> = serde_json::from_value(markets_json)?;

	// 2. 开始事务，使用 JSONB 操作符逐个更新 market
	let mut tx = pool.begin().await?;

	for (market_identifier, win_outcome_token_id, win_outcome_name) in markets_results {
		// 找到对应的 market_id（即 HashMap 的 key）
		let market_key = markets_map.iter().find(|(_, market)| &market.market_identifier == market_identifier).map(|(key, _)| key);

		if let Some(key) = market_key {
			// 使用 JSONB 操作符更新单个 market 的字段
			sqlx::query(
				r#"
				UPDATE events
				SET markets = jsonb_set(
					jsonb_set(
						markets,
						ARRAY[$1, 'win_outcome_token_id']::text[],
						to_jsonb($2::text)
					),
					ARRAY[$1, 'win_outcome_name']::text[],
					to_jsonb($3::text)
				)
				WHERE event_identifier = $4
				"#,
			)
			.bind(key)
			.bind(win_outcome_token_id)
			.bind(win_outcome_name)
			.bind(event_identifier)
			.execute(&mut *tx)
			.await?;
		}
	}

	// 3. 更新 event 的 closed 和 closed_at
	sqlx::query(
		r#"
		UPDATE events
		SET closed = true, closed_at = $1
		WHERE event_identifier = $2
		"#,
	)
	.bind(now)
	.bind(event_identifier)
	.execute(&mut *tx)
	.await?;

	// 4. 提交事务
	tx.commit().await?;

	Ok(event_id)
}

/// 检查市场是否所有订单都已处理完（status 不为 New 和 PartiallyFilled 则认为已处理）
pub async fn check_event_orders_resolved(event_id: i64) -> anyhow::Result<bool> {
	let pool = get_db_pool()?;
	let count: Option<i64> = sqlx::query_scalar(
		r#"
		SELECT COUNT(*)
		FROM orders
		WHERE event_id = $1
		AND status IN ($2, $3)
		"#,
	)
	.bind(event_id)
	.bind(OrderStatus::New)
	.bind(OrderStatus::PartiallyFilled)
	.fetch_one(&pool)
	.await?;

	Ok(count.unwrap_or(0) == 0)
}

/// 更新市场为 resolved
pub async fn update_event_resolved(event_id: i64) -> anyhow::Result<()> {
	let pool = get_db_pool()?;
	let now = Some(chrono::Utc::now());
	sqlx::query(
		r#"
		UPDATE events
		SET resolved = true, resolved_at = $1
		WHERE id = $2
		"#,
	)
	.bind(now)
	.bind(event_id)
	.execute(&pool)
	.await?;

	Ok(())
}

/// 查询所有 closed 但未 resolved 的市场 ID
pub async fn get_pending_resolved_events() -> anyhow::Result<Vec<i64>> {
	let pool = get_db_pool()?;
	let event_ids: Vec<i64> = sqlx::query_scalar(
		r#"
		SELECT id
		FROM events
		WHERE closed = true AND resolved = false
		"#,
	)
	.fetch_all(&pool)
	.await?;

	Ok(event_ids)
}

/// 根据 event_id 获取 event 信息
pub async fn get_event_by_id(event_id: i64) -> anyhow::Result<Option<Events>> {
	let pool = get_db_pool()?;
	let event: Option<Events> = sqlx::query_as(
		r#"
		SELECT *
		FROM events
		WHERE id = $1
		"#,
	)
	.bind(event_id)
	.fetch_optional(&pool)
	.await?;

	Ok(event)
}

/// 删除 event 相关的缓存 (price/depth/volume)
pub async fn delete_event_cache(event_id: i64) -> anyhow::Result<()> {
	// 获取 event 信息以获取 market_ids
	let event = match get_event_by_id(event_id).await? {
		Some(e) => e,
		None => return Ok(()), // event 不存在，无需删除
	};

	let mut conn = redis_pool::get_cache_redis_connection().await?;

	// 构建需要删除的 fields
	let fields: Vec<String> = event
		.markets
		.keys()
		.map(|market_id_str| {
			let market_id: i16 = market_id_str.parse().unwrap_or(0);
			market_field(event_id, market_id)
		})
		.collect();

	// Pipeline 删除 price 和 depth 缓存
	let mut pipe = redis::pipe();
	for field in &fields {
		pipe.hdel(PRICE_CACHE_KEY, field);
		pipe.hdel(DEPTH_CACHE_KEY, field);
	}
	// 删除 volume 缓存
	pipe.hdel(VOLUME_CACHE_KEY, event_id.to_string());

	let _: () = pipe.query_async(&mut conn).await?;

	tracing::info!("Deleted cache for event_id={}, market_count={}", event_id, fields.len());

	Ok(())
}

/// 检查 topic 是否存在且为 active
pub async fn check_topic_exists(topic: &str) -> anyhow::Result<bool> {
	let pool = get_db_pool()?;
	let exists: Option<bool> = sqlx::query_scalar(
		r#"
		SELECT EXISTS(
			SELECT 1 FROM event_topics
			WHERE topic = $1 AND active = true
		)
		"#,
	)
	.bind(topic)
	.fetch_one(&pool)
	.await?;

	Ok(exists.unwrap_or(false))
}
