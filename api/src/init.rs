use {
	crate::config::{get_config, load_config},
	common::common_env,
};

pub async fn init_all() -> anyhow::Result<()> {
	init_load()?;
	init_logging()?;
	crate::internal_service::init_client()?; // 初始化 HTTP Client
	crate::db::init_db_pool().await?; // 初始化数据库连接池
	init_redis_pool().await?; // 初始化 Redis 连接池
	crate::cache::init_cache(); // 初始化缓存
	crate::consumer::init_shutdown(); // 初始化 consumer shutdown
	crate::load::load_events().await?; // 加载市场信息到缓存
	crate::rpc_client::init_asset_client().await?; // 初始化 asset RPC 客户端
	Ok(())
}

fn init_load() -> anyhow::Result<()> {
	common_env::load_common_env()?;
	load_config(common::consts::API_CONFIG_PATH)?;
	Ok(())
}

fn init_logging() -> anyhow::Result<()> {
	let config = get_config();
	if config.logging.console {
		common::logging::init_console_logging(&config.logging.level)?;
	} else if let Some(file) = config.logging.file.as_ref() {
		common::logging::init_file_logging(&config.logging.level, file, config.logging.rotation_max_files)?;
	}
	Ok(())
}

async fn init_redis_pool() -> anyhow::Result<()> {
	let env = common::common_env::get_common_env();

	// 初始化 cache Redis 连接池（用于查询价格、深度等缓存数据）
	common::redis_pool::init_cache_redis_pool(&env.cache_redis_host, env.cache_redis_password.clone(), common::consts::REDIS_DB_CACHE).await?;
	common::redis_pool::ping_cache_redis().await?;
	tracing::info!("Cache Redis pool initialized (host: {}, db: {})", env.cache_redis_host, common::consts::REDIS_DB_CACHE);

	// 初始化 engine_input_mq Redis 连接池（用于发送订单到 match_engine）
	common::redis_pool::init_engine_input_mq_pool(&env.engine_input_mq_redis_host, env.engine_input_mq_redis_password.clone(), common::consts::REDIS_DB_ENGINE_INPUT_MQ).await?;
	common::redis_pool::ping_engine_input_mq().await?;
	tracing::info!("Engine Input MQ Redis pool initialized (host: {}, db: {})", env.engine_input_mq_redis_host, common::consts::REDIS_DB_ENGINE_INPUT_MQ);

	// 初始化 common_mq Redis 连接池（用于消费市场事件）
	common::redis_pool::init_common_mq_pool(&env.common_mq_redis_host, env.common_mq_redis_password.clone(), common::consts::REDIS_DB_COMMON_MQ).await?;
	common::redis_pool::ping_common_mq().await?;
	tracing::info!("Common MQ Redis pool initialized (host: {}, db: {})", env.common_mq_redis_host, common::consts::REDIS_DB_COMMON_MQ);

	Ok(())
}
