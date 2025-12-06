use {
	crate::consts::COMMON_ENV_PATH,
	config::{Config, Environment},
	serde::{Deserialize, Serialize},
	tokio::sync::OnceCell,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommonEnv {
	pub internal_service_host: String,
	pub run_mode: String,
	// Engine Input MQ Redis 配置（用于 engine 接收订单和市场消息）
	pub engine_input_mq_redis_host: String,
	pub engine_input_mq_redis_password: Option<String>,

	// Engine Output MQ Redis 配置（用于 engine 发送消息到各个输出流）
	pub engine_output_mq_redis_host: String,
	pub engine_output_mq_redis_password: Option<String>,

	// WebSocket MQ Redis 配置
	pub websocket_mq_redis_host: String,
	pub websocket_mq_redis_password: Option<String>,

	// Common MQ Redis 配置
	pub common_mq_redis_host: String,
	pub common_mq_redis_password: Option<String>,

	// Cache Redis 配置
	pub cache_redis_host: String,
	pub cache_redis_password: Option<String>,

	// Lock Redis 配置
	pub lock_redis_host: String,
	pub lock_redis_password: Option<String>,

	// PostgreSQL 读配置（从库）
	pub postgres_read_host: String,
	pub postgres_read_port: u16,
	pub postgres_read_user: String,
	pub postgres_read_password: String,
	pub postgres_read_database: String,

	// PostgreSQL 写配置（主库）
	pub postgres_write_host: String,
	pub postgres_write_port: u16,
	pub postgres_write_user: String,
	pub postgres_write_password: String,
	pub postgres_write_database: String,

	// Asset RPC 服务 URL
	pub asset_rpc_url: String,

	// privy设置
	pub privy_pem_key: String,
	pub privy_secret_key: String,
	pub privy_app_id: String,
}

pub static COMMON_ENV: OnceCell<CommonEnv> = OnceCell::const_new();

pub fn load_common_env() -> anyhow::Result<()> {
	// 使用 dotenvy 从文件加载环境变量到进程环境变量中
	dotenvy::from_path(COMMON_ENV_PATH)?;

	// 使用 config crate 从环境变量反序列化到 CommonEnv
	let config = Config::builder().add_source(Environment::default()).build()?;

	let common_env: CommonEnv = config.try_deserialize()?;
	println!("Common env configuration: {:?}", common_env);
	COMMON_ENV.set(common_env)?;
	check_common_env()?;
	Ok(())
}

pub fn check_common_env() -> anyhow::Result<()> {
	let common_env = get_common_env();

	// 验证 RUN_MODE
	crate::consts::validate_run_mode(&common_env.run_mode)?;

	if common_env.engine_input_mq_redis_host.is_empty() {
		return Err(anyhow::anyhow!("Engine Input MQ Redis host is empty"));
	}
	if common_env.engine_output_mq_redis_host.is_empty() {
		return Err(anyhow::anyhow!("Engine Output MQ Redis host is empty"));
	}
	if common_env.websocket_mq_redis_host.is_empty() {
		return Err(anyhow::anyhow!("WebSocket MQ Redis host is empty"));
	}
	if common_env.common_mq_redis_host.is_empty() {
		return Err(anyhow::anyhow!("Common MQ Redis host is empty"));
	}
	if common_env.cache_redis_host.is_empty() {
		return Err(anyhow::anyhow!("Cache Redis host is empty"));
	}
	if common_env.lock_redis_host.is_empty() {
		return Err(anyhow::anyhow!("Lock Redis host is empty"));
	}
	if common_env.postgres_read_host.is_empty() {
		return Err(anyhow::anyhow!("PostgreSQL read host is empty"));
	}
	if common_env.postgres_read_user.is_empty() {
		return Err(anyhow::anyhow!("PostgreSQL read user is empty"));
	}
	if common_env.postgres_read_database.is_empty() {
		return Err(anyhow::anyhow!("PostgreSQL read database is empty"));
	}
	if common_env.postgres_write_host.is_empty() {
		return Err(anyhow::anyhow!("PostgreSQL write host is empty"));
	}
	if common_env.postgres_write_user.is_empty() {
		return Err(anyhow::anyhow!("PostgreSQL write user is empty"));
	}
	if common_env.postgres_write_database.is_empty() {
		return Err(anyhow::anyhow!("PostgreSQL write database is empty"));
	}

	if common_env.asset_rpc_url.is_empty() {
		return Err(anyhow::anyhow!("Asset RPC URL is empty"));
	}

	if common_env.internal_service_host.is_empty() {
		return Err(anyhow::anyhow!("Internal service host is empty"));
	}

	if common_env.privy_pem_key.is_empty() {
		return Err(anyhow::anyhow!("Privy PEM key is empty"));
	}
	if common_env.privy_secret_key.is_empty() {
		return Err(anyhow::anyhow!("Privy secret key is empty"));
	}
	if common_env.privy_app_id.is_empty() {
		return Err(anyhow::anyhow!("Privy app id is empty"));
	}

	Ok(())
}

pub fn get_common_env() -> &'static CommonEnv {
	COMMON_ENV.get().expect("Common env not loaded")
}
