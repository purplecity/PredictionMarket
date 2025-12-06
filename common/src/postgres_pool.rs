use {
	anyhow,
	serde::{Deserialize, Serialize},
	sqlx::{PgPool, postgres::PgPoolOptions},
	std::time::Duration,
	tracing::info,
};

/// PostgreSQL 连接池配置
#[derive(Debug, Clone)]
pub struct PostgresPoolConfig {
	pub max_connections: u32,
	pub min_connections: u32,
	pub idle_timeout: Duration,
	pub max_lifetime: Duration,
	pub acquire_timeout: Duration,
	pub test_before_acquire: bool,
}

impl Default for PostgresPoolConfig {
	fn default() -> Self {
		Self {
			max_connections: 10,
			min_connections: 2,
			idle_timeout: Duration::from_secs(600),
			max_lifetime: Duration::from_secs(1800),
			acquire_timeout: Duration::from_secs(30),
			test_before_acquire: false,
		}
	}
}

/// 初始化 PostgreSQL 连接池
pub async fn init_postgres_pool(host: &str, port: u16, user: &str, password: &str, database: &str, config: PostgresPoolConfig) -> anyhow::Result<PgPool> {
	let database_url = format!("postgres://{}:{}@{}:{}/{}", user, password, host, port, database);

	let pool_options = PgPoolOptions::new()
		.max_connections(config.max_connections)
		.min_connections(config.min_connections)
		.acquire_timeout(config.acquire_timeout)
		.test_before_acquire(config.test_before_acquire)
		.idle_timeout(config.idle_timeout)
		.max_lifetime(config.max_lifetime);

	let pool = pool_options.connect(&database_url).await?;

	// 测试连接
	sqlx::query("SELECT 1").execute(&pool).await?;

	info!("PostgreSQL pool initialized: {}:{}/{} (max: {}, min: {})", host, port, database, config.max_connections, config.min_connections);
	Ok(pool)
}

/// 从配置参数创建 PostgreSQL 连接池配置
pub fn create_postgres_pool_config(
	max_connections: u32,
	min_connections: u32,
	idle_timeout_secs: u64,
	max_lifetime_secs: u64,
	acquire_timeout_secs: u64,
	test_before_acquire: bool,
) -> PostgresPoolConfig {
	PostgresPoolConfig {
		max_connections,
		min_connections,
		idle_timeout: Duration::from_secs(idle_timeout_secs),
		max_lifetime: Duration::from_secs(max_lifetime_secs),
		acquire_timeout: Duration::from_secs(acquire_timeout_secs),
		test_before_acquire,
	}
}

/// 使用全局常量创建 PostgreSQL 连接池配置
pub fn create_default_postgres_pool_config() -> PostgresPoolConfig {
	use crate::consts::*;
	PostgresPoolConfig {
		max_connections: POSTGRES_MAX_CONNECTIONS,
		min_connections: POSTGRES_MIN_CONNECTIONS,
		idle_timeout: Duration::from_secs(POSTGRES_IDLE_TIMEOUT_SECS),
		max_lifetime: Duration::from_secs(POSTGRES_MAX_LIFETIME_SECS),
		acquire_timeout: Duration::from_secs(POSTGRES_ACQUIRE_TIMEOUT_SECS),
		test_before_acquire: POSTGRES_TEST_BEFORE_ACQUIRE,
	}
}

/// PostgreSQL 配置文件结构体（用于从 TOML 配置文件反序列化）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PostgresConfig {
	pub max_connections: u32,
	pub min_connections: u32,
	pub idle_timeout_secs: u64,
	pub max_lifetime_secs: u64,
	#[serde(default)]
	pub acquire_timeout_secs: u64,
	#[serde(default)]
	pub test_before_acquire: bool,
}

impl Default for PostgresConfig {
	fn default() -> Self {
		Self {
			max_connections: 0,   // 必须从配置文件提供
			min_connections: 0,   // 必须从配置文件提供
			idle_timeout_secs: 0, // 必须从配置文件提供
			max_lifetime_secs: 0, // 必须从配置文件提供
			acquire_timeout_secs: 30,
			test_before_acquire: false,
		}
	}
}

impl PostgresConfig {
	/// 检查配置是否有效
	pub fn check(&self) -> anyhow::Result<()> {
		if self.max_connections == 0 {
			return Err(anyhow::anyhow!("Postgres max_connections must be greater than 0"));
		}
		if self.min_connections == 0 {
			return Err(anyhow::anyhow!("Postgres min_connections must be greater than 0"));
		}
		if self.idle_timeout_secs == 0 {
			return Err(anyhow::anyhow!("Postgres idle_timeout_secs must be greater than 0"));
		}
		if self.max_lifetime_secs == 0 {
			return Err(anyhow::anyhow!("Postgres max_lifetime_secs must be greater than 0"));
		}
		if self.acquire_timeout_secs == 0 {
			return Err(anyhow::anyhow!("Postgres acquire_timeout_secs must be greater than 0"));
		}
		if self.max_connections <= self.min_connections {
			return Err(anyhow::anyhow!("Postgres max_connections must be greater than min_connections"));
		}
		Ok(())
	}
}
