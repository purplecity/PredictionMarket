//! Asset cancel order tests

use {
	common::{
		consts::USDC_TOKEN_ID,
		engine_types::{OrderSide, OrderType},
		model::SignatureOrderMsg,
	},
	rust_decimal::Decimal,
	tests::test_utils::*,
};

fn create_test_signature_msg() -> SignatureOrderMsg {
	SignatureOrderMsg {
		expiration: "1234567890".to_string(),
		fee_rate_bps: "100".to_string(),
		maker: "0x1234".to_string(),
		maker_amount: "100".to_string(),
		nonce: "1".to_string(),
		salt: 12345,
		side: "buy".to_string(),
		signature: "0xabcd".to_string(),
		signature_type: 1,
		signer: "0x5678".to_string(),
		taker: "0x9999".to_string(),
		taker_amount: "100".to_string(),
		token_id: "test_token".to_string(),
	}
}

/// 测试取消买单
#[tokio::test]
async fn test_cancel_buy_order() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let event_id = generate_test_event_id();
	let market_id = 1i16;

	// 创建测试用户和市场
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");
	let (token_0_id, _token_1_id) = env.create_test_event_and_market(event_id, market_id).await.expect("Failed to create event and market");

	// 存入 USDC
	let usdc_amount = parse_decimal("1000.00");
	let deposit_tx_hash = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(user_id, USDC_TOKEN_ID, usdc_amount, &deposit_tx_hash, None, None, Some(privy_id.clone()), None, None, Some(env.pool.clone()))
		.await
		.expect("USDC deposit should succeed");

	// 创建买单
	let price = parse_decimal("0.6");
	let quantity = parse_decimal("100.0");
	let volume = price * quantity; // 60.0
	let order_id = asset::handlers::handle_create_order(
		user_id,
		event_id,
		market_id,
		&token_0_id,
		"Yes",
		OrderSide::Buy,
		OrderType::Limit,
		price,
		quantity,
		volume,
		create_test_signature_msg(),
		Some(env.pool.clone()),
	)
	.await
	.expect("Create order should succeed");

	// 取消全部订单
	let cancelled_quantity = quantity;
	let cancelled_volume = volume;
	let result = asset::handlers::handle_cancel_order(user_id, order_id, cancelled_quantity, cancelled_volume, Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Cancel order should succeed: {:?}", result.err());

	// 验证 USDC 余额恢复
	let usdc_available = env.get_user_available_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC available balance");
	let usdc_frozen = env.get_user_frozen_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC frozen balance");
	assert_eq!(usdc_available, usdc_amount, "USDC available should be restored");
	assert_eq!(usdc_frozen, Decimal::ZERO, "USDC frozen should be zero");
}

/// 测试取消卖单
#[tokio::test]
async fn test_cancel_sell_order() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let event_id = generate_test_event_id();
	let market_id = 1i16;

	// 创建测试用户和市场
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");
	let (token_0_id, _token_1_id) = env.create_test_event_and_market(event_id, market_id).await.expect("Failed to create event and market");

	// 存入 Token
	let token_amount = parse_decimal("100.0");
	let deposit_tx_hash = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(
		user_id,
		&token_0_id,
		token_amount,
		&deposit_tx_hash,
		Some(event_id),
		Some(market_id),
		Some(privy_id.clone()),
		Some("Yes".to_string()),
		None,
		Some(env.pool.clone()),
	)
	.await
	.expect("Token deposit should succeed");

	// 创建卖单
	let price = parse_decimal("0.4");
	let quantity = parse_decimal("50.0");
	let volume = price * quantity;
	let order_id = asset::handlers::handle_create_order(
		user_id,
		event_id,
		market_id,
		&token_0_id,
		"Yes",
		OrderSide::Sell,
		OrderType::Limit,
		price,
		quantity,
		volume,
		create_test_signature_msg(),
		Some(env.pool.clone()),
	)
	.await
	.expect("Create order should succeed");

	// 取消全部订单
	let cancelled_quantity = quantity;
	let cancelled_volume = volume;
	let result = asset::handlers::handle_cancel_order(user_id, order_id, cancelled_quantity, cancelled_volume, Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Cancel order should succeed: {:?}", result.err());

	// 验证 Token 余额恢复
	let token_available = env.get_user_available_balance(user_id, &token_0_id).await.expect("Failed to get token available balance");
	let token_frozen = env.get_user_frozen_balance(user_id, &token_0_id).await.expect("Failed to get token frozen balance");
	assert_eq!(token_available, token_amount, "Token available should be restored");
	assert_eq!(token_frozen, Decimal::ZERO, "Token frozen should be zero");
}

/// 测试部分取消订单
#[tokio::test]
async fn test_partial_cancel_order() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let event_id = generate_test_event_id();
	let market_id = 1i16;

	// 创建测试用户和市场
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");
	let (token_0_id, _token_1_id) = env.create_test_event_and_market(event_id, market_id).await.expect("Failed to create event and market");

	// 存入 USDC
	let usdc_amount = parse_decimal("1000.00");
	let deposit_tx_hash = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(user_id, USDC_TOKEN_ID, usdc_amount, &deposit_tx_hash, None, None, Some(privy_id.clone()), None, None, Some(env.pool.clone()))
		.await
		.expect("USDC deposit should succeed");

	// 创建买单
	let price = parse_decimal("0.6");
	let quantity = parse_decimal("100.0");
	let volume = price * quantity; // 60.0
	let order_id = asset::handlers::handle_create_order(
		user_id,
		event_id,
		market_id,
		&token_0_id,
		"Yes",
		OrderSide::Buy,
		OrderType::Limit,
		price,
		quantity,
		volume,
		create_test_signature_msg(),
		Some(env.pool.clone()),
	)
	.await
	.expect("Create order should succeed");

	// 部分取消订单（取消 30%）
	let cancelled_quantity = parse_decimal("30.0");
	let cancelled_volume = price * cancelled_quantity; // 18.0
	let result = asset::handlers::handle_cancel_order(user_id, order_id, cancelled_quantity, cancelled_volume, Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Partial cancel should succeed: {:?}", result.err());

	// 验证 USDC 余额（部分恢复）
	let usdc_available = env.get_user_available_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC available balance");
	let usdc_frozen = env.get_user_frozen_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC frozen balance");
	let expected_available = usdc_amount - volume + cancelled_volume;
	let expected_frozen = volume - cancelled_volume;
	assert_eq!(usdc_available, expected_available, "USDC available should increase by cancelled volume");
	assert_eq!(usdc_frozen, expected_frozen, "USDC frozen should decrease by cancelled volume");
}

/// 测试订单被拒绝
#[tokio::test]
async fn test_order_rejected() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let event_id = generate_test_event_id();
	let market_id = 1i16;

	// 创建测试用户和市场
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");
	let (token_0_id, _token_1_id) = env.create_test_event_and_market(event_id, market_id).await.expect("Failed to create event and market");

	// 存入 USDC
	let usdc_amount = parse_decimal("1000.00");
	let deposit_tx_hash = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(user_id, USDC_TOKEN_ID, usdc_amount, &deposit_tx_hash, None, None, Some(privy_id.clone()), None, None, Some(env.pool.clone()))
		.await
		.expect("USDC deposit should succeed");

	// 创建买单
	let price = parse_decimal("0.6");
	let quantity = parse_decimal("100.0");
	let volume = price * quantity;
	let order_id = asset::handlers::handle_create_order(
		user_id,
		event_id,
		market_id,
		&token_0_id,
		"Yes",
		OrderSide::Buy,
		OrderType::Limit,
		price,
		quantity,
		volume,
		create_test_signature_msg(),
		Some(env.pool.clone()),
	)
	.await
	.expect("Create order should succeed");

	// 模拟订单被拒绝
	let result = asset::handlers::handle_order_rejected(user_id, order_id, Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Order rejected should succeed: {:?}", result.err());

	// 验证 USDC 余额恢复
	let usdc_available = env.get_user_available_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC available balance");
	let usdc_frozen = env.get_user_frozen_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC frozen balance");
	assert_eq!(usdc_available, usdc_amount, "USDC available should be fully restored");
	assert_eq!(usdc_frozen, Decimal::ZERO, "USDC frozen should be zero");
}
