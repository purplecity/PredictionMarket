use {
	rust_decimal::Decimal,
	sqlx::PgPool,
	std::{env, str::FromStr},
};

/// 测试环境配置
pub struct TestEnv {
	pub pool: PgPool,
}

#[allow(dead_code)]
impl TestEnv {
	/// 设置测试环境（初始化数据库连接池）
	pub async fn setup() -> anyhow::Result<Self> {
		// 加载 .env 文件
		let env_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("deploy/common.env");
		dotenv::from_path(&env_path).ok();

		// 从环境变量读取数据库配置
		let host = env::var("PREDICTION_EVENT_postgres_write_host").unwrap_or_else(|_| "127.0.0.1".to_string());
		let port = env::var("PREDICTION_EVENT_postgres_write_port").unwrap_or_else(|_| "5432".to_string()).parse::<u16>().unwrap();
		let user = env::var("PREDICTION_EVENT_postgres_write_user").unwrap_or_else(|_| "postgres".to_string());
		let password = env::var("PREDICTION_EVENT_postgres_write_password").unwrap_or_else(|_| "123456".to_string());
		let database = env::var("PREDICTION_EVENT_postgres_write_database").unwrap_or_else(|_| "prediction_market".to_string());

		// 创建数据库连接池（小连接数，用于测试隔离）
		let database_url = format!("postgres://{}:{}@{}:{}/{}", user, password, host, port, database);
		let pool = sqlx::postgres::PgPoolOptions::new()
			.max_connections(3) // 小连接池，避免资源耗尽
			.min_connections(1)
			.acquire_timeout(std::time::Duration::from_secs(5))
			.connect(&database_url)
			.await?;

		// 不设置全局 DB_POOL - 每个测试使用独立的 pool

		Ok(Self { pool })
	}

	/// 清理测试环境（关闭数据库连接池）
	pub async fn teardown(self) {
		self.pool.close().await;
	}

	/// 创建测试用户
	pub async fn create_test_user(&self, user_id: i64, privy_id: &str) -> anyhow::Result<()> {
		sqlx::query(
			r#"
			INSERT INTO users (id, privy_id, privy_evm_address, created_at)
			VALUES ($1, $2, $3, NOW())
			ON CONFLICT (id) DO NOTHING
			"#,
		)
		.bind(user_id)
		.bind(privy_id)
		.bind(format!("0x{:040x}", user_id))
		.execute(&self.pool)
		.await?;
		Ok(())
	}

	/// 创建测试事件和市场
	pub async fn create_test_event_and_market(&self, event_id: i64, market_id: i16) -> anyhow::Result<(String, String)> {
		// 生成 token_id
		let token_0_id = format!("{}|{}|0", event_id, market_id);
		let token_1_id = format!("{}|{}|1", event_id, market_id);

		// 构造 market JSON 数据
		let market_json = serde_json::json!({
			market_id.to_string(): {
				"id": market_id,
				"condition_id": format!("test_condition_{}", market_id),
				"parent_collection_id": format!("test_parent_{}", event_id),
				"market_identifier": format!("test_market_{}_{}", event_id, market_id),
				"slug": format!("test-market-{}-{}", event_id, market_id),
				"title": format!("Test Market {}", market_id),
				"description": "Test market description",
				"image": "",
				"outcomes": ["Yes", "No"],
				"token_ids": [&token_0_id, &token_1_id],
				"win_outcome_name": "",
				"win_outcome_token_id": ""
			}
		});

		// 创建事件（包含 markets JSONB）
		sqlx::query(
			r#"
			INSERT INTO events (
				id, event_identifier, slug, title, description,
				image, end_date, closed, resolved, topic,
				volume, markets, recommended, created_at
			)
			VALUES ($1, $2, $3, $4, $5, $6, NOW() + INTERVAL '1 day', FALSE, FALSE, $7, 0, $8, FALSE, NOW())
			ON CONFLICT (id) DO NOTHING
			"#,
		)
		.bind(event_id)
		.bind(format!("test_event_{}", event_id))
		.bind(format!("test-event-{}", event_id))
		.bind(format!("Test Event {}", event_id))
		.bind("Test event description")
		.bind("")
		.bind("Test")
		.bind(market_json)
		.execute(&self.pool)
		.await?;

		Ok((token_0_id, token_1_id))
	}

	/// 获取用户持仓总余额（balance + frozen_balance）
	pub async fn get_user_balance(&self, user_id: i64, token_id: &str) -> anyhow::Result<Decimal> {
		let balance: Option<Decimal> = sqlx::query_scalar(
			r#"
			SELECT balance + frozen_balance
			FROM positions
			WHERE user_id = $1 AND token_id = $2
			"#,
		)
		.bind(user_id)
		.bind(token_id)
		.fetch_optional(&self.pool)
		.await?;

		Ok(balance.unwrap_or(Decimal::ZERO))
	}

	/// 获取用户可用余额（balance）
	pub async fn get_user_available_balance(&self, user_id: i64, token_id: &str) -> anyhow::Result<Decimal> {
		let balance: Option<Decimal> = sqlx::query_scalar(
			r#"
			SELECT balance
			FROM positions
			WHERE user_id = $1 AND token_id = $2
			"#,
		)
		.bind(user_id)
		.bind(token_id)
		.fetch_optional(&self.pool)
		.await?;

		Ok(balance.unwrap_or(Decimal::ZERO))
	}

	/// 获取用户冻结余额（frozen_balance）
	pub async fn get_user_frozen_balance(&self, user_id: i64, token_id: &str) -> anyhow::Result<Decimal> {
		let frozen: Option<Decimal> = sqlx::query_scalar(
			r#"
			SELECT frozen_balance
			FROM positions
			WHERE user_id = $1 AND token_id = $2
			"#,
		)
		.bind(user_id)
		.bind(token_id)
		.fetch_optional(&self.pool)
		.await?;

		Ok(frozen.unwrap_or(Decimal::ZERO))
	}

	/// 清理测试用户数据
	pub async fn cleanup_test_user(&self, user_id: i64) -> anyhow::Result<()> {
		let mut tx = self.pool.begin().await?;

		// 删除订单
		sqlx::query("DELETE FROM orders WHERE user_id = $1").bind(user_id).execute(tx.as_mut()).await?;

		// 删除交易
		sqlx::query("DELETE FROM trades WHERE user_id = $1").bind(user_id).execute(tx.as_mut()).await?;

		// 删除持仓
		sqlx::query("DELETE FROM positions WHERE user_id = $1").bind(user_id).execute(tx.as_mut()).await?;

		// 删除资产历史
		sqlx::query("DELETE FROM asset_history WHERE user_id = $1").bind(user_id).execute(tx.as_mut()).await?;

		// 删除用户
		sqlx::query("DELETE FROM users WHERE id = $1").bind(user_id).execute(tx.as_mut()).await?;

		tx.commit().await?;
		Ok(())
	}

	/// 清理测试事件和市场
	pub async fn cleanup_test_event(&self, event_id: i64) -> anyhow::Result<()> {
		let mut tx = self.pool.begin().await?;

		// 删除事件（markets 数据在 events 表的 JSONB 字段中）
		sqlx::query("DELETE FROM events WHERE id = $1").bind(event_id).execute(tx.as_mut()).await?;

		tx.commit().await?;
		Ok(())
	}
}

/// 生成唯一的测试用户 ID
pub fn generate_test_user_id() -> i64 {
	use std::time::{SystemTime, UNIX_EPOCH};
	let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
	(now.as_secs() * 1_000_000 + now.subsec_micros() as u64) as i64
}

/// 生成唯一的测试事件 ID
pub fn generate_test_event_id() -> i64 {
	generate_test_user_id()
}

/// 解析 Decimal
pub fn parse_decimal(s: &str) -> Decimal {
	Decimal::from_str(s).unwrap()
}
