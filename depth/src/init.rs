use {
	crate::config::{get_config, load_config},
	common::{common_env, consts, logging, redis_pool},
	tracing::info,
};

pub async fn init_all() -> anyhow::Result<()> {
	init_load()?;
	init_logging()?;
	init_redis_pool().await?;
	crate::consumer::init_shutdown();
	Ok(())
}

fn init_load() -> anyhow::Result<()> {
	common_env::load_common_env()?;
	load_config(consts::DEPTH_CONFIG_PATH)?;
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

	// 初始化 websocket_mq (用于消费 match_engine 的 depth_stream 和推送深度到 websocket 服务)
	redis_pool::init_websocket_mq_pool(&env.websocket_mq_redis_host, env.websocket_mq_redis_password.clone(), consts::REDIS_DB_WEBSOCKET_MQ).await?;
	redis_pool::ping_websocket_mq().await?;
	info!("WebSocket MQ Redis pool initialized (host: {}, db: {})", env.websocket_mq_redis_host, consts::REDIS_DB_WEBSOCKET_MQ);

	// 初始化 cache_redis (用于缓存深度数据)
	redis_pool::init_cache_redis_pool(&env.cache_redis_host, env.cache_redis_password.clone(), consts::REDIS_DB_CACHE).await?;
	redis_pool::ping_cache_redis().await?;
	info!("Cache Redis pool initialized (host: {}, db: {})", env.cache_redis_host, consts::REDIS_DB_CACHE);

	Ok(())
}
