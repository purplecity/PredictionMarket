use {
	common::postgres_pool::{create_default_postgres_pool_config, init_postgres_pool},
	sqlx::PgPool,
	tokio::sync::OnceCell,
};

pub static DB_READ_POOL: OnceCell<PgPool> = OnceCell::const_new();
pub static DB_WRITE_POOL: OnceCell<PgPool> = OnceCell::const_new();

/// 初始化数据库连接池（读和写）
pub async fn init_db_pool() -> anyhow::Result<()> {
	let env = common::common_env::get_common_env();
	let config = create_default_postgres_pool_config();

	// 初始化读连接池（从库）
	let read_pool = init_postgres_pool(&env.postgres_read_host, env.postgres_read_port, &env.postgres_read_user, &env.postgres_read_password, &env.postgres_read_database, config.clone()).await?;
	DB_READ_POOL.set(read_pool)?;

	// 初始化写连接池（主库）
	let write_pool = init_postgres_pool(&env.postgres_write_host, env.postgres_write_port, &env.postgres_write_user, &env.postgres_write_password, &env.postgres_write_database, config).await?;
	DB_WRITE_POOL.set(write_pool)?;

	Ok(())
}

/// 获取读数据库连接池（用于查询操作）
pub fn get_db_read_pool() -> anyhow::Result<PgPool> {
	DB_READ_POOL.get().cloned().ok_or_else(|| anyhow::anyhow!("Database read pool not initialized"))
}

/// 获取写数据库连接池（用于更新和插入操作）
pub fn get_db_write_pool() -> anyhow::Result<PgPool> {
	DB_WRITE_POOL.get().cloned().ok_or_else(|| anyhow::anyhow!("Database write pool not initialized"))
}
