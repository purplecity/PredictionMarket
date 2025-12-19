//! Asset deposit and withdraw tests

use {rust_decimal::Decimal, tests::test_utils::*};

/// 测试 USDC 存款
#[tokio::test]
async fn test_usdc_deposit() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let token_id = common::consts::USDC_TOKEN_ID;
	let amount = parse_decimal("100.50");
	let tx_hash = uuid::Uuid::new_v4().to_string();

	// 创建测试用户
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");

	// 执行存款
	let result = asset::handlers::handle_deposit(user_id, token_id, amount, &tx_hash, None, None, Some(privy_id.clone()), None, Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Deposit should succeed: {:?}", result.err());

	// 验证余额
	let balance = env.get_user_balance(user_id, token_id).await.expect("Failed to get balance");
	assert_eq!(balance, amount, "Balance should equal deposit amount");

	// 验证可用余额
	let available = env.get_user_available_balance(user_id, token_id).await.expect("Failed to get available balance");
	assert_eq!(available, amount, "Available balance should equal deposit amount");

	// 验证冻结余额
	let frozen = env.get_user_frozen_balance(user_id, token_id).await.expect("Failed to get frozen balance");
	assert_eq!(frozen, Decimal::ZERO, "Frozen balance should be zero");
}

/// 测试 Token 存款（需要事件和市场）
#[tokio::test]
async fn test_token_deposit() {
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

	let amount = parse_decimal("50.25");
	let tx_hash = uuid::Uuid::new_v4().to_string();

	// 执行存款
	let result =
		asset::handlers::handle_deposit(user_id, &token_0_id, amount, &tx_hash, Some(event_id), Some(market_id), Some(privy_id.clone()), Some("Yes".to_string()), Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Token deposit should succeed: {:?}", result.err());

	// 验证余额
	let balance = env.get_user_balance(user_id, &token_0_id).await.expect("Failed to get balance");
	assert_eq!(balance, amount, "Balance should equal deposit amount");

	// Cleanup
	env.cleanup_test_user(user_id).await.expect("Failed to cleanup test user");
	env.cleanup_test_event(event_id).await.expect("Failed to cleanup test event");
	env.teardown().await;
}

/// 测试 USDC 取款
#[tokio::test]
async fn test_usdc_withdraw() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let token_id = common::consts::USDC_TOKEN_ID;
	let deposit_amount = parse_decimal("100.00");
	let withdraw_amount = parse_decimal("30.00");
	let deposit_tx_hash = uuid::Uuid::new_v4().to_string();
	let withdraw_tx_hash = uuid::Uuid::new_v4().to_string();

	// 创建测试用户
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");

	// 先存款
	asset::handlers::handle_deposit(user_id, token_id, deposit_amount, &deposit_tx_hash, None, None, Some(privy_id.clone()), None, Some(env.pool.clone())).await.expect("Deposit should succeed");

	// 执行取款
	let result = asset::handlers::handle_withdraw(user_id, token_id, withdraw_amount, &withdraw_tx_hash, None, None, Some(privy_id.clone()), None, Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Withdraw should succeed: {:?}", result.err());

	// 验证余额
	let expected_balance = deposit_amount - withdraw_amount;
	let balance = env.get_user_balance(user_id, token_id).await.expect("Failed to get balance");
	assert_eq!(balance, expected_balance, "Balance should equal deposit - withdraw");
}

/// 测试余额不足
#[tokio::test]
async fn test_withdraw_insufficient_balance() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let token_id = common::consts::USDC_TOKEN_ID;
	let deposit_amount = parse_decimal("50.00");
	let withdraw_amount = parse_decimal("100.00"); // 超过存款金额
	let deposit_tx_hash = uuid::Uuid::new_v4().to_string();
	let withdraw_tx_hash = uuid::Uuid::new_v4().to_string();

	// 创建测试用户
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");

	// 先存款
	asset::handlers::handle_deposit(user_id, token_id, deposit_amount, &deposit_tx_hash, None, None, Some(privy_id.clone()), None, Some(env.pool.clone())).await.expect("Deposit should succeed");

	// 执行取款（应该成功，允许余额为负，因为用户掌握私钥）
	let result = asset::handlers::handle_withdraw(user_id, token_id, withdraw_amount, &withdraw_tx_hash, None, None, Some(privy_id.clone()), None, Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Withdraw should succeed even with insufficient balance");

	// 验证余额变为负数
	let expected_balance = deposit_amount - withdraw_amount; // 50 - 100 = -50
	let balance = env.get_user_balance(user_id, token_id).await.expect("Failed to get balance");
	assert_eq!(balance, expected_balance, "Balance should be negative after withdraw");
}

/// 测试重复存款（幂等性）
/// 通过 UNIQUE NULLS NOT DISTINCT 约束确保相同 tx_hash 不能重复存款
#[tokio::test]
async fn test_duplicate_deposit_idempotency() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let token_id = common::consts::USDC_TOKEN_ID;
	let amount = parse_decimal("100.00");
	let tx_hash = uuid::Uuid::new_v4().to_string();

	// 创建测试用户
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");

	// 第一次存款
	let result1 = asset::handlers::handle_deposit(user_id, token_id, amount, &tx_hash, None, None, Some(privy_id.clone()), None, Some(env.pool.clone())).await;
	assert!(result1.is_ok(), "First deposit should succeed");

	// 使用相同的 tx_hash 再次存款（应该失败，因为违反联合唯一约束）
	let result2 = asset::handlers::handle_deposit(user_id, token_id, amount, &tx_hash, None, None, Some(privy_id.clone()), None, Some(env.pool.clone())).await;
	assert!(result2.is_err(), "Duplicate deposit should fail due to unique constraint violation");

	// 验证余额（应该只计入一次）
	let balance = env.get_user_balance(user_id, token_id).await.expect("Failed to get balance");
	assert_eq!(balance, amount, "Balance should only reflect one deposit");
}
