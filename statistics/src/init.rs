use {
	crate::config::{get_config, load_config},
	common::common_env,
	tokio::sync::{OnceCell, broadcast},
};

/// 全局 shutdown 信号广播器
static G_SHUTDOWN: OnceCell<broadcast::Sender<()>> = OnceCell::const_new();

/// 初始化 shutdown 信号广播器
pub fn init_shutdown_signal() {
	let (shutdown_tx, _) = broadcast::channel(1);
	if G_SHUTDOWN.set(shutdown_tx).is_err() {
		panic!("Failed to initialize shutdown signal");
	}
}

/// 获取 shutdown 信号接收器
pub fn get_shutdown_receiver() -> broadcast::Receiver<()> {
	G_SHUTDOWN.get().expect("Shutdown signal not initialized").subscribe()
}

/// 发送 shutdown 信号
pub fn send_shutdown_signal() {
	if let Some(sender) = G_SHUTDOWN.get() {
		let _ = sender.send(());
	}
}

pub async fn init_all() -> anyhow::Result<()> {
	init_load()?;
	init_logging()?;
	init_postgres_pool().await?;
	init_redis_pool().await?;

	// 初始化 shutdown 信号
	init_shutdown_signal();

	Ok(())
}

fn init_load() -> anyhow::Result<()> {
	common_env::load_common_env()?;
	load_config(common::consts::STATISTICS_CONFIG_PATH)?;
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

async fn init_postgres_pool() -> anyhow::Result<()> {
	crate::db::init_db_pool().await?;
	tracing::info!("PostgreSQL pools initialized");
	Ok(())
}

async fn init_redis_pool() -> anyhow::Result<()> {
	let env = common_env::get_common_env();
	common::redis_pool::init_common_mq_pool(&env.common_mq_redis_host, env.common_mq_redis_password.clone(), common::consts::REDIS_DB_COMMON_MQ).await?;
	common::redis_pool::ping_common_mq().await?;
	tracing::info!("Redis Common MQ pool initialized (host: {}, db: {})", env.common_mq_redis_host, common::consts::REDIS_DB_COMMON_MQ);
	Ok(())
}
