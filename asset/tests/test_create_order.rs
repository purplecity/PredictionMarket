mod test_utils;

use {
	common::{
		consts::USDC_TOKEN_ID,
		engine_types::{OrderSide, OrderType},
		model::SignatureOrderMsg,
	},
	rust_decimal::Decimal,
	test_utils::*,
};

/// 创建测试用的 SignatureOrderMsg
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

/// 测试创建买单
#[tokio::test]
async fn test_create_buy_order() {
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

	// 先存入 USDC（用于买单）
	let usdc_amount = parse_decimal("1000.00");
	let deposit_trace_id = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(user_id, USDC_TOKEN_ID, usdc_amount, &deposit_trace_id, None, None, Some(privy_id.clone()), None, Some(env.pool.clone()))
		.await
		.expect("USDC deposit should succeed");

	// 创建买单参数
	let price = parse_decimal("0.6");
	let quantity = parse_decimal("100.0");
	let volume = price * quantity; // 60.0
	let signature_msg = create_test_signature_msg();

	// 执行创建订单
	let result =
		asset::handlers::handle_create_order(user_id, event_id, market_id, &token_0_id, "Yes", OrderSide::Buy, OrderType::Limit, price, quantity, volume, signature_msg, Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Create buy order should succeed: {:?}", result.err());
	let order_id = result.unwrap();
	assert!(!order_id.is_nil(), "Order ID should not be nil");

	// 验证 USDC 余额变化（volume 应该被冻结）
	let usdc_available = env.get_user_available_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC available balance");
	let usdc_frozen = env.get_user_frozen_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC frozen balance");
	assert_eq!(usdc_available, usdc_amount - volume, "USDC available should decrease by volume");
	assert_eq!(usdc_frozen, volume, "USDC frozen should equal volume");

	// Cleanup - skip cleanup to avoid connection pool issues
	// Each test uses unique IDs so no conflicts
	// env.cleanup_test_user(user_id).await.expect("Failed to cleanup test user");
	// env.cleanup_test_event(event_id).await.expect("Failed to cleanup test event");
}

/// 测试创建卖单
#[tokio::test]
async fn test_create_sell_order() {
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

	// 先存入 Token（用于卖单）
	let token_amount = parse_decimal("100.0");
	let deposit_trace_id = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(user_id, &token_0_id, token_amount, &deposit_trace_id, Some(event_id), Some(market_id), Some(privy_id.clone()), Some("Yes".to_string()), Some(env.pool.clone()))
		.await
		.expect("Token deposit should succeed");

	// 创建卖单参数
	let price = parse_decimal("0.4");
	let quantity = parse_decimal("50.0");
	let volume = price * quantity; // 20.0
	let signature_msg = create_test_signature_msg();

	// 执行创建订单
	let result =
		asset::handlers::handle_create_order(user_id, event_id, market_id, &token_0_id, "Yes", OrderSide::Sell, OrderType::Limit, price, quantity, volume, signature_msg, Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Create sell order should succeed: {:?}", result.err());
	let order_id = result.unwrap();
	assert!(!order_id.is_nil(), "Order ID should not be nil");

	// 验证 Token 余额变化（quantity 应该被冻结）
	let token_available = env.get_user_available_balance(user_id, &token_0_id).await.expect("Failed to get token available balance");
	let token_frozen = env.get_user_frozen_balance(user_id, &token_0_id).await.expect("Failed to get token frozen balance");
	assert_eq!(token_available, token_amount - quantity, "Token available should decrease by quantity");
	assert_eq!(token_frozen, quantity, "Token frozen should equal quantity");

	// Cleanup - skip cleanup to avoid connection pool issues
	// Each test uses unique IDs so no conflicts
	// env.cleanup_test_user(user_id).await.expect("Failed to cleanup test user");
	// env.cleanup_test_event(event_id).await.expect("Failed to cleanup test event");
}

/// 测试余额不足时创建订单失败
#[tokio::test]
async fn test_create_order_insufficient_balance() {
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

	// 只存入少量 USDC
	let usdc_amount = parse_decimal("10.00");
	let deposit_trace_id = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(user_id, USDC_TOKEN_ID, usdc_amount, &deposit_trace_id, None, None, Some(privy_id.clone()), None, Some(env.pool.clone()))
		.await
		.expect("USDC deposit should succeed");

	// 尝试创建超过余额的买单
	let price = parse_decimal("0.6");
	let quantity = parse_decimal("100.0");
	let volume = price * quantity; // 60.0 > 10.0
	let signature_msg = create_test_signature_msg();

	// 执行创建订单（应该失败）
	let result =
		asset::handlers::handle_create_order(user_id, event_id, market_id, &token_0_id, "Yes", OrderSide::Buy, OrderType::Limit, price, quantity, volume, signature_msg, Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_err(), "Create order should fail with insufficient balance");

	// 验证余额未变化
	let usdc_available = env.get_user_available_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC available balance");
	let usdc_frozen = env.get_user_frozen_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC frozen balance");
	assert_eq!(usdc_available, usdc_amount, "USDC available should remain unchanged");
	assert_eq!(usdc_frozen, Decimal::ZERO, "USDC frozen should remain zero");

	// Cleanup - skip cleanup to avoid connection pool issues
	// Each test uses unique IDs so no conflicts
	// env.cleanup_test_user(user_id).await.expect("Failed to cleanup test user");
	// env.cleanup_test_event(event_id).await.expect("Failed to cleanup test event");
}

/// 测试创建多个订单
#[tokio::test]
async fn test_create_multiple_orders() {
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

	// 存入足够的 USDC
	let usdc_amount = parse_decimal("1000.00");
	let deposit_trace_id = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(user_id, USDC_TOKEN_ID, usdc_amount, &deposit_trace_id, None, None, Some(privy_id.clone()), None, Some(env.pool.clone()))
		.await
		.expect("USDC deposit should succeed");

	// 创建第一个订单
	let price1 = parse_decimal("0.5");
	let quantity1 = parse_decimal("100.0");
	let volume1 = price1 * quantity1;
	let result1 = asset::handlers::handle_create_order(
		user_id,
		event_id,
		market_id,
		&token_0_id,
		"Yes",
		OrderSide::Buy,
		OrderType::Limit,
		price1,
		quantity1,
		volume1,
		create_test_signature_msg(),
		Some(env.pool.clone()),
	)
	.await;
	assert!(result1.is_ok(), "First order should succeed");

	// 创建第二个订单
	let price2 = parse_decimal("0.6");
	let quantity2 = parse_decimal("200.0");
	let volume2 = price2 * quantity2;
	let result2 = asset::handlers::handle_create_order(
		user_id,
		event_id,
		market_id,
		&token_0_id,
		"Yes",
		OrderSide::Buy,
		OrderType::Limit,
		price2,
		quantity2,
		volume2,
		create_test_signature_msg(),
		Some(env.pool.clone()),
	)
	.await;
	assert!(result2.is_ok(), "Second order should succeed");

	// 验证总冻结金额
	let total_frozen = volume1 + volume2;
	let usdc_frozen = env.get_user_frozen_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC frozen balance");
	assert_eq!(usdc_frozen, total_frozen, "Total frozen should equal sum of both orders");

	// Cleanup - skip cleanup to avoid connection pool issues
	// Each test uses unique IDs so no conflicts
	// env.cleanup_test_user(user_id).await.expect("Failed to cleanup test user");
	// env.cleanup_test_event(event_id).await.expect("Failed to cleanup test event");
}
