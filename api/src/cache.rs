use {
	crate::db::get_db_read_pool,
	common::event_types::ApiMQEventCreate,
	std::collections::HashMap,
	thiserror::Error,
	tokio::sync::{OnceCell, RwLock},
};

/// 缓存相关错误类型
#[derive(Debug, Error)]
pub enum CacheError {
	#[error("User not found: {0}")]
	UserNotFound(String),

	#[error("Event not found: {0}")]
	EventNotFound(i64),

	#[error("Market not found: event_id={0}, market_id={1}")]
	MarketNotFound(i64, i16),

	#[error("Cache not initialized")]
	NotInitialized,

	#[error("Database error: {0}")]
	Database(#[from] sqlx::Error),
}

/// 全局缓存：privy_id -> id 的映射
static PRIVY_ID_TO_ID_CACHE: OnceCell<RwLock<HashMap<String, i64>>> = OnceCell::const_new();

/// 全局缓存：event_id -> ApiMQEventCreate 的映射
static EVENT_CACHE: OnceCell<RwLock<HashMap<i64, ApiMQEventCreate>>> = OnceCell::const_new();

/// 初始化缓存
pub fn init_cache() {
	let privy_cache = RwLock::new(HashMap::new());
	PRIVY_ID_TO_ID_CACHE.set(privy_cache).expect("Privy cache already initialized");

	let event_cache = RwLock::new(HashMap::new());
	EVENT_CACHE.set(event_cache).expect("Event cache already initialized");
}

/// 插入 privy_id -> user_id 映射到缓存
pub async fn insert_user_id_cache(privy_id: &str, user_id: i64) {
	if let Some(cache) = PRIVY_ID_TO_ID_CACHE.get() {
		let mut write_guard = cache.write().await;
		write_guard.insert(privy_id.to_string(), user_id);
	}
}

/// 根据 privy_id 查询用户 id
/// 首先从缓存中查找，如果找不到则从数据库中查询
pub async fn get_user_id_by_privy_id(privy_id: &str) -> Result<i64, CacheError> {
	let cache = PRIVY_ID_TO_ID_CACHE.get().ok_or(CacheError::NotInitialized)?;

	// 1. 先从缓存中查找
	{
		let read_guard = cache.read().await;
		if let Some(&id) = read_guard.get(privy_id) {
			return Ok(id);
		}
	}

	// 2. 缓存中没有，从数据库中查询
	let read_pool = get_db_read_pool().map_err(|_| CacheError::NotInitialized)?;
	let user_id: Option<i64> = sqlx::query_scalar("SELECT id FROM users WHERE privy_id = $1").bind(privy_id).fetch_optional(&read_pool).await.map_err(CacheError::Database)?;

	match user_id {
		Some(id) => {
			// 3. 查到了，插入缓存并返回
			let mut write_guard = cache.write().await;
			write_guard.insert(privy_id.to_string(), id);
			Ok(id)
		}
		None => {
			// 4. 还查不到，返回没找到错误
			Err(CacheError::UserNotFound(privy_id.to_string()))
		}
	}
}

/// 根据 event_id 查询市场信息
/// 首先从缓存中查找，如果找不到则从数据库中查询
pub async fn get_event(event_id: i64) -> Result<ApiMQEventCreate, CacheError> {
	let cache = EVENT_CACHE.get().ok_or(CacheError::NotInitialized)?;

	// 1. 先从缓存中查找
	{
		let read_guard = cache.read().await;
		if let Some(event) = read_guard.get(&event_id) {
			return Ok(event.clone());
		}
	}

	// 2. 缓存中没有，从数据库中查询
	let read_pool = get_db_read_pool().map_err(|_| CacheError::NotInitialized)?;
	let event: Option<common::model::Events> = sqlx::query_as("SELECT * FROM events WHERE id = $1 AND closed = false").bind(event_id).fetch_optional(&read_pool).await.map_err(CacheError::Database)?;

	match event {
		Some(event_model) => {
			// 3. 查到了，转换为 ApiMQEventCreate 并插入缓存
			let api_event = convert_event_to_api_mq_event_create(event_model);
			let mut write_guard = cache.write().await;
			write_guard.insert(event_id, api_event.clone());
			Ok(api_event)
		}
		None => {
			// 4. 还查不到，返回没找到错误
			Err(CacheError::EventNotFound(event_id))
		}
	}
}

/// 插入或更新市场缓存（内部使用）
pub(crate) async fn insert_event(event: ApiMQEventCreate) {
	let cache = EVENT_CACHE.get().expect("Event cache not initialized");
	let mut write_guard = cache.write().await;
	write_guard.insert(event.event_id, event);
}

/// 从缓存中删除市场（内部使用）
pub(crate) async fn remove_event(event_id: i64) {
	let cache = EVENT_CACHE.get().expect("Event cache not initialized");
	let mut write_guard = cache.write().await;
	write_guard.remove(&event_id);
}

/// 获取指定 event_id 和 market_id 的市场信息
pub async fn get_market(event_id: i64, market_id: i16) -> Result<common::event_types::ApiMQEventMarket, CacheError> {
	let event = get_event(event_id).await?;
	let market_id_str = market_id.to_string();
	event.markets.get(&market_id_str).cloned().ok_or(CacheError::MarketNotFound(event_id, market_id))
}

/// 批量获取多个事件信息
/// 提供多个 event_id，统一遍历去内存中找，找不到的拎出来放成一个待查询的列表
/// 找得到的收集已找到的结果，然后把待查询的列表一次性从 events 表中拿到
/// 构造内存中的 ApiMQEventCreate 信息，更新缓存，然后最后跟已经找到的结果一起返回
pub async fn get_events(event_ids: &[i64]) -> Result<HashMap<i64, ApiMQEventCreate>, CacheError> {
	let cache = EVENT_CACHE.get().ok_or(CacheError::NotInitialized)?;

	let mut found_events: HashMap<i64, ApiMQEventCreate> = HashMap::new();
	let mut missing_event_ids: Vec<i64> = Vec::new();

	// 1. 先从缓存中查找
	{
		let read_guard = cache.read().await;
		for &event_id in event_ids {
			if let Some(event) = read_guard.get(&event_id) {
				found_events.insert(event_id, event.clone());
			} else {
				missing_event_ids.push(event_id);
			}
		}
	}

	// 2. 如果有缺失的 event_id，从数据库批量查询
	if !missing_event_ids.is_empty() {
		let read_pool = get_db_read_pool().map_err(|_| CacheError::NotInitialized)?;
		let db_events: Vec<common::model::Events> =
			sqlx::query_as("SELECT * FROM events WHERE id = ANY($1) AND closed = false").bind(&missing_event_ids).fetch_all(&read_pool).await.map_err(CacheError::Database)?;

		// 3. 转换为 ApiMQEventCreate 并更新缓存
		let mut write_guard = cache.write().await;
		for event_model in db_events {
			let api_event = convert_event_to_api_mq_event_create(event_model);
			let event_id = api_event.event_id;
			write_guard.insert(event_id, api_event.clone());
			found_events.insert(event_id, api_event);
		}
	}

	Ok(found_events)
}

/// 批量获取多个市场信息
/// 提供多个 (event_id, market_id) 元组列表
/// 收集所有的 event_id 去重后构造一个列表，然后调用 get_events 得到结果信息
/// 再遍历请求参数根据 event_id 和 market_id 从 get_events 中得到的信息去查询返回结果
pub async fn get_markets(requests: &[(i64, i16)]) -> Result<HashMap<(i64, i16), common::event_types::ApiMQEventMarket>, CacheError> {
	// 1. 收集所有的 event_id 并去重
	let event_ids: Vec<i64> = requests.iter().map(|(event_id, _)| *event_id).collect::<std::collections::HashSet<_>>().into_iter().collect();

	// 2. 调用 get_events 批量获取事件信息
	let events = get_events(&event_ids).await?;

	// 3. 遍历请求参数，从 events 中提取对应的 market
	let mut markets: HashMap<(i64, i16), common::event_types::ApiMQEventMarket> = HashMap::new();

	for &(event_id, market_id) in requests {
		if let Some(event) = events.get(&event_id) {
			let market_id_str = market_id.to_string();
			if let Some(market) = event.markets.get(&market_id_str) {
				markets.insert((event_id, market_id), market.clone());
			}
			// 如果找不到 market，不添加到结果中（调用者可以通过检查 key 是否存在来判断）
		}
		// 如果找不到 event，也不添加到结果中
	}

	Ok(markets)
}

/// 将数据库 Events 模型转换为 ApiMQEventCreate
fn convert_event_to_api_mq_event_create(event: common::model::Events) -> ApiMQEventCreate {
	use std::collections::HashMap;

	// 将 markets 从 Json<HashMap<String, EventMarket>> 转换为 HashMap<String, ApiMQEventMarket>
	let mut api_markets = HashMap::new();
	for (market_id, market) in event.markets.0.iter() {
		// 构建 outcome_info: token_id -> outcome_name 的映射
		let mut outcome_info = HashMap::new();
		for (idx, token_id) in market.token_ids.iter().enumerate() {
			if let Some(outcome_name) = market.outcomes.get(idx) {
				outcome_info.insert(token_id.clone(), outcome_name.clone());
			}
		}

		let api_market = common::event_types::ApiMQEventMarket {
			parent_collection_id: market.parent_collection_id.clone(),
			market_id: market.id,
			condition_id: market.condition_id.clone(),
			market_identifier: market.market_identifier.clone(),
			question: market.question.clone(),
			slug: market.slug.clone(),
			title: market.title.clone(),
			image: market.image.clone(),
			outcome_info,
			outcome_names: market.outcomes.clone(),
			outcome_token_ids: market.token_ids.clone(),
		};
		api_markets.insert(market_id.clone(), api_market);
	}

	ApiMQEventCreate {
		event_id: event.id,
		event_identifier: event.event_identifier,
		slug: event.slug,
		title: event.title,
		description: event.description,
		image: event.image,
		end_date: event.end_date,
		topic: event.topic,
		markets: api_markets,
		created_at: event.created_at,
	}
}
