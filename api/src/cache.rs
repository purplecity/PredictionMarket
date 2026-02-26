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

	#[error("Event not found or closed: {0}")]
	EventNotFoundOrClosed(i64),

	#[error("Event expired: {0}")]
	EventExpired(i64),

	#[error("Market not found or closed: event_id={0}, market_id={1}")]
	MarketNotFoundOrClosed(i64, i16),

	#[error("Cache not initialized")]
	NotInitialized,

	#[error("Database error: {0}")]
	Database(#[from] sqlx::Error),
}

/// 全局缓存：privy_id -> id 的映射
static PRIVY_ID_TO_ID_CACHE: OnceCell<RwLock<HashMap<String, i64>>> = OnceCell::const_new();

/// 全局缓存：event_id -> ApiMQEventCreate 的映射
static EVENT_CACHE: OnceCell<RwLock<HashMap<i64, ApiMQEventCreate>>> = OnceCell::const_new();

/// 全局缓存：api_key -> (user_id, privy_id) 的映射
static API_KEY_TO_USER_INFO_CACHE: OnceCell<RwLock<HashMap<String, (i64, String)>>> = OnceCell::const_new();

/// 初始化缓存
pub fn init_cache() {
	let privy_cache = RwLock::new(HashMap::new());
	PRIVY_ID_TO_ID_CACHE.set(privy_cache).expect("Privy cache already initialized");

	let event_cache = RwLock::new(HashMap::new());
	EVENT_CACHE.set(event_cache).expect("Event cache already initialized");

	let api_key_cache = RwLock::new(HashMap::new());
	API_KEY_TO_USER_INFO_CACHE.set(api_key_cache).expect("API key cache already initialized");
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

/// 根据 api_key 查询用户 id 和 privy_id
/// 首先从缓存中查找，如果找不到则从数据库中查询
pub async fn get_user_id_and_privy_id_by_api_key(api_key: &str) -> Result<(i64, String), CacheError> {
	let cache = API_KEY_TO_USER_INFO_CACHE.get().ok_or(CacheError::NotInitialized)?;

	// 1. 先从缓存中查找
	{
		let read_guard = cache.read().await;
		if let Some((user_id, privy_id)) = read_guard.get(api_key) {
			return Ok((*user_id, privy_id.clone()));
		}
	}

	// 2. 缓存中没有，从数据库中查询
	let read_pool = get_db_read_pool().map_err(|_| CacheError::NotInitialized)?;
	let result: Option<(i64, String)> =
		sqlx::query_as("SELECT user_id, privy_id FROM user_api_keys WHERE api_key = $1").bind(api_key).fetch_optional(&read_pool).await.map_err(CacheError::Database)?;

	match result {
		Some((user_id, privy_id)) => {
			// 3. 查到了，插入缓存并返回
			let mut write_guard = cache.write().await;
			write_guard.insert(api_key.to_string(), (user_id, privy_id.clone()));
			Ok((user_id, privy_id))
		}
		None => {
			// 4. 还查不到，返回没找到错误
			Err(CacheError::UserNotFound(format!("api_key:{}", api_key)))
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
	let event: Option<common::model::Events> = sqlx::query_as("SELECT * FROM events WHERE id = $1").bind(event_id).fetch_optional(&read_pool).await.map_err(CacheError::Database)?;

	match event {
		Some(event_model) => {
			// 3. 查到了，转换为 ApiMQEventCreate 并插入缓存
			let api_event = convert_event_to_api_mq_event_create(event_model);
			let mut write_guard = cache.write().await;
			write_guard.insert(event_id, api_event.clone());
			Ok(api_event)
		}
		None => {
			// 4. 还查不到，返回没找到或已关闭错误
			Err(CacheError::EventNotFoundOrClosed(event_id))
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
pub(crate) async fn remove_event(_event_id: i64) {
	// let cache = EVENT_CACHE.get().expect("Event cache not initialized");
	// let mut write_guard = cache.write().await;
	// write_guard.remove(&event_id);
}

/// 向事件中添加单个 market（内部使用）
pub(crate) async fn add_market_to_event(event_id: i64, market_id: i16, market: common::event_types::ApiMQEventMarket) {
	let cache = EVENT_CACHE.get().expect("Event cache not initialized");
	let mut write_guard = cache.write().await;

	if let Some(event) = write_guard.get_mut(&event_id) {
		let market_id_str = market_id.to_string();
		event.markets.insert(market_id_str, market);
	}
}

/// 关闭事件中的单个 market（内部使用）
/// market关闭时不需要修改缓存，因为缓存只存储不变的信息
/// closed状态从数据库查询获取
pub(crate) async fn remove_market_from_event(_event_id: i64, _market_id: i16) {
	// market关闭时不需要做任何操作，closed状态从数据库查询获取
}

/// 获取指定 event_id 和 market_id 的市场信息（仅从缓存获取）
pub async fn get_market(event_id: i64, market_id: i16) -> Result<common::event_types::ApiMQEventMarket, CacheError> {
	let event = get_event(event_id).await?;
	let market_id_str = market_id.to_string();
	event.markets.get(&market_id_str).cloned().ok_or(CacheError::MarketNotFoundOrClosed(event_id, market_id))
}

/// 获取市场信息并检查是否关闭（用于下单等需要验证状态的场景）
/// 直接查询数据库，检查 event.closed、event.end_date 和 market.closed 状态
pub async fn get_market_and_check_closed(event_id: i64, market_id: i16) -> Result<common::event_types::ApiMQEventMarket, CacheError> {
	let read_pool = get_db_read_pool().map_err(|_| CacheError::NotInitialized)?;
	let market_id_str = market_id.to_string();

	// 查询 event 以及对应的 market（包含 end_date）
	let result: Option<(bool, Option<chrono::DateTime<chrono::Utc>>, Option<serde_json::Value>)> =
		sqlx::query_as("SELECT closed, end_date, markets->$1::text FROM events WHERE id = $2").bind(&market_id_str).bind(event_id).fetch_optional(&read_pool).await.map_err(CacheError::Database)?;

	match result {
		Some((event_closed, end_date, market_json)) => {
			// 检查 event 是否关闭
			if event_closed {
				return Err(CacheError::EventNotFoundOrClosed(event_id));
			}

			// 检查 event 是否过期
			if let Some(end_date) = end_date
				&& chrono::Utc::now() >= end_date
			{
				return Err(CacheError::EventExpired(event_id));
			}

			// 检查 market 是否存在
			let market_json = market_json.ok_or(CacheError::MarketNotFoundOrClosed(event_id, market_id))?;

			// 解析 market 数据
			let market: common::model::EventMarket = serde_json::from_value(market_json).map_err(|e| CacheError::Database(sqlx::Error::Decode(Box::new(e))))?;

			// 检查 market 是否关闭
			if market.closed {
				return Err(CacheError::MarketNotFoundOrClosed(event_id, market_id));
			}

			// 构造 outcome_info
			let mut outcome_info = std::collections::HashMap::new();
			for (idx, token_id) in market.token_ids.iter().enumerate() {
				if let Some(outcome_name) = market.outcomes.get(idx) {
					outcome_info.insert(token_id.clone(), outcome_name.clone());
				}
			}

			// 返回 ApiMQEventMarket
			Ok(common::event_types::ApiMQEventMarket {
				parent_collection_id: market.parent_collection_id,
				market_id: market.id,
				condition_id: market.condition_id,
				market_identifier: market.market_identifier,
				question: market.question,
				slug: market.slug,
				title: market.title,
				image: market.image,
				outcome_info,
				outcome_names: market.outcomes.clone(),
				outcome_token_ids: market.token_ids.clone(),
			})
		}
		None => {
			// Event 不存在
			Err(CacheError::EventNotFoundOrClosed(event_id))
		}
	}
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
		let db_events: Vec<common::model::Events> = sqlx::query_as("SELECT * FROM events WHERE id = ANY($1)").bind(&missing_event_ids).fetch_all(&read_pool).await.map_err(CacheError::Database)?;

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
