use {
	common::postgres_pool::{create_default_postgres_pool_config, init_postgres_pool},
	sqlx::PgPool,
};

pub static DB_POOL: tokio::sync::OnceCell<PgPool> = tokio::sync::OnceCell::const_new();

/// 初始化数据库连接池
pub async fn init_db_pool() -> anyhow::Result<()> {
	let env = common::common_env::get_common_env();
	let config = create_default_postgres_pool_config();
	let pool = init_postgres_pool(&env.postgres_write_host, env.postgres_write_port, &env.postgres_write_user, &env.postgres_write_password, &env.postgres_write_database, config).await?;
	DB_POOL.set(pool)?;
	Ok(())
}

/// 获取数据库连接池
pub fn get_db_pool() -> anyhow::Result<PgPool> {
	DB_POOL.get().cloned().ok_or_else(|| anyhow::anyhow!("Database pool not initialized"))
}

/// 关闭数据库连接池
pub async fn close_db_pool() {
	if let Some(pool) = DB_POOL.get() {
		pool.close().await;
	}
}
