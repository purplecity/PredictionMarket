use {
	crate::{
		config::{get_config, load_config},
		consumer,
	},
	common::common_env,
	tracing::info,
};

pub async fn init_all() -> anyhow::Result<()> {
	init_load()?;
	init_logging()?;
	init_redis_pool().await?;
	consumer::init_shutdown();
	Ok(())
}

fn init_load() -> anyhow::Result<()> {
	common_env::load_common_env()?;
	load_config(common::consts::STORE_CONFIG_PATH)?;
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
	let env = common_env::get_common_env();
	common::redis_pool::init_engine_output_mq_pool(&env.engine_output_mq_redis_host, env.engine_output_mq_redis_password.clone(), common::consts::REDIS_DB_ENGINE_OUTPUT_MQ).await?;
	common::redis_pool::ping_engine_output_mq().await?;
	info!("Engine Output MQ Redis pool initialized (host: {}, db: {})", env.engine_output_mq_redis_host, common::consts::REDIS_DB_ENGINE_OUTPUT_MQ);
	Ok(())
}
