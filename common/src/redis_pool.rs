use {
	crate::consts::REDIS_POOL_MAX_SIZE,
	anyhow,
	deadpool_redis::{Config, Connection, Pool, PoolConfig, Runtime},
	redis::AsyncCommands,
	tokio::sync::OnceCell,
};

// Engine Input MQ 连接池（用于 engine 接收订单和市场消息）
pub static ENGINE_INPUT_MQ_POOL: OnceCell<Pool> = OnceCell::const_new();

// Engine Output MQ 连接池（用于 engine 发送消息到各个输出流）
pub static ENGINE_OUTPUT_MQ_POOL: OnceCell<Pool> = OnceCell::const_new();

// WebSocket MQ 连接池
pub static WEBSOCKET_MQ_POOL: OnceCell<Pool> = OnceCell::const_new();

// Common MQ 连接池
pub static COMMON_MQ_POOL: OnceCell<Pool> = OnceCell::const_new();

// Cache Redis 连接池
pub static CACHE_REDIS_POOL: OnceCell<Pool> = OnceCell::const_new();

// Lock Redis 连接池
pub static LOCK_REDIS_POOL: OnceCell<Pool> = OnceCell::const_new();

/// 初始化 Engine Input MQ 连接池
pub async fn init_engine_input_mq_pool(redis_host: &str, redis_password: Option<String>, db: u32) -> anyhow::Result<()> {
	let redis_url = format!("redis://{}{}/{}", if let Some(pwd) = redis_password { format!(":{}@", pwd) } else { "".to_string() }, redis_host, db);
	let mut cfg = Config::from_url(redis_url);
	let pool_config = PoolConfig::new(REDIS_POOL_MAX_SIZE);
	cfg.pool = Some(pool_config);
	let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
	ENGINE_INPUT_MQ_POOL.set(pool)?;
	Ok(())
}

/// 初始化 Engine Output MQ 连接池
pub async fn init_engine_output_mq_pool(redis_host: &str, redis_password: Option<String>, db: u32) -> anyhow::Result<()> {
	let redis_url = format!("redis://{}{}/{}", if let Some(pwd) = redis_password { format!(":{}@", pwd) } else { "".to_string() }, redis_host, db);
	let mut cfg = Config::from_url(redis_url);
	let pool_config = PoolConfig::new(REDIS_POOL_MAX_SIZE);
	cfg.pool = Some(pool_config);
	let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
	ENGINE_OUTPUT_MQ_POOL.set(pool)?;
	Ok(())
}

/// 获取 Engine Input MQ 连接池
pub fn get_engine_input_mq_pool() -> anyhow::Result<Pool> {
	ENGINE_INPUT_MQ_POOL.get().cloned().ok_or_else(|| anyhow::anyhow!("Engine Input MQ Redis pool not initialized"))
}

/// 获取 Engine Output MQ 连接池
pub fn get_engine_output_mq_pool() -> anyhow::Result<Pool> {
	ENGINE_OUTPUT_MQ_POOL.get().cloned().ok_or_else(|| anyhow::anyhow!("Engine Output MQ Redis pool not initialized"))
}

/// 获取 Engine Input MQ 连接
pub async fn get_engine_input_mq_connection() -> anyhow::Result<Connection> {
	let pool = get_engine_input_mq_pool()?;
	pool.get().await.map_err(|e| anyhow::anyhow!("Failed to get engine input MQ redis connection: {}", e))
}

/// 获取 Engine Output MQ 连接
pub async fn get_engine_output_mq_connection() -> anyhow::Result<Connection> {
	let pool = get_engine_output_mq_pool()?;
	pool.get().await.map_err(|e| anyhow::anyhow!("Failed to get engine output MQ redis connection: {}", e))
}

/// Ping Engine Input MQ
pub async fn ping_engine_input_mq() -> anyhow::Result<()> {
	get_engine_input_mq_connection().await?.ping().await.map_err(|e| anyhow::anyhow!("Engine Input MQ Redis ping error: {}", e))
}

/// Ping Engine Output MQ
pub async fn ping_engine_output_mq() -> anyhow::Result<()> {
	get_engine_output_mq_connection().await?.ping().await.map_err(|e| anyhow::anyhow!("Engine Output MQ Redis ping error: {}", e))
}

/// 初始化 WebSocket MQ 连接池
pub async fn init_websocket_mq_pool(redis_host: &str, redis_password: Option<String>, db: u32) -> anyhow::Result<()> {
	let redis_url = format!("redis://{}{}/{}", if let Some(pwd) = redis_password { format!(":{}@", pwd) } else { "".to_string() }, redis_host, db);
	let mut cfg = Config::from_url(redis_url);
	let pool_config = PoolConfig::new(REDIS_POOL_MAX_SIZE);
	cfg.pool = Some(pool_config);
	let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
	WEBSOCKET_MQ_POOL.set(pool)?;
	Ok(())
}

/// 初始化 Common MQ 连接池
pub async fn init_common_mq_pool(redis_host: &str, redis_password: Option<String>, db: u32) -> anyhow::Result<()> {
	let redis_url = format!("redis://{}{}/{}", if let Some(pwd) = redis_password { format!(":{}@", pwd) } else { "".to_string() }, redis_host, db);
	let mut cfg = Config::from_url(redis_url);
	let pool_config = PoolConfig::new(REDIS_POOL_MAX_SIZE);
	cfg.pool = Some(pool_config);
	let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
	COMMON_MQ_POOL.set(pool)?;
	Ok(())
}

/// 初始化 Cache Redis 连接池
pub async fn init_cache_redis_pool(redis_host: &str, redis_password: Option<String>, db: u32) -> anyhow::Result<()> {
	let redis_url = format!("redis://{}{}/{}", if let Some(pwd) = redis_password { format!(":{}@", pwd) } else { "".to_string() }, redis_host, db);
	let mut cfg = Config::from_url(redis_url);
	let pool_config = PoolConfig::new(REDIS_POOL_MAX_SIZE);
	cfg.pool = Some(pool_config);
	let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
	CACHE_REDIS_POOL.set(pool)?;
	Ok(())
}

/// 初始化 Lock Redis 连接池
pub async fn init_lock_redis_pool(redis_host: &str, redis_password: Option<String>, db: u32) -> anyhow::Result<()> {
	let redis_url = format!("redis://{}{}/{}", if let Some(pwd) = redis_password { format!(":{}@", pwd) } else { "".to_string() }, redis_host, db);
	let mut cfg = Config::from_url(redis_url);
	let pool_config = PoolConfig::new(REDIS_POOL_MAX_SIZE);
	cfg.pool = Some(pool_config);
	let pool = cfg.create_pool(Some(Runtime::Tokio1))?;
	LOCK_REDIS_POOL.set(pool)?;
	Ok(())
}

/// 获取 WebSocket MQ 连接池
pub fn get_websocket_mq_pool() -> anyhow::Result<Pool> {
	WEBSOCKET_MQ_POOL.get().cloned().ok_or_else(|| anyhow::anyhow!("WebSocket MQ Redis pool not initialized"))
}

/// 获取 Common MQ 连接池
pub fn get_common_mq_pool() -> anyhow::Result<Pool> {
	COMMON_MQ_POOL.get().cloned().ok_or_else(|| anyhow::anyhow!("Common MQ Redis pool not initialized"))
}

/// 获取 Cache Redis 连接池
pub fn get_cache_redis_pool() -> anyhow::Result<Pool> {
	CACHE_REDIS_POOL.get().cloned().ok_or_else(|| anyhow::anyhow!("Cache Redis pool not initialized"))
}

/// 关闭 Cache Redis 连接池 (释放所有连接)
pub fn close_cache_redis_pool() {
	if let Some(pool) = CACHE_REDIS_POOL.get() {
		pool.close();
	}
}

/// 获取 Lock Redis 连接池
pub fn get_lock_redis_pool() -> anyhow::Result<Pool> {
	LOCK_REDIS_POOL.get().cloned().ok_or_else(|| anyhow::anyhow!("Lock Redis pool not initialized"))
}

/// 获取 WebSocket MQ 连接
pub async fn get_websocket_mq_connection() -> anyhow::Result<Connection> {
	let pool = get_websocket_mq_pool()?;
	pool.get().await.map_err(|e| anyhow::anyhow!("Failed to get websocket MQ redis connection: {}", e))
}

/// 获取 Common MQ 连接
pub async fn get_common_mq_connection() -> anyhow::Result<Connection> {
	let pool = get_common_mq_pool()?;
	pool.get().await.map_err(|e| anyhow::anyhow!("Failed to get common MQ redis connection: {}", e))
}

/// 获取 Cache Redis 连接
pub async fn get_cache_redis_connection() -> anyhow::Result<Connection> {
	let pool = get_cache_redis_pool()?;
	pool.get().await.map_err(|e| anyhow::anyhow!("Failed to get cache redis connection: {}", e))
}

/// 获取 Lock Redis 连接
pub async fn get_lock_redis_connection() -> anyhow::Result<Connection> {
	let pool = get_lock_redis_pool()?;
	pool.get().await.map_err(|e| anyhow::anyhow!("Failed to get lock redis connection: {}", e))
}

/// Ping WebSocket MQ
pub async fn ping_websocket_mq() -> anyhow::Result<()> {
	get_websocket_mq_connection().await?.ping().await.map_err(|e| anyhow::anyhow!("WebSocket MQ Redis ping error: {}", e))
}

/// Ping Common MQ
pub async fn ping_common_mq() -> anyhow::Result<()> {
	get_common_mq_connection().await?.ping().await.map_err(|e| anyhow::anyhow!("Common MQ Redis ping error: {}", e))
}

/// Ping Cache Redis
pub async fn ping_cache_redis() -> anyhow::Result<()> {
	get_cache_redis_connection().await?.ping().await.map_err(|e| anyhow::anyhow!("Cache Redis ping error: {}", e))
}

/// Ping Lock Redis
pub async fn ping_lock_redis() -> anyhow::Result<()> {
	get_lock_redis_connection().await?.ping().await.map_err(|e| anyhow::anyhow!("Lock Redis ping error: {}", e))
}
