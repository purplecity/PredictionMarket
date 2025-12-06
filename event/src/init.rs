use {
	crate::{
		config::{get_config, load_config},
		db::init_db_pool,
	},
	common::{common_env, consts, logging, redis_pool},
	tracing::info,
};

pub async fn init_all() -> anyhow::Result<()> {
	init_load()?;
	init_logging()?;
	init_db_pool().await?;
	init_redis_pool().await?;
	crate::consumer::init_shutdown();
	Ok(())
}

fn init_load() -> anyhow::Result<()> {
	common_env::load_common_env()?;
	load_config(consts::EVENT_CONFIG_PATH)?;
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

	// 初始化 engine_input_mq (用于发送事件到 match_engine)
	redis_pool::init_engine_input_mq_pool(&env.engine_input_mq_redis_host, env.engine_input_mq_redis_password.clone(), consts::REDIS_DB_ENGINE_INPUT_MQ).await?;
	redis_pool::ping_engine_input_mq().await?;
	info!("Engine Input MQ Redis pool initialized (host: {}, db: {})", env.engine_input_mq_redis_host, consts::REDIS_DB_ENGINE_INPUT_MQ);

	// 初始化 common_mq (用于消费事件源和发送到其他服务)
	redis_pool::init_common_mq_pool(&env.common_mq_redis_host, env.common_mq_redis_password.clone(), consts::REDIS_DB_COMMON_MQ).await?;
	redis_pool::ping_common_mq().await?;
	info!("Common MQ Redis pool initialized (host: {}, db: {})", env.common_mq_redis_host, consts::REDIS_DB_COMMON_MQ);

	// 初始化 cache_redis (用于统计模块)
	redis_pool::init_cache_redis_pool(&env.cache_redis_host, env.cache_redis_password.clone(), consts::REDIS_DB_CACHE).await?;
	redis_pool::ping_cache_redis().await?;
	info!("Cache Redis pool initialized (host: {}, db: {})", env.cache_redis_host, consts::REDIS_DB_CACHE);

	Ok(())
}
