use {
	crate::{
		config::{get_config, load_config},
		storage::ApiKeyStorage,
	},
	common::{
		common_env, consts, logging,
		postgres_pool::{create_default_postgres_pool_config, init_postgres_pool},
		redis_pool,
	},
	std::sync::Arc,
	tokio::sync::RwLock,
	tracing::info,
};

pub async fn init_all() -> anyhow::Result<()> {
	init_load()?;
	init_logging()?;
	init_redis_pool().await?;
	crate::consumer::init_shutdown();
	Ok(())
}

/// 从数据库加载所有 api_keys 到内存，加载完成后关闭连接
pub async fn load_api_keys(api_key_storage: Arc<RwLock<ApiKeyStorage>>) -> anyhow::Result<()> {
	let env = common_env::get_common_env();
	let config = create_default_postgres_pool_config();

	// 创建临时连接池
	let pool = init_postgres_pool(&env.postgres_read_host, env.postgres_read_port, &env.postgres_read_user, &env.postgres_read_password, &env.postgres_read_database, config).await?;

	let rows: Vec<(String, String)> = sqlx::query_as("SELECT api_key, privy_id FROM user_api_keys").fetch_all(&pool).await?;

	let entries: Vec<(String, String)> = rows.into_iter().collect();

	let mut storage = api_key_storage.write().await;
	storage.load_batch(entries);

	info!("Loaded {} api_keys from database", storage.count());

	// 关闭连接池
	pool.close().await;
	info!("PostgreSQL connection closed after loading api_keys");

	Ok(())
}

fn init_load() -> anyhow::Result<()> {
	common_env::load_common_env()?;
	load_config(consts::WEBSOCKET_USER_CONFIG_PATH)?;
	Ok(())
}

fn init_logging() -> anyhow::Result<()> {
	let config = get_config();
	if config.logging.console {
		logging::init_console_logging(&config.logging.level)?;
	} else if let Some(file) = config.logging.file.as_ref() {
		logging::init_file_logging(&config.logging.level, file, config.logging.rotation_max_files)?;
	}
	Ok(())
}

async fn init_redis_pool() -> anyhow::Result<()> {
	let env = common_env::get_common_env();

	// 初始化 websocket_mq (用于消费 USER_EVENT_STREAM)
	redis_pool::init_websocket_mq_pool(&env.websocket_mq_redis_host, env.websocket_mq_redis_password.clone(), consts::REDIS_DB_WEBSOCKET_MQ).await?;
	redis_pool::ping_websocket_mq().await?;
	info!("WebSocket MQ Redis pool initialized");

	// 初始化 common_mq (用于消费 API_KEY_STREAM)
	redis_pool::init_common_mq_pool(&env.common_mq_redis_host, env.common_mq_redis_password.clone(), consts::REDIS_DB_COMMON_MQ).await?;
	redis_pool::ping_common_mq().await?;
	info!("Common MQ Redis pool initialized");

	Ok(())
}
