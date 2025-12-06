use {
	crate::config::{get_config, load_config},
	tracing::info,
};

pub async fn init_all() -> anyhow::Result<()> {
	init_load()?;
	init_logging()?;
	init_redis_pools().await?;
	init_output_publisher()?;
	crate::engine::init_manager();
	crate::engine::init_shutdown();
	Ok(())
}

pub fn init_load() -> anyhow::Result<()> {
	common::common_env::load_common_env()?;
	load_config(common::consts::MATCH_ENGINE_CONFIG_PATH)?;
	Ok(())
}

pub fn init_logging() -> anyhow::Result<()> {
	let config = get_config();
	if config.logging.console {
		common::logging::init_console_logging(&config.logging.level)?;
	} else if let Some(file) = config.logging.file.as_ref() {
		common::logging::init_file_logging(&config.logging.level, file, config.logging.rotation_max_files)?;
	}
	Ok(())
}

/// 初始化 Engine Input MQ、Engine Output MQ 和 WebSocket MQ 的连接池
pub async fn init_redis_pools() -> anyhow::Result<()> {
	let env = common::common_env::get_common_env();

	// 初始化 Engine Input MQ 连接池（用于接收订单和市场消息）
	common::redis_pool::init_engine_input_mq_pool(&env.engine_input_mq_redis_host, env.engine_input_mq_redis_password.clone(), common::consts::REDIS_DB_ENGINE_INPUT_MQ).await?;
	common::redis_pool::ping_engine_input_mq().await?;
	info!("Engine Input MQ Redis pool initialized (host: {}, db: {})", env.engine_input_mq_redis_host, common::consts::REDIS_DB_ENGINE_INPUT_MQ);

	// 初始化 Engine Output MQ 连接池（用于发送消息到各个输出流）
	common::redis_pool::init_engine_output_mq_pool(&env.engine_output_mq_redis_host, env.engine_output_mq_redis_password.clone(), common::consts::REDIS_DB_ENGINE_OUTPUT_MQ).await?;
	common::redis_pool::ping_engine_output_mq().await?;
	info!("Engine Output MQ Redis pool initialized (host: {}, db: {})", env.engine_output_mq_redis_host, common::consts::REDIS_DB_ENGINE_OUTPUT_MQ);

	// 初始化 WebSocket MQ 连接池（用于发送价格变化消息到 websocket 服务）
	common::redis_pool::init_websocket_mq_pool(&env.websocket_mq_redis_host, env.websocket_mq_redis_password.clone(), common::consts::REDIS_DB_WEBSOCKET_MQ).await?;
	common::redis_pool::ping_websocket_mq().await?;
	info!("WebSocket MQ Redis pool initialized (host: {}, db: {})", env.websocket_mq_redis_host, common::consts::REDIS_DB_WEBSOCKET_MQ);

	Ok(())
}

pub fn init_output_publisher() -> anyhow::Result<()> {
	let config = get_config();
	crate::output::init_output_publisher(config.engine_output_mq.output_task_count);
	Ok(())
}
