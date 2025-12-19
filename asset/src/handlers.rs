use {
	crate::db::get_db_pool,
	chrono::Utc,
	common::{
		consts::USDC_TOKEN_ID,
		engine_types::{OrderSide, OrderStatus},
		model::{AssetHistoryType, Positions, SignatureOrderMsg},
	},
	rust_decimal::Decimal,
	sqlx::{Postgres, Row, Transaction, types::Uuid},
	std::collections::HashMap,
	tracing::info,
};

/// 辅助函数：安全的 Decimal 加法
fn checked_add(a: Decimal, b: Decimal, msg: &str) -> anyhow::Result<Decimal> {
	a.checked_add(b).ok_or_else(|| anyhow::anyhow!("Decimal addition overflow: {} ({} + {})", msg, a, b))
}

/// 辅助函数：安全的 Decimal 减法
fn checked_sub(a: Decimal, b: Decimal, msg: &str) -> anyhow::Result<Decimal> {
	a.checked_sub(b).ok_or_else(|| anyhow::anyhow!("Decimal subtraction overflow: {} ({} - {})", msg, a, b))
}

/// 辅助函数：安全的 Decimal 乘法
#[allow(dead_code)]
fn checked_mul(a: Decimal, b: Decimal, msg: &str) -> anyhow::Result<Decimal> {
	a.checked_mul(b).ok_or_else(|| anyhow::anyhow!("Decimal multiplication overflow: {} ({} * {})", msg, a, b))
}

/// 辅助函数：安全的 Decimal 除法
fn checked_div(a: Decimal, b: Decimal, msg: &str) -> anyhow::Result<Decimal> {
	if b.is_zero() {
		return Err(anyhow::anyhow!("Decimal division by zero: {}", msg));
	}
	a.checked_div(b).ok_or_else(|| anyhow::anyhow!("Decimal division overflow: {} ({} / {})", msg, a, b))
}

/// 排序键，用于避免死锁（按字典序排序）
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct LockKey {
	user_id: String, //好统一字典序
	token_id: String,
}

impl LockKey {
	fn new(user_id: i64, token_id: String) -> Self {
		Self { user_id: user_id.to_string(), token_id }
	}
}

/// 对需要加锁的键进行排序并去重
fn sort_and_dedup_lock_keys(keys: &mut Vec<LockKey>) {
	keys.sort();
	keys.dedup();
}

/// 使用行锁获取 Position（如果不存在返回 None）
async fn get_position_with_lock(tx: &mut Transaction<'_, Postgres>, user_id: i64, token_id: &str) -> anyhow::Result<Option<Positions>> {
	let position = sqlx::query_as::<_, Positions>("SELECT * FROM positions WHERE user_id = $1 AND token_id = $2 FOR UPDATE").bind(user_id).bind(token_id).fetch_optional(tx.as_mut()).await?;
	Ok(position)
}

/// 获取或创建 Position（使用行锁）
async fn get_or_create_position(
	tx: &mut Transaction<'_, Postgres>,
	user_id: i64,
	event_id: Option<i64>,
	market_id: Option<i16>,
	token_id: &str,
	privy_id: Option<String>,
	outcome_name: Option<String>,
) -> anyhow::Result<Positions> {
	// 先尝试获取
	if let Some(position) = get_position_with_lock(tx, user_id, token_id).await? {
		return Ok(position);
	}

	// 不存在则创建
	let now = Utc::now();
	let position = Positions {
		user_id,
		event_id,
		market_id,
		token_id: token_id.to_string(),
		balance: Decimal::ZERO,
		frozen_balance: Decimal::ZERO,
		usdc_cost: if token_id == USDC_TOKEN_ID { None } else { Some(Decimal::ZERO) },
		avg_price: if token_id == USDC_TOKEN_ID { None } else { Some(Decimal::ZERO) },
		redeemed: if token_id == USDC_TOKEN_ID { None } else { Some(false) },
		payout: None,
		redeemed_timestamp: None,
		privy_id,
		outcome_name,
		update_id: 1, // 新创建的 position，update_id 为 1
		updated_at: now,
		created_at: now,
	};

	// 插入数据库
	sqlx::query(
		"INSERT INTO positions (user_id, event_id, market_id, token_id, balance, frozen_balance, usdc_cost, avg_price, redeemed, payout, redeemed_timestamp, privy_id, outcome_name, update_id, updated_at, created_at)
		 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)",
	)
	.bind(position.user_id)
	.bind(position.event_id)
	.bind(position.market_id)
	.bind(&position.token_id)
	.bind(position.balance)
	.bind(position.frozen_balance)
	.bind(position.usdc_cost)
	.bind(position.avg_price)
	.bind(position.redeemed)
	.bind(position.payout)
	.bind(position.redeemed_timestamp)
	.bind(&position.privy_id)
	.bind(&position.outcome_name)
	.bind(position.update_id)
	.bind(position.updated_at)
	.bind(position.created_at)
	.execute(tx.as_mut())
	.await?;

	Ok(position)
}

/// Position 更新参数
struct PositionUpdate<'a> {
	user_id: i64,
	token_id: &'a str,
	balance: Decimal,
	frozen_balance: Decimal,
	usdc_cost: Option<Decimal>,
	avg_price: Option<Decimal>,
	redeemed: Option<bool>,
	payout: Option<Decimal>,
	redeemed_timestamp: Option<chrono::DateTime<Utc>>,
}

/// MakerOnchainInfo 结构（来自链上交易结果）
pub struct MakerOnchainInfo {
	pub maker_side: OrderSide,
	pub maker_user_id: i64,
	pub maker_usdc_amount: Decimal,
	pub maker_token_amount: Decimal,
	pub maker_token_id: String,
	#[allow(dead_code)]
	pub maker_order_id: Uuid,
	pub real_maker_usdc_amount: Decimal,
	pub real_maker_token_amount: Decimal,
	pub maker_privy_user_id: String,
	pub maker_outcome_name: String,
}

/// 链上交易结果处理参数
pub struct TradeOnchainSendParams<'a> {
	pub trade_id: Uuid,
	pub event_id: i64,
	pub market_id: i16,
	pub taker_side: OrderSide,
	pub taker_user_id: i64,
	pub taker_usdc_amount: Decimal,
	pub taker_token_amount: Decimal,
	pub taker_token_id: &'a str,
	pub taker_order_id: Uuid,
	pub real_taker_usdc_amount: Decimal,
	pub real_taker_token_amount: Decimal,
	pub taker_unfreeze_amount: Decimal,
	pub taker_privy_user_id: &'a str,
	pub taker_outcome_name: &'a str,
	pub maker_infos: Vec<MakerOnchainInfo>,
	pub tx_hash: &'a str,
	pub success: bool,
	pub pool: Option<sqlx::PgPool>,
}

use crate::user_event::{PositionEvent, PositionEventType};

/// 更新 Position，返回新的 update_id
async fn update_position(tx: &mut Transaction<'_, Postgres>, params: PositionUpdate<'_>) -> anyhow::Result<i64> {
	let now = Utc::now();
	let update_id: i64 = sqlx::query_scalar(
		"UPDATE positions
		 SET balance = $3, frozen_balance = $4, usdc_cost = $5, avg_price = $6, redeemed = $7, payout = $8, redeemed_timestamp = $9, update_id = update_id + 1, updated_at = $10
		 WHERE user_id = $1 AND token_id = $2
		 RETURNING update_id",
	)
	.bind(params.user_id)
	.bind(params.token_id)
	.bind(params.balance)
	.bind(params.frozen_balance)
	.bind(params.usdc_cost)
	.bind(params.avg_price)
	.bind(params.redeemed)
	.bind(params.payout)
	.bind(params.redeemed_timestamp)
	.bind(now)
	.fetch_one(tx.as_mut())
	.await?;
	Ok(update_id)
}

/// AssetHistoryInsert helper struct
struct AssetHistoryInsert<'a> {
	user_id: i64,
	history_type: AssetHistoryType,
	usdc_amount: Option<Decimal>,
	usdc_balance_before: Option<Decimal>,
	usdc_balance_after: Option<Decimal>,
	usdc_frozen_balance_before: Option<Decimal>,
	usdc_frozen_balance_after: Option<Decimal>,
	token_id: Option<&'a str>,
	token_amount: Option<Decimal>,
	token_balance_before: Option<Decimal>,
	token_balance_after: Option<Decimal>,
	token_frozen_balance_before: Option<Decimal>,
	token_frozen_balance_after: Option<Decimal>,
	tx_hash: Option<&'a str>,
	trade_id: Option<String>,
	order_id: Option<String>,
}

/// 插入 AssetHistory 记录 (wrapper using struct)
async fn insert_asset_history_struct(tx: &mut Transaction<'_, Postgres>, params: AssetHistoryInsert<'_>) -> anyhow::Result<()> {
	insert_asset_history(
		tx,
		params.user_id,
		params.history_type,
		params.usdc_amount,
		params.usdc_balance_before,
		params.usdc_balance_after,
		params.usdc_frozen_balance_before,
		params.usdc_frozen_balance_after,
		params.token_id.map(|s| s.to_string()),
		params.token_amount,
		params.token_balance_before,
		params.token_balance_after,
		params.token_frozen_balance_before,
		params.token_frozen_balance_after,
		params.tx_hash.map(|s| s.to_string()),
		params.trade_id,
		params.order_id,
	)
	.await
}

/// 插入 AssetHistory 记录
#[allow(clippy::too_many_arguments)]
async fn insert_asset_history(
	tx: &mut Transaction<'_, Postgres>,
	user_id: i64,
	history_type: AssetHistoryType,
	usdc_amount: Option<Decimal>,
	usdc_balance_before: Option<Decimal>,
	usdc_balance_after: Option<Decimal>,
	usdc_frozen_balance_before: Option<Decimal>,
	usdc_frozen_balance_after: Option<Decimal>,
	token_id: Option<String>,
	token_amount: Option<Decimal>,
	token_balance_before: Option<Decimal>,
	token_balance_after: Option<Decimal>,
	token_frozen_balance_before: Option<Decimal>,
	token_frozen_balance_after: Option<Decimal>,
	tx_hash: Option<String>,
	trade_id: Option<String>,
	order_id: Option<String>,
) -> anyhow::Result<()> {
	sqlx::query(
		"INSERT INTO asset_history
		(user_id, history_type, usdc_amount, usdc_balance_before, usdc_balance_after, usdc_frozen_balance_before, usdc_frozen_balance_after,
		 token_id, token_amount, token_balance_before, token_balance_after, token_frozen_balance_before, token_frozen_balance_after,
		 tx_hash, trade_id, order_id, created_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
	)
	.bind(user_id)
	.bind(history_type)
	.bind(usdc_amount)
	.bind(usdc_balance_before)
	.bind(usdc_balance_after)
	.bind(usdc_frozen_balance_before)
	.bind(usdc_frozen_balance_after)
	.bind(token_id)
	.bind(token_amount)
	.bind(token_balance_before)
	.bind(token_balance_after)
	.bind(token_frozen_balance_before)
	.bind(token_frozen_balance_after)
	.bind(tx_hash)
	.bind(trade_id)
	.bind(order_id)
	.bind(Utc::now())
	.execute(tx.as_mut())
	.await?;
	Ok(())
}

/// 插入 OperationHistory 记录
#[allow(clippy::too_many_arguments)]
async fn insert_operation_history(
	tx: &mut Transaction<'_, Postgres>,
	event_id: i64,
	market_id: i16,
	user_id: i64,
	history_type: AssetHistoryType,
	outcome_name: Option<String>,
	token_id: Option<String>,
	quantity: Option<Decimal>,
	price: Option<Decimal>,
	value: Option<Decimal>,
	tx_hash: &str,
) -> anyhow::Result<()> {
	sqlx::query(
		"INSERT INTO operation_history
		(event_id, market_id, user_id, history_type, outcome_name, token_id, quantity, price, value, tx_hash, created_at)
		VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
	)
	.bind(event_id)
	.bind(market_id)
	.bind(user_id)
	.bind(history_type)
	.bind(outcome_name)
	.bind(token_id)
	.bind(quantity)
	.bind(price)
	.bind(value)
	.bind(tx_hash)
	.bind(Utc::now())
	.execute(tx.as_mut())
	.await?;
	Ok(())
}

/// 处理充值
#[allow(clippy::too_many_arguments)]
pub async fn handle_deposit(
	user_id: i64,
	token_id: &str,
	amount: Decimal,
	tx_hash: &str,
	event_id: Option<i64>,
	market_id: Option<i16>,
	privy_id: Option<String>,
	outcome_name: Option<String>,
	pool: Option<sqlx::PgPool>,
) -> anyhow::Result<()> {
	let pool = match pool {
		Some(p) => p,
		None => get_db_pool()?,
	};
	let mut tx = pool.begin().await?;

	// 获取或创建 Position (使用行锁)
	let mut position = get_or_create_position(&mut tx, user_id, event_id, market_id, token_id, privy_id, outcome_name).await?;

	// 记录变化前的状态
	let balance_before = position.balance;
	let frozen_balance_before = position.frozen_balance;

	// 计算新余额
	let new_balance = checked_add(position.balance, amount, "deposit balance")?;

	// 更新 position
	position.balance = new_balance;
	let _update_id = update_position(
		&mut tx,
		PositionUpdate {
			user_id,
			token_id,
			balance: new_balance,
			frozen_balance: position.frozen_balance,
			usdc_cost: position.usdc_cost,
			avg_price: position.avg_price,
			redeemed: position.redeemed,
			payout: position.payout,
			redeemed_timestamp: None,
		},
	)
	.await?;

	// 插入 asset_history
	if token_id == USDC_TOKEN_ID {
		// USDC 充值
		insert_asset_history(
			&mut tx,
			user_id,
			AssetHistoryType::Deposit,
			Some(amount),                // usdc_amount
			Some(balance_before),        // usdc_balance_before
			Some(new_balance),           // usdc_balance_after
			Some(frozen_balance_before), // usdc_frozen_balance_before
			Some(frozen_balance_before), // usdc_frozen_balance_after (unchanged)
			None,                        // token_id
			None,                        // token_amount
			None,                        // token_balance_before
			None,                        // token_balance_after
			None,                        // token_frozen_balance_before
			None,                        // token_frozen_balance_after
			Some(tx_hash.to_string()),   // tx_hash
			None,                        // trade_id (deposit 不涉及交易)
			None,                        // order_id (deposit 不涉及订单)
		)
		.await?;
	} else {
		// Token 充值
		insert_asset_history(
			&mut tx,
			user_id,
			AssetHistoryType::Deposit,
			None, // usdc_amount
			None, // usdc_balance_before
			None, // usdc_balance_after
			None, // usdc_frozen_balance_before
			None, // usdc_frozen_balance_after
			Some(token_id.to_string()),
			Some(amount),                // token_amount
			Some(balance_before),        // token_balance_before
			Some(new_balance),           // token_balance_after
			Some(frozen_balance_before), // token_frozen_balance_before
			Some(frozen_balance_before), // token_frozen_balance_after (unchanged)
			Some(tx_hash.to_string()),   // tx_hash
			None,                        // trade_id (deposit 不涉及交易)
			None,                        // order_id (deposit 不涉及订单)
		)
		.await?;
	}

	tx.commit().await?;

	info!("handle_deposit success: user_id={}, token_id={}, amount={}, balance={}->{}, tx_hash={}", user_id, token_id, amount, balance_before, new_balance, tx_hash);

	Ok(())
}

/// 处理提现
#[allow(clippy::too_many_arguments)]
pub async fn handle_withdraw(
	user_id: i64,
	token_id: &str,
	amount: Decimal,
	tx_hash: &str,
	_event_id: Option<i64>,
	_market_id: Option<i16>,
	_privy_id: Option<String>,
	_outcome_name: Option<String>,
	pool: Option<sqlx::PgPool>,
) -> anyhow::Result<()> {
	let pool = match pool {
		Some(p) => p,
		None => get_db_pool()?,
	};
	let mut tx = pool.begin().await?;

	// 获取 Position (使用行锁，提现必须有资产存在)
	let mut position = get_position_with_lock(&mut tx, user_id, token_id).await?.ok_or_else(|| anyhow::anyhow!("Position not found for withdrawal: user_id={}, token_id={}", user_id, token_id))?;

	// 不需要检查余额是否足够 因为提现是链上操作 用于控制自己的私钥 提现之后冻结的资产后续会因为交易失败而解冻
	// if position.balance < amount {
	// 	anyhow::bail!("Insufficient balance for withdrawal: user_id={}, token_id={}, balance={}, withdraw_amount={}", user_id, token_id, position.balance, amount);
	// }

	// 记录变化前的状态
	let balance_before = position.balance;
	let frozen_balance_before = position.frozen_balance;

	// 计算新余额
	let new_balance = checked_sub(position.balance, amount, "withdraw balance")?;

	// 更新 position
	position.balance = new_balance;
	let _update_id = update_position(
		&mut tx,
		PositionUpdate {
			user_id,
			token_id,
			balance: new_balance,
			frozen_balance: position.frozen_balance,
			usdc_cost: position.usdc_cost,
			avg_price: position.avg_price,
			redeemed: position.redeemed,
			payout: position.payout,
			redeemed_timestamp: None,
		},
	)
	.await?;

	// 插入 asset_history
	if token_id == USDC_TOKEN_ID {
		// USDC 提现
		insert_asset_history(
			&mut tx,
			user_id,
			AssetHistoryType::Withdraw,
			Some(-amount),               // usdc_amount (negative)
			Some(balance_before),        // usdc_balance_before
			Some(new_balance),           // usdc_balance_after
			Some(frozen_balance_before), // usdc_frozen_balance_before
			Some(frozen_balance_before), // usdc_frozen_balance_after (unchanged)
			None,                        // token_id
			None,                        // token_amount
			None,                        // token_balance_before
			None,                        // token_balance_after
			None,                        // token_frozen_balance_before
			None,                        // token_frozen_balance_after
			Some(tx_hash.to_string()),   // tx_hash
			None,                        // trade_id (withdraw 不涉及交易)
			None,                        // order_id (withdraw 不涉及订单)
		)
		.await?;
	} else {
		// Token 提现
		insert_asset_history(
			&mut tx,
			user_id,
			AssetHistoryType::Withdraw,
			None, // usdc_amount
			None, // usdc_balance_before
			None, // usdc_balance_after
			None, // usdc_frozen_balance_before
			None, // usdc_frozen_balance_after
			Some(token_id.to_string()),
			Some(-amount),               // token_amount (negative)
			Some(balance_before),        // token_balance_before
			Some(new_balance),           // token_balance_after
			Some(frozen_balance_before), // token_frozen_balance_before
			Some(frozen_balance_before), // token_frozen_balance_after (unchanged)
			Some(tx_hash.to_string()),   // tx_hash
			None,                        // trade_id (withdraw 不涉及交易)
			None,                        // order_id (withdraw 不涉及订单)
		)
		.await?;
	}

	tx.commit().await?;

	info!("handle_withdraw success: user_id={}, token_id={}, amount={}, balance={}->{}, tx_hash={}", user_id, token_id, amount, balance_before, new_balance, tx_hash);

	Ok(())
}

/// 处理创建订单
#[allow(clippy::too_many_arguments)]
pub async fn handle_create_order(
	user_id: i64,
	event_id: i64,
	market_id: i16,
	token_id: &str,
	outcome_name: &str,
	order_side: common::engine_types::OrderSide,
	order_type: common::engine_types::OrderType,
	price: Decimal,
	quantity: Decimal,
	volume: Decimal,
	signature_order_msg: common::model::SignatureOrderMsg,
	pool: Option<sqlx::PgPool>,
) -> anyhow::Result<Uuid> {
	use common::engine_types::OrderSide;

	let pool = match pool {
		Some(p) => p,
		None => get_db_pool()?,
	};
	let mut tx = pool.begin().await?;

	// 根据订单方向确定需要冻结的资产
	let (freeze_token_id, freeze_amount) = match order_side {
		OrderSide::Buy => (USDC_TOKEN_ID, volume), // Buy: freeze USDC
		OrderSide::Sell => (token_id, quantity),   // Sell: freeze token
	};

	// 1. 获取 Position (使用行锁，下单必须有资产存在)
	let position =
		get_position_with_lock(&mut tx, user_id, freeze_token_id).await?.ok_or_else(|| anyhow::anyhow!("Position not found for creating order: user_id={}, token_id={}", user_id, freeze_token_id))?;

	// 检查余额是否足够
	if position.balance < freeze_amount {
		anyhow::bail!("Insufficient balance for order: user_id={}, token_id={}, balance={}, freeze_amount={}", user_id, freeze_token_id, position.balance, freeze_amount);
	}

	// 记录变化前的状态
	let balance_before = position.balance;
	let frozen_balance_before = position.frozen_balance;

	// 计算新状态
	let new_balance = checked_sub(position.balance, freeze_amount, "create_order balance")?;
	let new_frozen_balance = checked_add(position.frozen_balance, freeze_amount, "create_order frozen_balance")?;

	// 2. 更新 position（冻结资产）
	let _update_id = update_position(
		&mut tx,
		PositionUpdate {
			user_id,
			token_id: freeze_token_id,
			balance: new_balance,
			frozen_balance: new_frozen_balance,
			usdc_cost: position.usdc_cost,
			avg_price: position.avg_price,
			redeemed: position.redeemed,
			payout: position.payout,
			redeemed_timestamp: None,
		},
	)
	.await?;

	// 3. 插入 orders 表
	let now = Utc::now();
	let order_id = Uuid::new_v4();

	// 根据订单类型决定插入的字段值
	// 限价单和市价单的字段完全分离：一种类型的单子另一边为 0
	let (limit_price, limit_quantity, limit_volume, market_price, market_quantity, market_volume) = match order_type {
		common::engine_types::OrderType::Market => {
			// 市价单：填充 market_* 字段，price/quantity/volume 为 0
			(Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, price, quantity, volume)
		}
		common::engine_types::OrderType::Limit => {
			// 限价单：填充 price/quantity/volume，market_* 字段为 0
			(price, quantity, volume, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO)
		}
	};

	sqlx::query(
		"INSERT INTO orders (id, user_id, event_id, market_id, token_id, outcome, order_side, order_type, price, quantity, volume, filled_quantity, cancelled_quantity, market_price, market_quantity, market_volume, market_filled_quantity, market_cancelled_quantity, market_filled_volume, market_cancelled_volume, status, signature_order_msg, created_at, updated_at)
		 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24)"
	)
	.bind(order_id)
	.bind(user_id)
	.bind(event_id)
	.bind(market_id)
	.bind(token_id)
	.bind(outcome_name)
	.bind(order_side)
	.bind(order_type)
	.bind(limit_price)
	.bind(limit_quantity)
	.bind(limit_volume)
	.bind(Decimal::ZERO) // filled_quantity
	.bind(Decimal::ZERO) // cancelled_quantity
	.bind(market_price)
	.bind(market_quantity)
	.bind(market_volume)
	.bind(Decimal::ZERO) // market_filled_quantity
	.bind(Decimal::ZERO) // market_cancelled_quantity
	.bind(Decimal::ZERO) // market_filled_volume
	.bind(Decimal::ZERO) // market_cancelled_volume
	.bind(common::engine_types::OrderStatus::New)
	.bind(sqlx::types::Json(&signature_order_msg))
	.bind(now)
	.bind(now) // updated_at
	.execute(tx.as_mut())
	.await?;

	// 4. 插入 asset_history
	if freeze_token_id == USDC_TOKEN_ID {
		// USDC 冻结
		insert_asset_history(
			&mut tx,
			user_id,
			AssetHistoryType::CreateOrder,
			Some(-freeze_amount),        // usdc_amount (negative for freeze)
			Some(balance_before),        // usdc_balance_before
			Some(new_balance),           // usdc_balance_after
			Some(frozen_balance_before), // usdc_frozen_balance_before
			Some(new_frozen_balance),    // usdc_frozen_balance_after
			None,
			None,
			None,
			None,
			None,
			None,
			None,
			None,                       // trade_id (创建订单不涉及交易)
			Some(order_id.to_string()), // order_id
		)
		.await?;
	} else {
		// Token 冻结
		insert_asset_history(
			&mut tx,
			user_id,
			AssetHistoryType::CreateOrder,
			None,
			None,
			None,
			None,
			None,
			Some(freeze_token_id.to_string()),
			Some(-freeze_amount),        // token_amount (negative for freeze)
			Some(balance_before),        // token_balance_before
			Some(new_balance),           // token_balance_after
			Some(frozen_balance_before), // token_frozen_balance_before
			Some(new_frozen_balance),    // token_frozen_balance_after
			None,
			None,                       // trade_id (创建订单不涉及交易)
			Some(order_id.to_string()), // order_id
		)
		.await?;
	}

	tx.commit().await?;

	info!(
		"handle_create_order success: user_id={}, order_id={}, token_id={}, side={:?}, freeze_amount={}, balance={}->{}, frozen_balance={}->{}",
		user_id, order_id, freeze_token_id, order_side, freeze_amount, balance_before, new_balance, frozen_balance_before, new_frozen_balance
	);

	Ok(order_id)
}

/// 处理订单拒绝
pub async fn handle_order_rejected(user_id: i64, order_id: Uuid, pool: Option<sqlx::PgPool>) -> anyhow::Result<()> {
	use common::engine_types::OrderSide;

	let pool = match pool {
		Some(p) => p,
		None => get_db_pool()?,
	};
	let mut tx = pool.begin().await?;

	// 查询订单信息
	let order: common::model::Orders = sqlx::query_as("SELECT * FROM orders WHERE id = $1 AND user_id = $2").bind(order_id).bind(user_id).fetch_one(tx.as_mut()).await?;

	// 根据订单类型获取对应的 quantity 和 volume
	let (quantity, volume) = match order.order_type {
		common::engine_types::OrderType::Limit => (order.quantity, order.volume),
		common::engine_types::OrderType::Market => (order.market_quantity, order.market_volume),
	};

	// 确定解冻的资产
	let (unfreeze_token_id, unfreeze_amount) = match order.order_side {
		OrderSide::Buy => (USDC_TOKEN_ID, volume),
		OrderSide::Sell => (order.token_id.as_str(), quantity),
	};

	// 获取 Position (使用行锁)
	let position = get_position_with_lock(&mut tx, user_id, unfreeze_token_id).await?.ok_or_else(|| anyhow::anyhow!("Position not found"))?;

	// 记录变化前的状态
	let balance_before = position.balance;
	let frozen_balance_before = position.frozen_balance;

	// 计算新状态
	let new_balance = checked_add(position.balance, unfreeze_amount, "order_rejected balance")?;
	let new_frozen_balance = checked_sub(position.frozen_balance, unfreeze_amount, "order_rejected frozen_balance")?;

	// 更新 position
	let _update_id = update_position(
		&mut tx,
		PositionUpdate {
			user_id,
			token_id: unfreeze_token_id,
			balance: new_balance,
			frozen_balance: new_frozen_balance,
			usdc_cost: position.usdc_cost,
			avg_price: position.avg_price,
			redeemed: position.redeemed,
			payout: position.payout,
			redeemed_timestamp: None,
		},
	)
	.await?;

	// 更新订单状态为 Rejected
	sqlx::query("UPDATE orders SET status = $1, updated_at = $2 WHERE id = $3").bind(common::engine_types::OrderStatus::Rejected).bind(Utc::now()).bind(order_id).execute(tx.as_mut()).await?;

	// 插入 asset_history
	if unfreeze_token_id == USDC_TOKEN_ID {
		insert_asset_history(
			&mut tx,
			user_id,
			AssetHistoryType::OrderRejected,
			Some(unfreeze_amount),       // usdc_amount (positive for unfreeze)
			Some(balance_before),        // usdc_balance_before
			Some(new_balance),           // usdc_balance_after
			Some(frozen_balance_before), // usdc_frozen_balance_before
			Some(new_frozen_balance),    // usdc_frozen_balance_after
			None,
			None,
			None,
			None,
			None,
			None,
			None,
			None,                       // trade_id (订单拒绝不涉及交易)
			Some(order_id.to_string()), // order_id
		)
		.await?;
	} else {
		insert_asset_history(
			&mut tx,
			user_id,
			AssetHistoryType::OrderRejected,
			None,
			None,
			None,
			None,
			None,
			Some(unfreeze_token_id.to_string()),
			Some(unfreeze_amount),       // token_amount (positive for unfreeze)
			Some(balance_before),        // token_balance_before
			Some(new_balance),           // token_balance_after
			Some(frozen_balance_before), // token_frozen_balance_before
			Some(new_frozen_balance),    // token_frozen_balance_after
			None,
			None,                       // trade_id (订单拒绝不涉及交易)
			Some(order_id.to_string()), // order_id
		)
		.await?;
	}

	tx.commit().await?;

	info!(
		"handle_order_rejected success: user_id={}, order_id={}, unfreeze_token_id={}, unfreeze_amount={}, balance={}->{}, frozen_balance={}->{}",
		user_id, order_id, unfreeze_token_id, unfreeze_amount, balance_before, new_balance, frozen_balance_before, new_frozen_balance
	);

	Ok(())
}

/// 处理取消订单
pub async fn handle_cancel_order(user_id: i64, order_id: Uuid, cancelled_quantity: Decimal, volume: Decimal, pool: Option<sqlx::PgPool>) -> anyhow::Result<i64> {
	use common::engine_types::OrderSide;

	let pool = match pool {
		Some(p) => p,
		None => get_db_pool()?,
	};
	let mut tx = pool.begin().await?;

	// 查询订单信息
	let order: common::model::Orders = sqlx::query_as("SELECT * FROM orders WHERE id = $1 AND user_id = $2").bind(order_id).bind(user_id).fetch_one(tx.as_mut()).await?;

	// 确定解冻的资产
	let (unfreeze_token_id, unfreeze_amount) = match order.order_side {
		OrderSide::Buy => (USDC_TOKEN_ID, volume),
		OrderSide::Sell => (order.token_id.as_str(), cancelled_quantity),
	};

	// 获取 Position (使用行锁)
	let position = get_position_with_lock(&mut tx, user_id, unfreeze_token_id).await?.ok_or_else(|| anyhow::anyhow!("Position not found"))?;

	// 记录变化前的状态
	let balance_before = position.balance;
	let frozen_balance_before = position.frozen_balance;

	// 计算新状态
	let new_balance = checked_add(position.balance, unfreeze_amount, "cancel_order balance")?;
	let new_frozen_balance = checked_sub(position.frozen_balance, unfreeze_amount, "cancel_order frozen_balance")?;

	// 更新 position
	let _update_id = update_position(
		&mut tx,
		PositionUpdate {
			user_id,
			token_id: unfreeze_token_id,
			balance: new_balance,
			frozen_balance: new_frozen_balance,
			usdc_cost: position.usdc_cost,
			avg_price: position.avg_price,
			redeemed: position.redeemed,
			payout: position.payout,
			redeemed_timestamp: None,
		},
	)
	.await?;

	// 更新订单状态为 Cancelled 并累加对应的 cancelled 字段，同时更新 update_id
	// 根据订单类型和方向选择更新的字段
	let order_update_id: (i64,) = match (order.order_type, order.order_side) {
		(common::engine_types::OrderType::Market, OrderSide::Buy) => {
			// 市价买单：更新 market_cancelled_volume
			sqlx::query_as("UPDATE orders SET status = $1, market_cancelled_volume = market_cancelled_volume + $2, updated_at = $3, update_id = update_id + 1 WHERE id = $4 RETURNING update_id")
				.bind(common::engine_types::OrderStatus::Cancelled)
				.bind(volume)
				.bind(Utc::now())
				.bind(order_id)
				.fetch_one(tx.as_mut())
				.await?
		}
		(common::engine_types::OrderType::Market, OrderSide::Sell) => {
			// 市价卖单：更新 market_cancelled_quantity
			sqlx::query_as("UPDATE orders SET status = $1, market_cancelled_quantity = market_cancelled_quantity + $2, updated_at = $3, update_id = update_id + 1 WHERE id = $4 RETURNING update_id")
				.bind(common::engine_types::OrderStatus::Cancelled)
				.bind(cancelled_quantity)
				.bind(Utc::now())
				.bind(order_id)
				.fetch_one(tx.as_mut())
				.await?
		}
		(common::engine_types::OrderType::Limit, _) => {
			// 限价单：更新 cancelled_quantity
			sqlx::query_as("UPDATE orders SET status = $1, cancelled_quantity = cancelled_quantity + $2, updated_at = $3, update_id = update_id + 1 WHERE id = $4 RETURNING update_id")
				.bind(common::engine_types::OrderStatus::Cancelled)
				.bind(cancelled_quantity)
				.bind(Utc::now())
				.bind(order_id)
				.fetch_one(tx.as_mut())
				.await?
		}
	};

	// 插入 asset_history
	if unfreeze_token_id == USDC_TOKEN_ID {
		insert_asset_history(
			&mut tx,
			user_id,
			AssetHistoryType::CancelOrder,
			Some(unfreeze_amount),       // usdc_amount (positive for unfreeze)
			Some(balance_before),        // usdc_balance_before
			Some(new_balance),           // usdc_balance_after
			Some(frozen_balance_before), // usdc_frozen_balance_before
			Some(new_frozen_balance),    // usdc_frozen_balance_after
			None,
			None,
			None,
			None,
			None,
			None,
			None,
			None,                       // trade_id (取消订单不涉及交易)
			Some(order_id.to_string()), // order_id
		)
		.await?;
	} else {
		insert_asset_history(
			&mut tx,
			user_id,
			AssetHistoryType::CancelOrder,
			None,
			None,
			None,
			None,
			None,
			Some(unfreeze_token_id.to_string()),
			Some(unfreeze_amount),       // token_amount (positive for unfreeze)
			Some(balance_before),        // token_balance_before
			Some(new_balance),           // token_balance_after
			Some(frozen_balance_before), // token_frozen_balance_before
			Some(new_frozen_balance),    // token_frozen_balance_after
			None,
			None,                       // trade_id (取消订单不涉及交易)
			Some(order_id.to_string()), // order_id
		)
		.await?;
	}

	tx.commit().await?;

	info!(
		"handle_cancel_order success: user_id={}, order_id={}, update_id={}, unfreeze_token_id={}, unfreeze_amount={}, balance={}->{}, frozen_balance={}->{}",
		user_id, order_id, order_update_id.0, unfreeze_token_id, unfreeze_amount, balance_before, new_balance, frozen_balance_before, new_frozen_balance
	);

	Ok(order_update_id.0)
}

/// 处理 Split (USDC -> Token0 + Token1)
#[allow(clippy::too_many_arguments)]
pub async fn handle_split(
	user_id: i64,
	event_id: i64,
	market_id: i16,
	usdc_amount: Decimal,
	token_0_id: &str,
	token_1_id: &str,
	token_0_amount: Decimal,
	token_1_amount: Decimal,
	tx_hash: &str,
	privy_id: &str,
	outcome_name_0: &str,
	outcome_name_1: &str,
	pool: Option<sqlx::PgPool>,
) -> anyhow::Result<()> {
	let pool = match pool {
		Some(p) => p,
		None => get_db_pool()?,
	};
	let mut tx = pool.begin().await?;

	// 收集 LockKey 并排序
	let mut keys = vec![LockKey::new(user_id, USDC_TOKEN_ID.to_string()), LockKey::new(user_id, token_0_id.to_string()), LockKey::new(user_id, token_1_id.to_string())];
	sort_and_dedup_lock_keys(&mut keys);

	// 按排序后的顺序行锁所有资产，并存入 HashMap
	let mut positions = HashMap::new();
	for key in keys.iter() {
		let token_id = key.token_id.as_str();
		if token_id == USDC_TOKEN_ID {
			let pos = get_position_with_lock(&mut tx, user_id, token_id).await?.ok_or_else(|| anyhow::anyhow!("Position not found for token: {}", token_id))?;
			positions.insert(token_id.to_string(), pos);
		} else if token_id == token_0_id {
			let pos = get_or_create_position(&mut tx, user_id, Some(event_id), Some(market_id), token_id, Some(privy_id.to_string()), Some(outcome_name_0.to_string())).await?;
			positions.insert(token_id.to_string(), pos);
		} else {
			let pos = get_or_create_position(&mut tx, user_id, Some(event_id), Some(market_id), token_id, Some(privy_id.to_string()), Some(outcome_name_1.to_string())).await?;
			positions.insert(token_id.to_string(), pos);
		};
	}

	// 从 HashMap 获取各个 position
	let usdc_pos = positions.get(USDC_TOKEN_ID).ok_or_else(|| anyhow::anyhow!("USDC position not found"))?.clone();
	let token_0_pos = positions.get(token_0_id).ok_or_else(|| anyhow::anyhow!("Token_0 position not found"))?.clone();
	let token_1_pos = positions.get(token_1_id).ok_or_else(|| anyhow::anyhow!("Token_1 position not found"))?.clone();

	// 不需要检查 USDC 余额 因为split是链上操作 用于控制自己的私钥 split之后冻结的资产后续会因为交易失败而解冻
	// if usdc_pos.balance < usdc_amount {
	// 	anyhow::bail!("Insufficient USDC balance for split");
	// }

	// 计算所有的中间值
	let half_usdc = checked_div(usdc_amount, Decimal::from(2), "split usdc_amount by 2")?;

	let usdc_balance_before = usdc_pos.balance;
	let usdc_balance_after = checked_sub(usdc_balance_before, usdc_amount, "split usdc balance")?;

	let token_0_balance_before = token_0_pos.balance;
	let token_0_balance_after = checked_add(token_0_balance_before, token_0_amount, "split token_0 balance")?;
	let token_0_usdc_cost = checked_add(token_0_pos.usdc_cost.unwrap_or(Decimal::ZERO), half_usdc, "split token_0 usdc_cost")?;
	let token_0_total = token_0_balance_after + token_0_pos.frozen_balance;
	let token_0_avg_price = if token_0_total > Decimal::ZERO { checked_div(token_0_usdc_cost, token_0_total, "split token_0 avg_price")?.trunc_with_scale(8).normalize() } else { Decimal::ZERO };

	let token_1_balance_before = token_1_pos.balance;
	let token_1_balance_after = checked_add(token_1_balance_before, token_1_amount, "split token_1 balance")?;
	let token_1_usdc_cost = checked_add(token_1_pos.usdc_cost.unwrap_or(Decimal::ZERO), half_usdc, "split token_1 usdc_cost")?;
	let token_1_total = token_1_balance_after + token_1_pos.frozen_balance;
	let token_1_avg_price = if token_1_total > Decimal::ZERO { checked_div(token_1_usdc_cost, token_1_total, "split token_1 avg_price")?.trunc_with_scale(8).normalize() } else { Decimal::ZERO };

	// 收集 update_id
	let mut token_0_update_id = 0i64;
	let mut token_1_update_id = 0i64;

	// 按照 keys 的排序顺序更新 position 和插入 asset_history
	for key in keys.iter() {
		let token_id = key.token_id.as_str();
		if token_id == USDC_TOKEN_ID {
			// USDC: balance -= usdc_amount
			let _update_id = update_position(
				&mut tx,
				PositionUpdate {
					user_id,
					token_id: USDC_TOKEN_ID,
					balance: usdc_balance_after,
					frozen_balance: usdc_pos.frozen_balance,
					usdc_cost: usdc_pos.usdc_cost,
					avg_price: usdc_pos.avg_price,
					redeemed: usdc_pos.redeemed,
					payout: usdc_pos.payout,
					redeemed_timestamp: None,
				},
			)
			.await?;

			// 插入 asset_history (USDC 变化)
			insert_asset_history(
				&mut tx,
				user_id,
				AssetHistoryType::Split,
				Some(-usdc_amount),
				Some(usdc_balance_before),
				Some(usdc_balance_after),
				Some(usdc_pos.frozen_balance),
				Some(usdc_pos.frozen_balance),
				None,
				None,
				None,
				None,
				None,
				None,
				Some(tx_hash.to_string()),
				None, // trade_id
				None, // order_id
			)
			.await?;
		} else if token_id == token_0_id {
			// Token_0: balance += token_0_amount, update usdc_cost
			token_0_update_id = update_position(
				&mut tx,
				PositionUpdate {
					user_id,
					token_id: token_0_id,
					balance: token_0_balance_after,
					frozen_balance: token_0_pos.frozen_balance,
					usdc_cost: Some(token_0_usdc_cost),
					avg_price: Some(token_0_avg_price),
					redeemed: token_0_pos.redeemed,
					payout: token_0_pos.payout,
					redeemed_timestamp: None,
				},
			)
			.await?;

			// 插入 asset_history (Token_0 变化)
			insert_asset_history(
				&mut tx,
				user_id,
				AssetHistoryType::Split,
				None,
				None,
				None,
				None,
				None,
				Some(token_0_id.to_string()),
				Some(token_0_amount),
				Some(token_0_balance_before),
				Some(token_0_balance_after),
				Some(token_0_pos.frozen_balance),
				Some(token_0_pos.frozen_balance),
				Some(tx_hash.to_string()),
				None, // trade_id
				None, // order_id
			)
			.await?;
		} else if token_id == token_1_id {
			// Token_1: balance += token_1_amount, update usdc_cost
			token_1_update_id = update_position(
				&mut tx,
				PositionUpdate {
					user_id,
					token_id: token_1_id,
					balance: token_1_balance_after,
					frozen_balance: token_1_pos.frozen_balance,
					usdc_cost: Some(token_1_usdc_cost),
					avg_price: Some(token_1_avg_price),
					redeemed: token_1_pos.redeemed,
					payout: token_1_pos.payout,
					redeemed_timestamp: None,
				},
			)
			.await?;

			// 插入 asset_history (Token_1 变化)
			insert_asset_history(
				&mut tx,
				user_id,
				AssetHistoryType::Split,
				None,
				None,
				None,
				None,
				None,
				Some(token_1_id.to_string()),
				Some(token_1_amount),
				Some(token_1_balance_before),
				Some(token_1_balance_after),
				Some(token_1_pos.frozen_balance),
				Some(token_1_pos.frozen_balance),
				Some(tx_hash.to_string()),
				None, // trade_id
				None, // order_id
			)
			.await?;
		}
	}

	// 插入 OperationHistory 记录
	let abs_usdc = usdc_amount.abs();
	insert_operation_history(&mut tx, event_id, market_id, user_id, AssetHistoryType::Split, None, None, Some(abs_usdc), None, Some(abs_usdc), tx_hash).await?;

	tx.commit().await?;

	info!(
		"handle_split success: user_id={}, usdc_amount={}, token_0_id={}, token_0_amount={}, token_1_id={}, token_1_amount={}, tx_hash={}",
		user_id, usdc_amount, token_0_id, token_0_amount, token_1_id, token_1_amount, tx_hash
	);

	// 推送仓位变化到 websocket
	// token_0: 判断是新创建还是更新
	if token_0_balance_before == Decimal::ZERO && token_0_pos.frozen_balance == Decimal::ZERO {
		crate::user_event::send_position_created(privy_id.to_string(), event_id, market_id, outcome_name_0.to_string(), token_0_id.to_string(), token_0_avg_price, token_0_total, token_0_update_id);
	} else {
		crate::user_event::send_position_updated(privy_id.to_string(), event_id, market_id, outcome_name_0.to_string(), token_0_id.to_string(), token_0_avg_price, token_0_total, token_0_update_id);
	}
	// token_1: 判断是新创建还是更新
	if token_1_balance_before == Decimal::ZERO && token_1_pos.frozen_balance == Decimal::ZERO {
		crate::user_event::send_position_created(privy_id.to_string(), event_id, market_id, outcome_name_1.to_string(), token_1_id.to_string(), token_1_avg_price, token_1_total, token_1_update_id);
	} else {
		crate::user_event::send_position_updated(privy_id.to_string(), event_id, market_id, outcome_name_1.to_string(), token_1_id.to_string(), token_1_avg_price, token_1_total, token_1_update_id);
	}

	Ok(())
}

/// 处理 Merge (Token0 + Token1 -> USDC)
#[allow(clippy::too_many_arguments)]
pub async fn handle_merge(
	user_id: i64,
	event_id: i64,
	market_id: i16,
	token_0_id: &str,
	token_1_id: &str,
	token_0_amount: Decimal,
	token_1_amount: Decimal,
	usdc_amount: Decimal,
	tx_hash: &str,
	privy_id: &str,
	outcome_name_0: &str,
	outcome_name_1: &str,
	pool: Option<sqlx::PgPool>,
) -> anyhow::Result<()> {
	let pool = match pool {
		Some(p) => p,
		None => get_db_pool()?,
	};
	let mut tx = pool.begin().await?;

	// 收集 LockKey 并排序
	let mut keys = vec![LockKey::new(user_id, token_0_id.to_string()), LockKey::new(user_id, token_1_id.to_string()), LockKey::new(user_id, USDC_TOKEN_ID.to_string())];
	sort_and_dedup_lock_keys(&mut keys);

	// 按排序后的顺序行锁所有资产，并存入 HashMap
	let mut positions = HashMap::new();
	for key in keys.iter() {
		let token_id = key.token_id.as_str();
		if token_id == USDC_TOKEN_ID {
			let pos = get_or_create_position(&mut tx, user_id, Some(event_id), Some(market_id), token_id, Some(privy_id.to_string()), None).await?;
			positions.insert(token_id.to_string(), pos);
		} else {
			let pos = get_position_with_lock(&mut tx, user_id, token_id).await?.ok_or_else(|| anyhow::anyhow!("Position not found for token: {}", token_id))?;
			positions.insert(token_id.to_string(), pos);
		}
	}

	// 从 HashMap 获取各个 position
	let token_0_pos = positions.get(token_0_id).ok_or_else(|| anyhow::anyhow!("Token_0 position not found"))?.clone();
	let token_1_pos = positions.get(token_1_id).ok_or_else(|| anyhow::anyhow!("Token_1 position not found"))?.clone();
	let usdc_pos = positions.get(USDC_TOKEN_ID).ok_or_else(|| anyhow::anyhow!("USDC position not found"))?.clone();

	// 不需要检查 token 余额 因为merge是链上操作 用于控制自己的私钥 merge之后冻结的资产后续会因为交易失败而解冻
	// if token_0_pos.balance < token_0_amount {
	// 	anyhow::bail!("Insufficient token_0 balance for merge");
	// }
	// if token_1_pos.balance < token_1_amount {
	// 	anyhow::bail!("Insufficient token_1 balance for merge");
	// }

	// 计算所有的中间值
	let half_usdc = checked_div(usdc_amount, Decimal::from(2), "merge usdc_amount by 2")?;

	let token_0_balance_before = token_0_pos.balance;
	let token_0_balance_after = checked_sub(token_0_balance_before, token_0_amount, "merge token_0 balance")?;
	let token_0_usdc_cost = checked_sub(token_0_pos.usdc_cost.unwrap_or(Decimal::ZERO), half_usdc, "merge token_0 usdc_cost")?;
	let token_0_total = token_0_balance_after + token_0_pos.frozen_balance;
	let token_0_avg_price = if token_0_total > Decimal::ZERO { checked_div(token_0_usdc_cost, token_0_total, "merge token_0 avg_price")?.trunc_with_scale(8).normalize() } else { Decimal::ZERO };

	let token_1_balance_before = token_1_pos.balance;
	let token_1_balance_after = checked_sub(token_1_balance_before, token_1_amount, "merge token_1 balance")?;
	let token_1_usdc_cost = checked_sub(token_1_pos.usdc_cost.unwrap_or(Decimal::ZERO), half_usdc, "merge token_1 usdc_cost")?;
	let token_1_total = token_1_balance_after + token_1_pos.frozen_balance;
	let token_1_avg_price = if token_1_total > Decimal::ZERO { checked_div(token_1_usdc_cost, token_1_total, "merge token_1 avg_price")?.trunc_with_scale(8).normalize() } else { Decimal::ZERO };

	let usdc_balance_before = usdc_pos.balance;
	let usdc_balance_after = checked_add(usdc_balance_before, usdc_amount, "merge usdc balance")?;

	// 收集 update_id
	let mut token_0_update_id = 0i64;
	let mut token_1_update_id = 0i64;

	// 按照 keys 的排序顺序更新 position 和插入 asset_history
	for key in keys.iter() {
		let token_id = key.token_id.as_str();
		if token_id == USDC_TOKEN_ID {
			// USDC: balance += usdc_amount
			let _update_id = update_position(
				&mut tx,
				PositionUpdate {
					user_id,
					token_id: USDC_TOKEN_ID,
					balance: usdc_balance_after,
					frozen_balance: usdc_pos.frozen_balance,
					usdc_cost: usdc_pos.usdc_cost,
					avg_price: usdc_pos.avg_price,
					redeemed: usdc_pos.redeemed,
					payout: usdc_pos.payout,
					redeemed_timestamp: None,
				},
			)
			.await?;

			// 插入 asset_history (USDC 变化)
			insert_asset_history(
				&mut tx,
				user_id,
				AssetHistoryType::Merge,
				Some(usdc_amount),
				Some(usdc_balance_before),
				Some(usdc_balance_after),
				Some(usdc_pos.frozen_balance),
				Some(usdc_pos.frozen_balance),
				None,
				None,
				None,
				None,
				None,
				None,
				Some(tx_hash.to_string()),
				None, // trade_id
				None, // order_id
			)
			.await?;
		} else if token_id == token_0_id {
			// Token_0: balance -= token_0_amount, update usdc_cost and avg_price
			token_0_update_id = update_position(
				&mut tx,
				PositionUpdate {
					user_id,
					token_id: token_0_id,
					balance: token_0_balance_after,
					frozen_balance: token_0_pos.frozen_balance,
					usdc_cost: Some(token_0_usdc_cost),
					avg_price: Some(token_0_avg_price),
					redeemed: token_0_pos.redeemed,
					payout: token_0_pos.payout,
					redeemed_timestamp: None,
				},
			)
			.await?;

			// 插入 asset_history (Token_0 变化)
			insert_asset_history(
				&mut tx,
				user_id,
				AssetHistoryType::Merge,
				None,
				None,
				None,
				None,
				None,
				Some(token_0_id.to_string()),
				Some(-token_0_amount),
				Some(token_0_balance_before),
				Some(token_0_balance_after),
				Some(token_0_pos.frozen_balance),
				Some(token_0_pos.frozen_balance),
				Some(tx_hash.to_string()),
				None, // trade_id
				None, // order_id
			)
			.await?;
		} else if token_id == token_1_id {
			// Token_1: balance -= token_1_amount, update usdc_cost and avg_price
			token_1_update_id = update_position(
				&mut tx,
				PositionUpdate {
					user_id,
					token_id: token_1_id,
					balance: token_1_balance_after,
					frozen_balance: token_1_pos.frozen_balance,
					usdc_cost: Some(token_1_usdc_cost),
					avg_price: Some(token_1_avg_price),
					redeemed: token_1_pos.redeemed,
					payout: token_1_pos.payout,
					redeemed_timestamp: None,
				},
			)
			.await?;

			// 插入 asset_history (Token_1 变化)
			insert_asset_history(
				&mut tx,
				user_id,
				AssetHistoryType::Merge,
				None,
				None,
				None,
				None,
				None,
				Some(token_1_id.to_string()),
				Some(-token_1_amount),
				Some(token_1_balance_before),
				Some(token_1_balance_after),
				Some(token_1_pos.frozen_balance),
				Some(token_1_pos.frozen_balance),
				Some(tx_hash.to_string()),
				None, // trade_id
				None, // order_id
			)
			.await?;
		}
	}

	// 插入 OperationHistory 记录
	let abs_usdc = usdc_amount.abs();
	insert_operation_history(&mut tx, event_id, market_id, user_id, AssetHistoryType::Merge, None, None, Some(abs_usdc), None, Some(abs_usdc), tx_hash).await?;

	tx.commit().await?;

	info!(
		"handle_merge success: user_id={}, token_0_id={}, token_0_amount={}, token_1_id={}, token_1_amount={}, usdc_amount={}, tx_hash={}",
		user_id, token_0_id, token_0_amount, token_1_id, token_1_amount, usdc_amount, tx_hash
	);

	// Websocket push for position changes
	// Token 0: check if position was removed or updated
	let token_0_before_total = token_0_balance_before + token_0_pos.frozen_balance;
	if token_0_total == Decimal::ZERO && token_0_before_total > Decimal::ZERO {
		crate::user_event::send_position_removed(privy_id.to_string(), event_id, market_id, token_0_id.to_string(), token_0_update_id);
	} else if token_0_total != token_0_before_total {
		crate::user_event::send_position_updated(privy_id.to_string(), event_id, market_id, outcome_name_0.to_string(), token_0_id.to_string(), token_0_avg_price, token_0_total, token_0_update_id);
	}

	// Token 1: check if position was removed or updated
	let token_1_before_total = token_1_balance_before + token_1_pos.frozen_balance;
	if token_1_total == Decimal::ZERO && token_1_before_total > Decimal::ZERO {
		crate::user_event::send_position_removed(privy_id.to_string(), event_id, market_id, token_1_id.to_string(), token_1_update_id);
	} else if token_1_total != token_1_before_total {
		crate::user_event::send_position_updated(privy_id.to_string(), event_id, market_id, outcome_name_1.to_string(), token_1_id.to_string(), token_1_avg_price, token_1_total, token_1_update_id);
	}

	Ok(())
}

/// 处理 Redeem
#[allow(clippy::too_many_arguments)]
pub async fn handle_redeem(
	user_id: i64,
	event_id: i64,
	market_id: i16,
	token_id_0: &str,
	token_id_1: &str,
	usdc_amount: Decimal,
	tx_hash: &str,
	privy_id: &str,
	outcome_name_0: &str,
	outcome_name_1: &str,
	pool: Option<sqlx::PgPool>,
) -> anyhow::Result<()> {
	let pool = match pool {
		Some(p) => p,
		None => get_db_pool()?,
	};
	let mut tx = pool.begin().await?;

	// 收集 LockKey 并排序
	let mut keys = vec![LockKey::new(user_id, token_id_0.to_string()), LockKey::new(user_id, token_id_1.to_string()), LockKey::new(user_id, USDC_TOKEN_ID.to_string())];
	sort_and_dedup_lock_keys(&mut keys);

	// 按排序后的顺序行锁所有资产，并存入 HashMap
	let mut positions = HashMap::new();
	for key in keys.iter() {
		let token_id = key.token_id.as_str();
		let pos = if token_id == USDC_TOKEN_ID {
			get_or_create_position(&mut tx, user_id, Some(event_id), Some(market_id), token_id, Some(privy_id.to_string()), None).await?
		} else if token_id == token_id_0 {
			get_or_create_position(&mut tx, user_id, Some(event_id), Some(market_id), token_id, Some(privy_id.to_string()), Some(outcome_name_0.to_string())).await?
		} else {
			get_or_create_position(&mut tx, user_id, Some(event_id), Some(market_id), token_id, Some(privy_id.to_string()), Some(outcome_name_1.to_string())).await?
		};
		positions.insert(token_id.to_string(), pos);
	}

	// 从 HashMap 获取各个 position
	let token_0_pos = positions.get(token_id_0).ok_or_else(|| anyhow::anyhow!("Token_0 position not found"))?.clone();
	let token_1_pos = positions.get(token_id_1).ok_or_else(|| anyhow::anyhow!("Token_1 position not found"))?.clone();
	let usdc_pos = positions.get(USDC_TOKEN_ID).ok_or_else(|| anyhow::anyhow!("USDC position not found"))?.clone();

	// 收集 update_id
	let mut token_0_update_id = 0i64;
	let mut token_1_update_id = 0i64;

	// 按照 keys 的排序顺序更新 token position
	for key in keys.iter() {
		let token_id = key.token_id.as_str();
		if token_id == USDC_TOKEN_ID {
			// USDC 稍后处理
			// 如果 usdc_amount > 0，更新 USDC position 并插入 asset_history
			if usdc_amount > Decimal::ZERO {
				let usdc_balance_before = usdc_pos.balance;
				let usdc_balance_after = checked_add(usdc_balance_before, usdc_amount, "redeem usdc balance")?;

				// 更新 USDC position
				let _update_id = update_position(
					&mut tx,
					PositionUpdate {
						user_id,
						token_id: USDC_TOKEN_ID,
						balance: usdc_balance_after,
						frozen_balance: usdc_pos.frozen_balance,
						usdc_cost: usdc_pos.usdc_cost,
						avg_price: usdc_pos.avg_price,
						redeemed: usdc_pos.redeemed,
						payout: usdc_pos.payout,
						redeemed_timestamp: None,
					},
				)
				.await?;

				// 插入 asset_history (只记录 USDC 变化，token 部分全为 None)
				insert_asset_history(
					&mut tx,
					user_id,
					AssetHistoryType::Redeem,
					Some(usdc_amount),
					Some(usdc_balance_before),
					Some(usdc_balance_after),
					Some(usdc_pos.frozen_balance),
					Some(usdc_pos.frozen_balance),
					None,
					None,
					None,
					None,
					None,
					None,
					Some(tx_hash.to_string()),
					None, // trade_id
					None, // order_id
				)
				.await?;
			}
		} else if token_id == token_id_0 {
			// Token_0: 只更新 redeemed、payout 和 redeemed_timestamp，其他字段保持不变
			let now = Utc::now();
			token_0_update_id = update_position(
				&mut tx,
				PositionUpdate {
					user_id,
					token_id: token_id_0,
					balance: token_0_pos.balance,
					frozen_balance: token_0_pos.frozen_balance,
					usdc_cost: token_0_pos.usdc_cost,
					avg_price: token_0_pos.avg_price,
					redeemed: Some(true),
					payout: Some(usdc_amount),
					redeemed_timestamp: Some(now),
				},
			)
			.await?;
		} else if token_id == token_id_1 {
			// Token_1: 只更新 redeemed、payout 和 redeemed_timestamp，其他字段保持不变
			let now = Utc::now();
			token_1_update_id = update_position(
				&mut tx,
				PositionUpdate {
					user_id,
					token_id: token_id_1,
					balance: token_1_pos.balance,
					frozen_balance: token_1_pos.frozen_balance,
					usdc_cost: token_1_pos.usdc_cost,
					avg_price: token_1_pos.avg_price,
					redeemed: Some(true),
					payout: Some(usdc_amount),
					redeemed_timestamp: Some(now),
				},
			)
			.await?;
		}
	}

	// 插入 OperationHistory 记录
	let abs_usdc = usdc_amount.abs();
	insert_operation_history(&mut tx, event_id, market_id, user_id, AssetHistoryType::Redeem, None, None, Some(abs_usdc), None, Some(abs_usdc), tx_hash).await?;

	tx.commit().await?;

	info!("handle_redeem success: user_id={}, token_0_id={}, token_1_id={}, usdc_amount={}, tx_hash={}", user_id, token_id_0, token_id_1, usdc_amount, tx_hash);

	// Websocket push for position changes - redeem closes both positions
	// Send PositionRemoved for both tokens since they are redeemed
	let token_0_total = token_0_pos.balance + token_0_pos.frozen_balance;
	let token_1_total = token_1_pos.balance + token_1_pos.frozen_balance;
	if token_0_total > Decimal::ZERO {
		crate::user_event::send_position_removed(privy_id.to_string(), event_id, market_id, token_id_0.to_string(), token_0_update_id);
	}
	if token_1_total > Decimal::ZERO {
		crate::user_event::send_position_removed(privy_id.to_string(), event_id, market_id, token_id_1.to_string(), token_1_update_id);
	}

	Ok(())
}

/// TakerTradeInfo 结构
pub struct TakerTradeInfo {
	pub taker_id: i64,
	pub taker_order_id: Uuid,
	pub taker_order_side: OrderSide,
	pub taker_token_id: String,
	pub taker_usdc_amount: Decimal,
	pub taker_token_amount: Decimal,
}

/// MakerTradeInfo 结构
pub struct MakerTradeInfo {
	pub maker_id: i64,
	pub maker_order_id: Uuid,
	pub maker_order_side: OrderSide,
	pub maker_token_id: String,
	pub maker_usdc_amount: Decimal,
	pub maker_token_amount: Decimal,
	pub maker_price: Decimal,
}

/// handle_trade: 处理成交记录，插入 trades 表，更新 orders 表，返回签名信息
/// 返回顺序：首先是 taker 的 SignatureOrderMsg，然后是各个 maker 的 SignatureOrderMsg
pub async fn handle_trade(
	trade_id: Uuid,
	timestamp: i64,
	event_id: i64,
	market_id: i16,
	taker: TakerTradeInfo,
	makers: Vec<MakerTradeInfo>,
	pool: Option<sqlx::PgPool>,
) -> anyhow::Result<(Vec<SignatureOrderMsg>, Vec<proto::OrderUpdateId>)> {
	let pool = match pool {
		Some(p) => p,
		None => get_db_pool()?,
	};
	let mut tx = pool.begin().await?;
	let match_timestamp = chrono::DateTime::from_timestamp_millis(timestamp).ok_or_else(|| anyhow::anyhow!("Invalid timestamp"))?;

	let mut order_filled_map: HashMap<Uuid, Decimal> = HashMap::new(); // order_id -> total_filled_quantity

	// 0. 计算 trade_volume = taker_usdc_amount + sum(maker_usdc_amount where maker side == taker side)
	let mut trade_volume = taker.taker_usdc_amount;
	for maker in &makers {
		if maker.maker_order_side == taker.taker_order_side {
			trade_volume = checked_add(trade_volume, maker.maker_usdc_amount, "trade_volume")?;
		}
	}

	// 1. 插入 taker trade 记录并累积成交数量
	let taker_price = checked_div(taker.taker_usdc_amount, taker.taker_token_amount, "taker avg_price")?.trunc_with_scale(8).normalize();
	sqlx::query(
		"INSERT INTO trades (
			batch_id, match_timestamp, order_id, user_id, event_id, market_id, token_id,
			side, taker, trade_volume, avg_price, usdc_amount, token_amount, fee, real_amount, onchain_send_handled, tx_hash
		) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
	)
	.bind(trade_id)
	.bind(match_timestamp)
	.bind(taker.taker_order_id)
	.bind(taker.taker_id)
	.bind(event_id)
	.bind(market_id)
	.bind(&taker.taker_token_id)
	.bind(taker.taker_order_side)
	.bind(true) // taker = true
	.bind(trade_volume)
	.bind(taker_price)
	.bind(taker.taker_usdc_amount)
	.bind(taker.taker_token_amount)
	.bind(None::<Decimal>) // fee 暂时为 None
	.bind(Decimal::ZERO) // real_amount 在 onchain_send_result 时更新
	.bind(false)
	.bind(None::<String>)
	.execute(tx.as_mut())
	.await?;

	*order_filled_map.entry(taker.taker_order_id).or_insert(Decimal::ZERO) =
		checked_add(*order_filled_map.get(&taker.taker_order_id).unwrap_or(&Decimal::ZERO), taker.taker_token_amount, "taker filled quantity")?;

	// 2. 插入所有 maker trade 记录并累积成交数量
	for maker in &makers {
		sqlx::query(
			"INSERT INTO trades (
				batch_id, match_timestamp, order_id, user_id, event_id, market_id, token_id,
				side, taker, trade_volume, avg_price, usdc_amount, token_amount, fee, real_amount, onchain_send_handled, tx_hash
			) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17)",
		)
		.bind(trade_id)
		.bind(match_timestamp)
		.bind(maker.maker_order_id)
		.bind(maker.maker_id)
		.bind(event_id)
		.bind(market_id)
		.bind(&maker.maker_token_id)
		.bind(maker.maker_order_side)
		.bind(false) // taker = false
		.bind(Decimal::ZERO) //只有taker
		.bind(maker.maker_price)
		.bind(maker.maker_usdc_amount)
		.bind(maker.maker_token_amount)
		.bind(None::<Decimal>)
		.bind(Decimal::ZERO)
		.bind(false)
		.bind(None::<String>)
		.execute(tx.as_mut())
		.await?;

		*order_filled_map.entry(maker.maker_order_id).or_insert(Decimal::ZERO) =
			checked_add(*order_filled_map.get(&maker.maker_order_id).unwrap_or(&Decimal::ZERO), maker.maker_token_amount, "maker filled quantity")?;
	}

	// 3. 更新 orders 的 filled_quantity 和 status，同时返回 signature_order_msg
	// 为了防止死锁，需要按照订单 ID 排序后依次加锁更新

	// 收集所有订单 ID 并排序
	let mut order_ids: Vec<Uuid> = vec![taker.taker_order_id];
	order_ids.extend(makers.iter().map(|m| m.maker_order_id));
	order_ids.sort();

	// 存储每个订单的签名信息和 update_id
	let mut order_signatures: HashMap<Uuid, SignatureOrderMsg> = HashMap::new();
	let mut order_update_ids: HashMap<Uuid, i64> = HashMap::new();

	// 按照排序后的订单 ID 依次加锁并更新
	for order_id in order_ids {
		let filled_qty = order_filled_map.get(&order_id).unwrap_or(&Decimal::ZERO);

		// 使用 FOR UPDATE 加行锁查询订单状态（查询所有必要字段，包括市价单字段）
		let order_info = sqlx::query(
			"SELECT status, order_type, order_side, filled_quantity, quantity, volume,
			        market_filled_quantity, market_filled_volume, market_quantity, market_volume,
			        signature_order_msg
			 FROM orders WHERE id = $1 FOR UPDATE",
		)
		.bind(order_id)
		.fetch_one(tx.as_mut())
		.await?;

		let current_status: OrderStatus = order_info.try_get("status")?;
		let order_type: common::engine_types::OrderType = order_info.try_get("order_type")?;
		let order_side: OrderSide = order_info.try_get("order_side")?;

		// 获取 signature_order_msg (作为 JSONB 处理)
		let signature_json: sqlx::types::JsonValue = order_info.try_get("signature_order_msg")?;
		let signature_msg: SignatureOrderMsg = serde_json::from_value(signature_json)?;
		order_signatures.insert(order_id, signature_msg);

		// 判断是否是taker的市价单
		let is_taker_market_order = order_id == taker.taker_order_id && order_type == common::engine_types::OrderType::Market;

		// 根据订单类型计算新状态
		let new_status = if is_taker_market_order {
			// Taker 市价单：根据买卖方向判断状态
			match order_side {
				OrderSide::Buy => {
					// 市价买单：基于 market_filled_volume 判断
					let current_filled: Decimal = order_info.try_get("market_filled_volume")?;
					let target: Decimal = order_info.try_get("market_volume")?;
					let new_filled = checked_add(current_filled, taker.taker_usdc_amount, "new market_filled_volume")?;
					match current_status {
						OrderStatus::Cancelled | OrderStatus::Rejected | OrderStatus::Filled => current_status,
						_ => {
							if new_filled >= target {
								OrderStatus::Filled
							} else if current_status == OrderStatus::New {
								OrderStatus::PartiallyFilled
							} else {
								current_status
							}
						}
					}
				}
				OrderSide::Sell => {
					// 市价卖单：基于 market_filled_quantity 判断
					let current_filled: Decimal = order_info.try_get("market_filled_quantity")?;
					let target: Decimal = order_info.try_get("market_quantity")?;
					let new_filled = checked_add(current_filled, taker.taker_token_amount, "new market_filled_quantity")?;
					match current_status {
						OrderStatus::Cancelled | OrderStatus::Rejected | OrderStatus::Filled => current_status,
						_ => {
							if new_filled >= target {
								OrderStatus::Filled
							} else if current_status == OrderStatus::New {
								OrderStatus::PartiallyFilled
							} else {
								current_status
							}
						}
					}
				}
			}
		} else {
			// 限价单（包括 maker 订单和 taker 限价单）
			let current_filled: Decimal = order_info.try_get("filled_quantity")?;
			let target: Decimal = order_info.try_get("quantity")?;
			let new_filled = checked_add(current_filled, *filled_qty, "new filled_quantity")?;
			match current_status {
				OrderStatus::Cancelled | OrderStatus::Rejected | OrderStatus::Filled => current_status,
				_ => {
					if new_filled >= target {
						OrderStatus::Filled
					} else if current_status == OrderStatus::New {
						OrderStatus::PartiallyFilled
					} else {
						current_status
					}
				}
			}
		};

		// 根据订单类型更新订单
		let update_id: (i64,) = if is_taker_market_order {
			// 市价单：同时更新 market_filled_volume 和 market_filled_quantity
			sqlx::query_as(
				"UPDATE orders
				 SET market_filled_volume = market_filled_volume + $1,
				     market_filled_quantity = market_filled_quantity + $2,
				     status = $3,
				     updated_at = $4,
				     update_id = update_id + 1
				 WHERE id = $5
				 RETURNING update_id",
			)
			.bind(taker.taker_usdc_amount)
			.bind(taker.taker_token_amount)
			.bind(new_status)
			.bind(Utc::now())
			.bind(order_id)
			.fetch_one(tx.as_mut())
			.await?
		} else {
			// 限价单：只更新 filled_quantity
			sqlx::query_as(
				"UPDATE orders
				 SET filled_quantity = filled_quantity + $1,
				     status = $2,
				     updated_at = $3,
				     update_id = update_id + 1
				 WHERE id = $4
				 RETURNING update_id",
			)
			.bind(filled_qty)
			.bind(new_status)
			.bind(Utc::now())
			.bind(order_id)
			.fetch_one(tx.as_mut())
			.await?
		};

		order_update_ids.insert(order_id, update_id.0);
	}

	// 按照 taker 在前，makers 在后的顺序构建返回列表
	let mut signature_order_msgs = Vec::new();
	let mut update_ids = Vec::new();

	// Taker
	signature_order_msgs.push(order_signatures.remove(&taker.taker_order_id).ok_or_else(|| anyhow::anyhow!("Taker signature not found"))?);
	let taker_update_id = order_update_ids.get(&taker.taker_order_id).copied().ok_or_else(|| anyhow::anyhow!("Taker update_id not found"))?;
	update_ids.push(proto::OrderUpdateId { order_id: taker.taker_order_id.to_string(), update_id: taker_update_id });

	// Makers
	for maker in &makers {
		if let Some(sig) = order_signatures.remove(&maker.maker_order_id) {
			signature_order_msgs.push(sig);
		}
		if let Some(update_id) = order_update_ids.get(&maker.maker_order_id) {
			update_ids.push(proto::OrderUpdateId { order_id: maker.maker_order_id.to_string(), update_id: *update_id });
		}
	}

	tx.commit().await?;

	info!("handle_trade success: trade_id={}, taker_id={}, makers_count={}", trade_id, taker.taker_id, makers.len());

	Ok((signature_order_msgs, update_ids))
}

/// handle_trade_onchain_send_result: 处理链上交易结果，更新资产和 position
pub async fn handle_trade_onchain_send_result(params: TradeOnchainSendParams<'_>) -> anyhow::Result<()> {
	// 解构参数
	let TradeOnchainSendParams {
		trade_id,
		event_id,
		market_id,
		taker_side,
		taker_user_id,
		taker_usdc_amount,
		taker_token_amount: _taker_token_amount,
		taker_token_id,
		taker_order_id,
		real_taker_usdc_amount,
		real_taker_token_amount,
		taker_unfreeze_amount,
		taker_privy_user_id,
		taker_outcome_name,
		maker_infos,
		tx_hash,
		success,
		pool,
	} = params;

	// 构建 user_id -> privy_id 映射
	let mut user_privy_map: HashMap<i64, String> = HashMap::new();
	user_privy_map.insert(taker_user_id, taker_privy_user_id.to_string());
	for maker_info in &maker_infos {
		user_privy_map.insert(maker_info.maker_user_id, maker_info.maker_privy_user_id.clone());
	}

	// 构建 token_id -> outcome_name 映射
	let mut token_outcome_map: HashMap<String, String> = HashMap::new();
	token_outcome_map.insert(taker_token_id.to_string(), taker_outcome_name.to_string());
	for maker_info in &maker_infos {
		token_outcome_map.insert(maker_info.maker_token_id.clone(), maker_info.maker_outcome_name.clone());
	}

	// 用于收集成功交易的仓位更新信息
	let mut position_events: Vec<PositionEvent> = Vec::new();

	// 判断链上交易是否成功：tx_hash 不为空且 success 为 true
	let is_success = !tx_hash.is_empty() && success;

	let pool = match pool {
		Some(p) => p,
		None => get_db_pool()?,
	};
	let mut tx = pool.begin().await?;

	// 1. 更新 trades 表（仅在成功时）
	if is_success {
		// 链上交易成功：批量更新 batch_id 等于 trade_id 的记录，更新 tx_hash 和 onchain_send_handled
		sqlx::query("UPDATE trades SET tx_hash = $1, onchain_send_handled = $2 WHERE batch_id = $3").bind(tx_hash).bind(success).bind(trade_id).execute(tx.as_mut()).await?;
	}

	// 2. 收集所有需要更新的 position 并按 (user_id, token_id) 排序
	let mut lock_keys = Vec::new();

	if is_success {
		// 链上交易成功：更新 position 和 asset_history
		lock_keys.push(LockKey::new(taker_user_id, USDC_TOKEN_ID.to_string()));
		lock_keys.push(LockKey::new(taker_user_id, taker_token_id.to_string()));

		for maker_info in &maker_infos {
			lock_keys.push(LockKey::new(maker_info.maker_user_id, USDC_TOKEN_ID.to_string()));
			lock_keys.push(LockKey::new(maker_info.maker_user_id, maker_info.maker_token_id.clone()));
		}
	} else {
		// 链上交易失败：只解冻资产
		match taker_side {
			OrderSide::Buy => {
				lock_keys.push(LockKey::new(taker_user_id, USDC_TOKEN_ID.to_string()));
			}
			OrderSide::Sell => {
				lock_keys.push(LockKey::new(taker_user_id, taker_token_id.to_string()));
			}
		}

		for maker_info in &maker_infos {
			match maker_info.maker_side {
				OrderSide::Buy => {
					lock_keys.push(LockKey::new(maker_info.maker_user_id, USDC_TOKEN_ID.to_string()));
				}
				OrderSide::Sell => {
					lock_keys.push(LockKey::new(maker_info.maker_user_id, maker_info.maker_token_id.clone()));
				}
			}
		}
	}

	// 去重并排序
	sort_and_dedup_lock_keys(&mut lock_keys);

	// 按顺序加锁
	let mut positions_map: HashMap<(i64, String), Positions> = HashMap::new();
	for key in lock_keys.iter() {
		let user_id = key.user_id.parse::<i64>()?;
		let token_id = key.token_id.as_str();

		// 从映射中获取 privy_id 和 outcome_name
		let privy_id_opt = user_privy_map.get(&user_id).map(|s| s.to_string());
		let outcome_name_opt = token_outcome_map.get(token_id).map(|s| s.to_string());

		let pos = get_or_create_position(&mut tx, user_id, Some(event_id), Some(market_id), token_id, privy_id_opt, outcome_name_opt).await?;
		positions_map.insert((user_id, token_id.to_string()), pos);
	}

	// 3. 处理 taker
	if is_success {
		// 链上交易成功
		match taker_side {
			OrderSide::Buy => {
				// Taker 买：USDC frozen_balance 减少，token balance 增加
				let usdc_pos = positions_map.get(&(taker_user_id, USDC_TOKEN_ID.to_string())).ok_or_else(|| anyhow::anyhow!("USDC position not found"))?;
				let usdc_balance_before = usdc_pos.balance;
				let usdc_frozen_before = usdc_pos.frozen_balance;

				let new_usdc_frozen = checked_sub(usdc_frozen_before, taker_unfreeze_amount, "taker usdc frozen")?;

				let _usdc_update_id = update_position(
					&mut tx,
					PositionUpdate {
						user_id: taker_user_id,
						token_id: USDC_TOKEN_ID,
						balance: usdc_balance_before,
						frozen_balance: new_usdc_frozen,
						usdc_cost: usdc_pos.usdc_cost,
						avg_price: usdc_pos.avg_price,
						redeemed: None,
						payout: None,
						redeemed_timestamp: None,
					},
				)
				.await?;

				// Token 增加
				let token_pos = positions_map.get(&(taker_user_id, taker_token_id.to_string())).ok_or_else(|| anyhow::anyhow!("Token position not found"))?;
				let token_balance_before = token_pos.balance;
				let token_frozen_before = token_pos.frozen_balance;

				let new_token_balance = checked_add(token_balance_before, real_taker_token_amount, "taker token balance")?;
				let new_usdc_cost = checked_add(token_pos.usdc_cost.unwrap_or(Decimal::ZERO), taker_usdc_amount, "taker usdc_cost")?;
				let total = checked_add(new_token_balance, token_frozen_before, "token total")?;
				let new_avg_price = if total > Decimal::ZERO { checked_div(new_usdc_cost, total, "avg_price")?.trunc_with_scale(8).normalize() } else { Decimal::ZERO };

				let token_update_id = update_position(
					&mut tx,
					PositionUpdate {
						user_id: taker_user_id,
						token_id: taker_token_id,
						balance: new_token_balance,
						frozen_balance: token_frozen_before,
						usdc_cost: Some(new_usdc_cost),
						avg_price: Some(new_avg_price),
						redeemed: None,
						payout: None,
						redeemed_timestamp: None,
					},
				)
				.await?;

				// 插入 asset_history
				insert_asset_history_struct(
					&mut tx,
					AssetHistoryInsert {
						user_id: taker_user_id,
						history_type: AssetHistoryType::OnChainBuySuccess,
						usdc_amount: Some(-taker_unfreeze_amount),
						usdc_balance_before: Some(usdc_balance_before),
						usdc_balance_after: Some(usdc_balance_before),
						usdc_frozen_balance_before: Some(usdc_frozen_before),
						usdc_frozen_balance_after: Some(new_usdc_frozen),
						token_id: Some(taker_token_id),
						token_amount: Some(real_taker_token_amount),
						token_balance_before: Some(token_balance_before),
						token_balance_after: Some(new_token_balance),
						token_frozen_balance_before: Some(token_frozen_before),
						token_frozen_balance_after: Some(token_frozen_before),
						tx_hash: Some(tx_hash),
						trade_id: Some(trade_id.to_string()),
						order_id: Some(taker_order_id.to_string()),
					},
				)
				.await?;

				// 插入 OperationHistory (taker 买成功)
				let taker_price =
					if real_taker_token_amount > Decimal::ZERO { checked_div(real_taker_usdc_amount, real_taker_token_amount, "taker price")?.trunc_with_scale(8).normalize() } else { Decimal::ZERO };
				insert_operation_history(
					&mut tx,
					event_id,
					market_id,
					taker_user_id,
					AssetHistoryType::OnChainBuySuccess,
					Some(taker_outcome_name.to_string()),
					Some(taker_token_id.to_string()),
					Some(real_taker_token_amount),
					Some(taker_price),
					Some(real_taker_usdc_amount),
					tx_hash,
				)
				.await?;

				// 收集仓位更新事件
				position_events.push(PositionEvent {
					event_type: PositionEventType::Updated,
					privy_id: taker_privy_user_id.to_string(),
					event_id,
					market_id,
					outcome_name: taker_outcome_name.to_string(),
					token_id: taker_token_id.to_string(),
					quantity: total,
					avg_price: new_avg_price,
					update_id: token_update_id,
				});
			}
			OrderSide::Sell => {
				// Taker 卖：token frozen_balance 减少，USDC balance 增加
				let token_pos = positions_map.get(&(taker_user_id, taker_token_id.to_string())).ok_or_else(|| anyhow::anyhow!("Token position not found"))?;
				let token_balance_before = token_pos.balance;
				let token_frozen_before = token_pos.frozen_balance;

				let new_token_frozen = checked_sub(token_frozen_before, taker_unfreeze_amount, "taker token frozen")?;
				let new_usdc_cost = checked_sub(token_pos.usdc_cost.unwrap_or(Decimal::ZERO), taker_usdc_amount, "taker usdc_cost")?;
				let total = checked_add(token_balance_before, new_token_frozen, "token total")?;
				let new_avg_price = if total > Decimal::ZERO { checked_div(new_usdc_cost, total, "avg_price")?.trunc_with_scale(8).normalize() } else { Decimal::ZERO };

				let token_update_id = update_position(
					&mut tx,
					PositionUpdate {
						user_id: taker_user_id,
						token_id: taker_token_id,
						balance: token_balance_before,
						frozen_balance: new_token_frozen,
						usdc_cost: Some(new_usdc_cost),
						avg_price: Some(new_avg_price),
						redeemed: None,
						payout: None,
						redeemed_timestamp: None,
					},
				)
				.await?;

				// USDC 增加
				let usdc_pos = positions_map.get(&(taker_user_id, USDC_TOKEN_ID.to_string())).ok_or_else(|| anyhow::anyhow!("USDC position not found"))?;
				let usdc_balance_before = usdc_pos.balance;
				let usdc_frozen_before = usdc_pos.frozen_balance;

				let new_usdc_balance = checked_add(usdc_balance_before, real_taker_usdc_amount, "taker usdc balance")?;

				let _usdc_update_id = update_position(
					&mut tx,
					PositionUpdate {
						user_id: taker_user_id,
						token_id: USDC_TOKEN_ID,
						balance: new_usdc_balance,
						frozen_balance: usdc_frozen_before,
						usdc_cost: usdc_pos.usdc_cost,
						avg_price: usdc_pos.avg_price,
						redeemed: None,
						payout: None,
						redeemed_timestamp: None,
					},
				)
				.await?;

				// 插入 asset_history
				insert_asset_history_struct(
					&mut tx,
					AssetHistoryInsert {
						user_id: taker_user_id,
						history_type: AssetHistoryType::OnChainSellSuccess,
						usdc_amount: Some(real_taker_usdc_amount),
						usdc_balance_before: Some(usdc_balance_before),
						usdc_balance_after: Some(new_usdc_balance),
						usdc_frozen_balance_before: Some(usdc_frozen_before),
						usdc_frozen_balance_after: Some(usdc_frozen_before),
						token_id: Some(taker_token_id),
						token_amount: Some(-taker_unfreeze_amount),
						token_balance_before: Some(token_balance_before),
						token_balance_after: Some(token_balance_before),
						token_frozen_balance_before: Some(token_frozen_before),
						token_frozen_balance_after: Some(new_token_frozen),
						tx_hash: Some(tx_hash),
						trade_id: Some(trade_id.to_string()),
						order_id: Some(taker_order_id.to_string()),
					},
				)
				.await?;

				// 插入 OperationHistory (taker 卖成功)
				let taker_price =
					if real_taker_token_amount > Decimal::ZERO { checked_div(real_taker_usdc_amount, real_taker_token_amount, "taker price")?.trunc_with_scale(8).normalize() } else { Decimal::ZERO };
				insert_operation_history(
					&mut tx,
					event_id,
					market_id,
					taker_user_id,
					AssetHistoryType::OnChainSellSuccess,
					Some(taker_outcome_name.to_string()),
					Some(taker_token_id.to_string()),
					Some(real_taker_token_amount),
					Some(taker_price),
					Some(real_taker_usdc_amount),
					tx_hash,
				)
				.await?;

				// 收集仓位更新事件
				position_events.push(PositionEvent {
					event_type: PositionEventType::Updated,
					privy_id: taker_privy_user_id.to_string(),
					event_id,
					market_id,
					outcome_name: taker_outcome_name.to_string(),
					token_id: taker_token_id.to_string(),
					quantity: total,
					avg_price: new_avg_price,
					update_id: token_update_id,
				});
			}
		}
	} else {
		// 链上交易失败
		match taker_side {
			OrderSide::Buy => {
				// Taker 买失败：USDC balance 增加，frozen_balance 减少
				let usdc_pos = positions_map.get(&(taker_user_id, USDC_TOKEN_ID.to_string())).ok_or_else(|| anyhow::anyhow!("USDC position not found"))?;
				let usdc_balance_before = usdc_pos.balance;
				let usdc_frozen_before = usdc_pos.frozen_balance;

				let new_usdc_balance = checked_add(usdc_balance_before, taker_unfreeze_amount, "taker usdc balance")?;
				let new_usdc_frozen = checked_sub(usdc_frozen_before, taker_unfreeze_amount, "taker usdc frozen")?;

				let _usdc_update_id = update_position(
					&mut tx,
					PositionUpdate {
						user_id: taker_user_id,
						token_id: USDC_TOKEN_ID,
						balance: new_usdc_balance,
						frozen_balance: new_usdc_frozen,
						usdc_cost: usdc_pos.usdc_cost,
						avg_price: usdc_pos.avg_price,
						redeemed: None,
						payout: None,
						redeemed_timestamp: None,
					},
				)
				.await?;

				// 插入 asset_history
				insert_asset_history_struct(
					&mut tx,
					AssetHistoryInsert {
						user_id: taker_user_id,
						history_type: AssetHistoryType::OnChainBuyFailed,
						usdc_amount: Some(taker_unfreeze_amount),
						usdc_balance_before: Some(usdc_balance_before),
						usdc_balance_after: Some(new_usdc_balance),
						usdc_frozen_balance_before: Some(usdc_frozen_before),
						usdc_frozen_balance_after: Some(new_usdc_frozen),
						token_id: None,
						token_amount: None,
						token_balance_before: None,
						token_balance_after: None,
						token_frozen_balance_before: None,
						token_frozen_balance_after: None,
						tx_hash: None,
						trade_id: Some(trade_id.to_string()),
						order_id: Some(taker_order_id.to_string()),
					},
				)
				.await?;
			}
			OrderSide::Sell => {
				// Taker 卖失败：token balance 增加，frozen_balance 减少
				let token_pos = positions_map.get(&(taker_user_id, taker_token_id.to_string())).ok_or_else(|| anyhow::anyhow!("Token position not found"))?;
				let token_balance_before = token_pos.balance;
				let token_frozen_before = token_pos.frozen_balance;

				let new_token_balance = checked_add(token_balance_before, taker_unfreeze_amount, "taker token balance")?;
				let new_token_frozen = checked_sub(token_frozen_before, taker_unfreeze_amount, "taker token frozen")?;

				let _token_update_id = update_position(
					&mut tx,
					PositionUpdate {
						user_id: taker_user_id,
						token_id: taker_token_id,
						balance: new_token_balance,
						frozen_balance: new_token_frozen,
						usdc_cost: token_pos.usdc_cost,
						avg_price: token_pos.avg_price,
						redeemed: None,
						payout: None,
						redeemed_timestamp: None,
					},
				)
				.await?;

				// 插入 asset_history
				insert_asset_history_struct(
					&mut tx,
					AssetHistoryInsert {
						user_id: taker_user_id,
						history_type: AssetHistoryType::OnChainSellFailed,
						usdc_amount: None,
						usdc_balance_before: None,
						usdc_balance_after: None,
						usdc_frozen_balance_before: None,
						usdc_frozen_balance_after: None,
						token_id: Some(taker_token_id),
						token_amount: Some(taker_unfreeze_amount),
						token_balance_before: Some(token_balance_before),
						token_balance_after: Some(new_token_balance),
						token_frozen_balance_before: Some(token_frozen_before),
						token_frozen_balance_after: Some(new_token_frozen),
						tx_hash: None,
						trade_id: Some(trade_id.to_string()),
						order_id: Some(taker_order_id.to_string()),
					},
				)
				.await?;
			}
		}
	}
	// 4. 处理每个 maker
	for maker_info in &maker_infos {
		if is_success {
			// 链上交易成功
			match maker_info.maker_side {
				OrderSide::Buy => {
					// Maker 买：USDC frozen_balance 减少，token balance 增加
					let usdc_pos = positions_map.get(&(maker_info.maker_user_id, USDC_TOKEN_ID.to_string())).ok_or_else(|| anyhow::anyhow!("Maker USDC position not found"))?;
					let usdc_balance_before = usdc_pos.balance;
					let usdc_frozen_before = usdc_pos.frozen_balance;

					let new_usdc_frozen = checked_sub(usdc_frozen_before, maker_info.maker_usdc_amount, "maker usdc frozen")?;

					let _usdc_update_id = update_position(
						&mut tx,
						PositionUpdate {
							user_id: maker_info.maker_user_id,
							token_id: USDC_TOKEN_ID,
							balance: usdc_balance_before,
							frozen_balance: new_usdc_frozen,
							usdc_cost: usdc_pos.usdc_cost,
							avg_price: usdc_pos.avg_price,
							redeemed: None,
							payout: None,
							redeemed_timestamp: None,
						},
					)
					.await?;

					// Token 增加
					let token_pos = positions_map.get(&(maker_info.maker_user_id, maker_info.maker_token_id.clone())).ok_or_else(|| anyhow::anyhow!("Maker token position not found"))?;
					let token_balance_before = token_pos.balance;
					let token_frozen_before = token_pos.frozen_balance;

					let new_token_balance = checked_add(token_balance_before, maker_info.real_maker_token_amount, "maker token balance")?;
					let new_usdc_cost = checked_add(token_pos.usdc_cost.unwrap_or(Decimal::ZERO), maker_info.maker_usdc_amount, "maker usdc_cost")?;
					let total = checked_add(new_token_balance, token_frozen_before, "token total")?;
					let new_avg_price = if total > Decimal::ZERO { checked_div(new_usdc_cost, total, "avg_price")?.trunc_with_scale(8).normalize() } else { Decimal::ZERO };

					let token_update_id = update_position(
						&mut tx,
						PositionUpdate {
							user_id: maker_info.maker_user_id,
							token_id: &maker_info.maker_token_id,
							balance: new_token_balance,
							frozen_balance: token_frozen_before,
							usdc_cost: Some(new_usdc_cost),
							avg_price: Some(new_avg_price),
							redeemed: None,
							payout: None,
							redeemed_timestamp: None,
						},
					)
					.await?;

					// 插入 asset_history
					insert_asset_history_struct(
						&mut tx,
						AssetHistoryInsert {
							user_id: maker_info.maker_user_id,
							history_type: AssetHistoryType::OnChainBuySuccess,
							usdc_amount: Some(-maker_info.maker_usdc_amount),
							usdc_balance_before: Some(usdc_balance_before),
							usdc_balance_after: Some(usdc_balance_before),
							usdc_frozen_balance_before: Some(usdc_frozen_before),
							usdc_frozen_balance_after: Some(new_usdc_frozen),
							token_id: Some(&maker_info.maker_token_id),
							token_amount: Some(maker_info.real_maker_token_amount),
							token_balance_before: Some(token_balance_before),
							token_balance_after: Some(new_token_balance),
							token_frozen_balance_before: Some(token_frozen_before),
							token_frozen_balance_after: Some(token_frozen_before),
							tx_hash: Some(tx_hash),
							trade_id: Some(trade_id.to_string()),
							order_id: Some(maker_info.maker_order_id.to_string()),
						},
					)
					.await?;

					// 插入 OperationHistory (maker 买成功)
					let maker_price = if maker_info.real_maker_token_amount > Decimal::ZERO {
						checked_div(maker_info.maker_usdc_amount, maker_info.real_maker_token_amount, "maker price")?.trunc_with_scale(8).normalize()
					} else {
						Decimal::ZERO
					};
					insert_operation_history(
						&mut tx,
						event_id,
						market_id,
						maker_info.maker_user_id,
						AssetHistoryType::OnChainBuySuccess,
						Some(maker_info.maker_outcome_name.clone()),
						Some(maker_info.maker_token_id.clone()),
						Some(maker_info.real_maker_token_amount),
						Some(maker_price),
						Some(maker_info.maker_usdc_amount),
						tx_hash,
					)
					.await?;

					// 收集仓位更新事件
					position_events.push(PositionEvent {
						event_type: PositionEventType::Updated,
						privy_id: maker_info.maker_privy_user_id.clone(),
						event_id,
						market_id,
						outcome_name: maker_info.maker_outcome_name.clone(),
						token_id: maker_info.maker_token_id.clone(),
						quantity: total,
						avg_price: new_avg_price,
						update_id: token_update_id,
					});
				}
				OrderSide::Sell => {
					// Maker 卖：token frozen_balance 减少，USDC balance 增加
					let token_pos = positions_map.get(&(maker_info.maker_user_id, maker_info.maker_token_id.clone())).ok_or_else(|| anyhow::anyhow!("Maker token position not found"))?;
					let token_balance_before = token_pos.balance;
					let token_frozen_before = token_pos.frozen_balance;

					let new_token_frozen = checked_sub(token_frozen_before, maker_info.maker_token_amount, "maker token frozen")?;
					let new_usdc_cost = checked_sub(token_pos.usdc_cost.unwrap_or(Decimal::ZERO), maker_info.maker_usdc_amount, "maker usdc_cost")?;
					let total = checked_add(token_balance_before, new_token_frozen, "token total")?;
					let new_avg_price = if total > Decimal::ZERO { checked_div(new_usdc_cost, total, "avg_price")?.trunc_with_scale(8).normalize() } else { Decimal::ZERO };

					let token_update_id = update_position(
						&mut tx,
						PositionUpdate {
							user_id: maker_info.maker_user_id,
							token_id: &maker_info.maker_token_id,
							balance: token_balance_before,
							frozen_balance: new_token_frozen,
							usdc_cost: Some(new_usdc_cost),
							avg_price: Some(new_avg_price),
							redeemed: None,
							payout: None,
							redeemed_timestamp: None,
						},
					)
					.await?;

					// USDC 增加
					let usdc_pos = positions_map.get(&(maker_info.maker_user_id, USDC_TOKEN_ID.to_string())).ok_or_else(|| anyhow::anyhow!("Maker USDC position not found"))?;
					let usdc_balance_before = usdc_pos.balance;
					let usdc_frozen_before = usdc_pos.frozen_balance;

					let new_usdc_balance = checked_add(usdc_balance_before, maker_info.real_maker_usdc_amount, "maker usdc balance")?;

					let _usdc_update_id = update_position(
						&mut tx,
						PositionUpdate {
							user_id: maker_info.maker_user_id,
							token_id: USDC_TOKEN_ID,
							balance: new_usdc_balance,
							frozen_balance: usdc_frozen_before,
							usdc_cost: usdc_pos.usdc_cost,
							avg_price: usdc_pos.avg_price,
							redeemed: None,
							payout: None,
							redeemed_timestamp: None,
						},
					)
					.await?;

					// 插入 asset_history
					insert_asset_history_struct(
						&mut tx,
						AssetHistoryInsert {
							user_id: maker_info.maker_user_id,
							history_type: AssetHistoryType::OnChainSellSuccess,
							usdc_amount: Some(maker_info.real_maker_usdc_amount),
							usdc_balance_before: Some(usdc_balance_before),
							usdc_balance_after: Some(new_usdc_balance),
							usdc_frozen_balance_before: Some(usdc_frozen_before),
							usdc_frozen_balance_after: Some(usdc_frozen_before),
							token_id: Some(&maker_info.maker_token_id),
							token_amount: Some(-maker_info.maker_token_amount),
							token_balance_before: Some(token_balance_before),
							token_balance_after: Some(token_balance_before),
							token_frozen_balance_before: Some(token_frozen_before),
							token_frozen_balance_after: Some(new_token_frozen),
							tx_hash: Some(tx_hash),
							trade_id: Some(trade_id.to_string()),
							order_id: Some(maker_info.maker_order_id.to_string()),
						},
					)
					.await?;

					// 插入 OperationHistory (maker 卖成功)
					let maker_price = if maker_info.real_maker_token_amount > Decimal::ZERO {
						checked_div(maker_info.real_maker_usdc_amount, maker_info.real_maker_token_amount, "maker price")?.trunc_with_scale(8).normalize()
					} else {
						Decimal::ZERO
					};
					insert_operation_history(
						&mut tx,
						event_id,
						market_id,
						maker_info.maker_user_id,
						AssetHistoryType::OnChainSellSuccess,
						Some(maker_info.maker_outcome_name.clone()),
						Some(maker_info.maker_token_id.clone()),
						Some(maker_info.real_maker_token_amount),
						Some(maker_price),
						Some(maker_info.real_maker_usdc_amount),
						tx_hash,
					)
					.await?;

					// 收集仓位更新事件
					position_events.push(PositionEvent {
						event_type: PositionEventType::Updated,
						privy_id: maker_info.maker_privy_user_id.clone(),
						event_id,
						market_id,
						outcome_name: maker_info.maker_outcome_name.clone(),
						token_id: maker_info.maker_token_id.clone(),
						quantity: total,
						avg_price: new_avg_price,
						update_id: token_update_id,
					});
				}
			}
		} else {
			// 链上交易失败
			match maker_info.maker_side {
				OrderSide::Buy => {
					// Maker 买失败：USDC balance 增加，frozen_balance 减少
					let usdc_pos = positions_map.get(&(maker_info.maker_user_id, USDC_TOKEN_ID.to_string())).ok_or_else(|| anyhow::anyhow!("Maker USDC position not found"))?;
					let usdc_balance_before = usdc_pos.balance;
					let usdc_frozen_before = usdc_pos.frozen_balance;

					let new_usdc_balance = checked_add(usdc_balance_before, maker_info.maker_usdc_amount, "maker usdc balance")?;
					let new_usdc_frozen = checked_sub(usdc_frozen_before, maker_info.maker_usdc_amount, "maker usdc frozen")?;

					let _usdc_update_id = update_position(
						&mut tx,
						PositionUpdate {
							user_id: maker_info.maker_user_id,
							token_id: USDC_TOKEN_ID,
							balance: new_usdc_balance,
							frozen_balance: new_usdc_frozen,
							usdc_cost: usdc_pos.usdc_cost,
							avg_price: usdc_pos.avg_price,
							redeemed: None,
							payout: None,
							redeemed_timestamp: None,
						},
					)
					.await?;

					// 插入 asset_history
					insert_asset_history_struct(
						&mut tx,
						AssetHistoryInsert {
							user_id: maker_info.maker_user_id,
							history_type: AssetHistoryType::OnChainBuyFailed,
							usdc_amount: Some(maker_info.maker_usdc_amount),
							usdc_balance_before: Some(usdc_balance_before),
							usdc_balance_after: Some(new_usdc_balance),
							usdc_frozen_balance_before: Some(usdc_frozen_before),
							usdc_frozen_balance_after: Some(new_usdc_frozen),
							token_id: None,
							token_amount: None,
							token_balance_before: None,
							token_balance_after: None,
							token_frozen_balance_before: None,
							token_frozen_balance_after: None,
							tx_hash: None,
							trade_id: Some(trade_id.to_string()),
							order_id: Some(maker_info.maker_order_id.to_string()),
						},
					)
					.await?;
				}
				OrderSide::Sell => {
					// Maker 卖失败：token balance 增加，frozen_balance 减少
					let token_pos = positions_map.get(&(maker_info.maker_user_id, maker_info.maker_token_id.clone())).ok_or_else(|| anyhow::anyhow!("Maker token position not found"))?;
					let token_balance_before = token_pos.balance;
					let token_frozen_before = token_pos.frozen_balance;

					let new_token_balance = checked_add(token_balance_before, maker_info.maker_token_amount, "maker token balance")?;
					let new_token_frozen = checked_sub(token_frozen_before, maker_info.maker_token_amount, "maker token frozen")?;

					let _token_update_id = update_position(
						&mut tx,
						PositionUpdate {
							user_id: maker_info.maker_user_id,
							token_id: &maker_info.maker_token_id,
							balance: new_token_balance,
							frozen_balance: new_token_frozen,
							usdc_cost: token_pos.usdc_cost,
							avg_price: token_pos.avg_price,
							redeemed: None,
							payout: None,
							redeemed_timestamp: None,
						},
					)
					.await?;

					// 插入 asset_history
					insert_asset_history_struct(
						&mut tx,
						AssetHistoryInsert {
							user_id: maker_info.maker_user_id,
							history_type: AssetHistoryType::OnChainSellFailed,
							usdc_amount: None,
							usdc_balance_before: None,
							usdc_balance_after: None,
							usdc_frozen_balance_before: None,
							usdc_frozen_balance_after: None,
							token_id: Some(&maker_info.maker_token_id),
							token_amount: Some(maker_info.maker_token_amount),
							token_balance_before: Some(token_balance_before),
							token_balance_after: Some(new_token_balance),
							token_frozen_balance_before: Some(token_frozen_before),
							token_frozen_balance_after: Some(new_token_frozen),
							tx_hash: None,
							trade_id: Some(trade_id.to_string()),
							order_id: Some(maker_info.maker_order_id.to_string()),
						},
					)
					.await?;
				}
			}
		}
	}

	tx.commit().await?;

	info!("handle_trade_onchain_send_result: trade_id={}, is_success={}", trade_id, is_success);

	// 发送仓位更新事件到 channel（非阻塞）
	crate::user_event::send_events(position_events);

	Ok(())
}
