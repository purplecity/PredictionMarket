use {
	chrono::{DateTime, Utc},
	common::{
		model::Season,
		postgres_pool::{create_default_postgres_pool_config, init_postgres_pool},
	},
	rust_decimal::Decimal,
	sqlx::{PgPool, Row},
	std::str::FromStr,
	tokio::sync::OnceCell,
	tracing::info,
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

/// 获取写数据库连接池（用于更新操作）
pub fn get_db_write_pool() -> anyhow::Result<PgPool> {
	DB_WRITE_POOL.get().cloned().ok_or_else(|| anyhow::anyhow!("Database write pool not initialized"))
}

/// 获取当前活跃赛季
pub async fn get_active_season() -> anyhow::Result<Option<Season>> {
	let pool = get_db_read_pool()?;

	let season = sqlx::query_as::<_, Season>("SELECT id, name, start_time, end_time, active, created_at FROM seasons WHERE active = true ORDER BY id DESC LIMIT 1").fetch_optional(&pool).await?;

	Ok(season)
}

/// 用户积分元数据 (trading_points_ratio, invite_points_ratio, invitee_boost)
#[derive(Debug, Clone)]
pub struct UserPointMetadata {
	pub user_id: i64,
	pub trading_points_ratio: i32,
	pub invite_points_ratio: i32,
	pub invitee_boost: Decimal,
}

/// 获取用户的积分元数据，如果没有则返回默认值
pub async fn get_user_point_metadata(user_id: i64) -> anyhow::Result<UserPointMetadata> {
	let pool = get_db_read_pool()?;

	let result =
		sqlx::query_as::<_, (i32, i32, Decimal)>("SELECT trading_points_ratio, invite_points_ratio, invitee_boost FROM point_metadata WHERE user_id = $1").bind(user_id).fetch_optional(&pool).await?;

	Ok(match result {
		Some((trading_points_ratio, invite_points_ratio, invitee_boost)) => UserPointMetadata { user_id, trading_points_ratio, invite_points_ratio, invitee_boost },
		None => {
			UserPointMetadata {
				user_id,
				trading_points_ratio: crate::consts::DEFAULT_TRADING_POINTS_RATIO,
				invite_points_ratio: crate::consts::DEFAULT_INVITE_POINTS_RATIO,
				invitee_boost: Decimal::from_str(crate::consts::DEFAULT_INVITEE_BOOST).expect("DEFAULT_INVITEE_BOOST is a valid decimal"),
			}
		}
	})
}

/// 批量获取所有用户的积分元数据
pub async fn get_all_user_point_metadata() -> anyhow::Result<Vec<UserPointMetadata>> {
	let pool = get_db_read_pool()?;

	let rows = sqlx::query("SELECT user_id, trading_points_ratio, invite_points_ratio, invitee_boost FROM point_metadata").fetch_all(&pool).await?;

	Ok(rows
		.iter()
		.map(|row| {
			UserPointMetadata {
				user_id: row.get("user_id"),
				trading_points_ratio: row.get("trading_points_ratio"),
				invite_points_ratio: row.get("invite_points_ratio"),
				invitee_boost: row.get("invitee_boost"),
			}
		})
		.collect())
}

/// 用户邀请关系
#[derive(Debug, Clone)]
pub struct UserInviterInfo {
	pub user_id: i64,
	pub inviter_id: Option<i64>,
}

/// 获取所有用户的邀请关系
pub async fn get_all_user_inviter_info() -> anyhow::Result<Vec<UserInviterInfo>> {
	let pool = get_db_read_pool()?;

	let rows = sqlx::query("SELECT user_id, inviter_id FROM user_invitations").fetch_all(&pool).await?;

	Ok(rows.iter().map(|row| UserInviterInfo { user_id: row.get("user_id"), inviter_id: row.get("inviter_id") }).collect())
}

/// 符合流动性积分条件的订单信息
#[derive(Debug, Clone)]
pub struct LiquidityOrderInfo {
	pub user_id: i64,
	pub price: Decimal,
	pub volume: Decimal,
}

/// 查询符合流动性积分条件的limit订单
/// 条件:
/// 1. order_type = 'limit'
/// 2. status = 'filled' OR status = 'cancelled' OR 当前时间 >= 赛季结束时间
/// 3. (updated_at - created_at) >= MIN_ORDER_DURATION_SECS
/// 4. volume >= MIN_ORDER_USDC_AMOUNT
/// 5. created_at >= season_start_time AND created_at < season_end_time
pub async fn get_liquidity_orders(season_start: DateTime<Utc>, season_end: DateTime<Utc>, min_duration_secs: i64, min_volume: Decimal) -> anyhow::Result<Vec<LiquidityOrderInfo>> {
	let pool = get_db_read_pool()?;
	let now = Utc::now();

	// 如果当前时间 >= 赛季结束时间，则包含所有状态的订单
	// 否则只包含 filled 或 cancelled 状态的订单
	let include_all_statuses = now >= season_end;

	let rows = if include_all_statuses {
		sqlx::query(
			r#"
            SELECT user_id, price, volume
            FROM orders
            WHERE order_type = 'limit'
              AND EXTRACT(EPOCH FROM (updated_at - created_at)) >= $1
              AND volume >= $2
              AND created_at >= $3
              AND created_at < $4
            "#,
		)
		.bind(min_duration_secs as f64)
		.bind(min_volume)
		.bind(season_start)
		.bind(season_end)
		.fetch_all(&pool)
		.await?
	} else {
		sqlx::query(
			r#"
            SELECT user_id, price, volume
            FROM orders
            WHERE order_type = 'limit'
              AND (status = 'filled' OR status = 'cancelled')
              AND EXTRACT(EPOCH FROM (updated_at - created_at)) >= $1
              AND volume >= $2
              AND created_at >= $3
              AND created_at < $4
            "#,
		)
		.bind(min_duration_secs as f64)
		.bind(min_volume)
		.bind(season_start)
		.bind(season_end)
		.fetch_all(&pool)
		.await?
	};

	Ok(rows.iter().map(|row| LiquidityOrderInfo { user_id: row.get("user_id"), price: row.get("price"), volume: row.get("volume") }).collect())
}

/// 用户交易量信息
#[derive(Debug, Clone)]
pub struct UserTradingVolume {
	pub user_id: i64,
	pub total_volume: Decimal,
}

/// 查询赛季内用户的总交易量
pub async fn get_user_trading_volumes(season_start: DateTime<Utc>, season_end: DateTime<Utc>) -> anyhow::Result<Vec<UserTradingVolume>> {
	let pool = get_db_read_pool()?;

	let rows = sqlx::query(
		r#"
        SELECT user_id, SUM(usdc_amount) as total_volume
        FROM trades
        WHERE match_timestamp >= $1
          AND match_timestamp < $2
        GROUP BY user_id
        "#,
	)
	.bind(season_start)
	.bind(season_end)
	.fetch_all(&pool)
	.await?;

	Ok(rows.iter().map(|row| UserTradingVolume { user_id: row.get("user_id"), total_volume: row.get("total_volume") }).collect())
}

/// 更新用户积分
#[derive(Debug, Clone)]
pub struct UserPointsUpdate {
	pub user_id: i64,
	pub liquidity_points: i64,
	pub trading_points: i64,
	pub self_points: i64,
	pub boost_points: i64,
	pub invite_earned_points: i64,
	pub total_points: i64,
	pub accumulated_volume: Decimal,
}

/// 批量更新用户积分
pub async fn batch_update_user_points(updates: &[UserPointsUpdate]) -> anyhow::Result<()> {
	if updates.is_empty() {
		return Ok(());
	}

	let pool = get_db_write_pool()?;

	for update in updates {
		sqlx::query(
			r#"
            UPDATE points
            SET liquidity_points = $2,
                trading_points = $3,
                self_points = $4,
                boost_points = $5,
                invite_earned_points = $6,
                total_points = $7,
                accumulated_volume = $8,
                updated_at = NOW()
            WHERE user_id = $1
            "#,
		)
		.bind(update.user_id)
		.bind(update.liquidity_points)
		.bind(update.trading_points)
		.bind(update.self_points)
		.bind(update.boost_points)
		.bind(update.invite_earned_points)
		.bind(update.total_points)
		.bind(update.accumulated_volume)
		.execute(&pool)
		.await?;
	}

	info!("Updated points for {} users", updates.len());
	Ok(())
}

/// 用户积分历史记录
#[derive(Debug, Clone)]
pub struct UserSeasonHistory {
	pub user_id: i64,
	pub total_points: i64,
	pub accumulated_volume: Decimal,
}

/// 获取所有用户的积分数据（用于赛季结束时迁移）
pub async fn get_all_user_points() -> anyhow::Result<Vec<UserSeasonHistory>> {
	let pool = get_db_read_pool()?;

	let rows = sqlx::query("SELECT user_id, total_points, accumulated_volume FROM points").fetch_all(&pool).await?;

	Ok(rows.iter().map(|row| UserSeasonHistory { user_id: row.get("user_id"), total_points: row.get("total_points"), accumulated_volume: row.get("accumulated_volume") }).collect())
}

/// 将用户积分写入赛季历史记录表（单个用户）
pub async fn insert_season_history(season_id: i32, record: &UserSeasonHistory) -> anyhow::Result<()> {
	let pool = get_db_write_pool()?;

	sqlx::query(
		r#"
		INSERT INTO season_history (season_id, user_id, total_points, accumulated_volume)
		VALUES ($1, $2, $3, $4)
		ON CONFLICT (season_id, user_id) DO UPDATE SET
			total_points = EXCLUDED.total_points,
			accumulated_volume = EXCLUDED.accumulated_volume
		"#,
	)
	.bind(season_id)
	.bind(record.user_id)
	.bind(record.total_points)
	.bind(record.accumulated_volume)
	.execute(&pool)
	.await?;

	Ok(())
}

/// 将赛季设为非活跃
pub async fn deactivate_season(season_id: i32) -> anyhow::Result<()> {
	let pool = get_db_write_pool()?;

	sqlx::query("UPDATE seasons SET active = false WHERE id = $1").bind(season_id).execute(&pool).await?;

	info!("Deactivated season {}", season_id);
	Ok(())
}
