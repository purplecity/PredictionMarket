use {common::postgres_pool::create_default_postgres_pool_config, sqlx::PgPool, tokio::sync::OnceCell};

static DB_READ_POOL: OnceCell<PgPool> = OnceCell::const_new();

pub async fn init_db_pool() -> anyhow::Result<()> {
	let common_env = common::common_env::get_common_env();
	let pool_config = create_default_postgres_pool_config();

	let read_pool = common::postgres_pool::init_postgres_pool(
		&common_env.postgres_read_host,
		common_env.postgres_read_port,
		&common_env.postgres_read_user,
		&common_env.postgres_read_password,
		&common_env.postgres_read_database,
		pool_config,
	)
	.await?;

	DB_READ_POOL.set(read_pool).map_err(|_| anyhow::anyhow!("DB read pool already initialized"))?;

	Ok(())
}

pub fn get_db_read_pool() -> anyhow::Result<&'static PgPool> {
	DB_READ_POOL.get().ok_or_else(|| anyhow::anyhow!("DB read pool not initialized"))
}
