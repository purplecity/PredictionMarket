use {
	crate::{
		config::{get_config, load_config},
		db::init_db_pool,
	},
	common::{common_env, consts, logging, redis_pool},
};

pub async fn init_all() -> anyhow::Result<()> {
	init_load()?;
	init_logging()?;
	init_redis_pool().await?;
	init_db_pool().await?;
	// 初始化 user event channel 并启动消费者 task
	crate::user_event::init_event_channel();
	Ok(())
}

fn init_load() -> anyhow::Result<()> {
	common_env::load_common_env()?;
	load_config(consts::ASSET_CONFIG_PATH)?;
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
	// 初始化 websocket_mq Redis 连接池（用于发送用户事件）
	let env = common_env::get_common_env();
	redis_pool::init_websocket_mq_pool(&env.websocket_mq_redis_host, env.websocket_mq_redis_password.clone(), consts::REDIS_DB_WEBSOCKET_MQ).await?;
	redis_pool::ping_websocket_mq().await?;
	tracing::info!("WebSocket MQ Redis pool initialized (host: {}, db: {})", env.websocket_mq_redis_host, consts::REDIS_DB_WEBSOCKET_MQ);
	Ok(())
}
