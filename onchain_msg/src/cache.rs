use {
	std::{collections::HashMap, sync::Arc},
	tokio::sync::RwLock,
	tracing::info,
};

// token_id -> (event_id, market_id, outcome_name)
type TokenIdCache = Arc<RwLock<HashMap<String, (i64, i16, String)>>>;
// condition_id -> (event_id, market_id, token_ids)
type ConditionIdCache = Arc<RwLock<HashMap<String, (i64, i16, Vec<String>)>>>;

static TOKEN_ID_CACHE: tokio::sync::OnceCell<TokenIdCache> = tokio::sync::OnceCell::const_new();
static CONDITION_ID_CACHE: tokio::sync::OnceCell<ConditionIdCache> = tokio::sync::OnceCell::const_new();

pub fn init_cache() {
	let _ = TOKEN_ID_CACHE.set(Arc::new(RwLock::new(HashMap::new())));
	let _ = CONDITION_ID_CACHE.set(Arc::new(RwLock::new(HashMap::new())));
	info!("Cache initialized");
}

pub fn get_token_id_cache() -> &'static TokenIdCache {
	TOKEN_ID_CACHE.get().expect("Cache not initialized")
}

pub fn get_condition_id_cache() -> &'static ConditionIdCache {
	CONDITION_ID_CACHE.get().expect("Cache not initialized")
}

// Query token_id from cache or database, returns (event_id, market_id, outcome_name)
pub async fn query_token_id(token_id: &str) -> anyhow::Result<(i64, i16, String)> {
	// Try cache first
	let cache = get_token_id_cache();
	{
		let read = cache.read().await;
		if let Some(result) = read.get(token_id) {
			return Ok(result.clone());
		}
	}

	// Query from database
	// 从 event 表的 markets JSON 字段中查询包含指定 token_id 的market
	let pool = crate::db::get_db_read_pool()?;
	let row: Option<(i64, sqlx::types::Json<common::model::EventMarket>)> = sqlx::query_as(
		r#"
		SELECT m.id, opt.value
		FROM events m,
		     jsonb_each(m.markets) opt
		WHERE $1 = ANY(SELECT jsonb_array_elements_text(opt.value->'token_ids'))
		"#,
	)
	.bind(token_id)
	.fetch_optional(pool)
	.await?;

	if let Some((event_id, event_market)) = row {
		// Find the outcome_name based on token_id position
		let token_index = event_market.token_ids.iter().position(|t| t == token_id).unwrap_or(0);
		let outcome_name = event_market.outcomes.get(token_index).cloned().unwrap_or_default();
		let result = (event_id, event_market.id, outcome_name);

		// Update cache
		let mut write = cache.write().await;
		write.insert(token_id.to_string(), result.clone());
		Ok(result)
	} else {
		Err(anyhow::anyhow!("Token ID {} not found", token_id))
	}
}

// Query condition_id from cache or database
pub async fn query_condition_id(condition_id: &str) -> anyhow::Result<(i64, i16, Vec<String>)> {
	// Try cache first
	let cache = get_condition_id_cache();
	{
		let read = cache.read().await;
		if let Some(result) = read.get(condition_id) {
			return Ok(result.clone());
		}
	}

	// Query from database
	// 从 event 表的 markets JSON 字段中查询具有指定 condition_id 的选项，提取 market_id 和 token_ids
	let pool = crate::db::get_db_read_pool()?;
	let row: Option<(i64, sqlx::types::Json<common::model::EventMarket>)> = sqlx::query_as(
		r#"
		SELECT m.id, opt.value
		FROM events m,
		     jsonb_each(m.markets) opt
		WHERE opt.value->>'condition_id' = $1
		"#,
	)
	.bind(condition_id)
	.fetch_optional(pool)
	.await?;

	if let Some((event_id, event_market)) = row {
		let market_id = event_market.id;
		let token_ids = event_market.token_ids.clone();
		let result = (event_id, market_id, token_ids);

		// Update cache
		let mut write = cache.write().await;
		write.insert(condition_id.to_string(), result.clone());
		Ok(result)
	} else {
		Err(anyhow::anyhow!("Condition ID {} not found", condition_id))
	}
}

// Update cache when new event is created
pub async fn update_event_cache(event_id: i64, markets: &HashMap<String, common::event_types::OnchainMQEventMarket>) {
	let token_cache = get_token_id_cache();
	let condition_cache = get_condition_id_cache();

	let mut token_write = token_cache.write().await;
	let mut condition_write = condition_cache.write().await;

	for market in markets.values() {
		// Update condition_id cache - 只缓存 market_id 和 token_ids
		condition_write.insert(market.condition_id.clone(), (event_id, market.market_id, market.token_ids.clone()));

		// Update token_id cache with outcome_name
		for (idx, token_id) in market.token_ids.iter().enumerate() {
			let outcome_name = market.outcomes.get(idx).cloned().unwrap_or_default();
			token_write.insert(token_id.clone(), (event_id, market.market_id, outcome_name));
		}
	}

	info!("Updated cache for event_id={} with {} markets", event_id, markets.len());
}

// Query outcome_name from token_id (uses unified token_id cache)
pub async fn query_outcome_name(token_id: &str) -> anyhow::Result<String> {
	let (_, _, outcome_name) = query_token_id(token_id).await?;
	Ok(outcome_name)
}
