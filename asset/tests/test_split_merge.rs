mod test_utils;

use {common::consts::USDC_TOKEN_ID, rust_decimal::Decimal, test_utils::*};

/// 测试 Split 操作（USDC -> token_0 + token_1）
#[tokio::test]
async fn test_split() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let event_id = generate_test_event_id();
	let market_id = 1i16;

	// 创建测试用户和市场
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");
	let (token_0_id, token_1_id) = env.create_test_event_and_market(event_id, market_id).await.expect("Failed to create event and market");

	// 存入 USDC
	let usdc_amount = parse_decimal("100.00");
	let deposit_trace_id = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(user_id, USDC_TOKEN_ID, usdc_amount, &deposit_trace_id, None, None, Some(privy_id.clone()), None, Some(env.pool.clone()))
		.await
		.expect("USDC deposit should succeed");

	// 执行 Split
	let split_amount = parse_decimal("50.00");
	let tx_hash = format!("0x{:064x}", generate_test_user_id());
	let result =
		asset::handlers::handle_split(user_id, event_id, market_id, split_amount, &token_0_id, &token_1_id, split_amount, split_amount, &tx_hash, &privy_id, "Yes", "No", Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Split should succeed: {:?}", result.err());

	// 验证 USDC 余额减少
	let usdc_balance = env.get_user_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC balance");
	assert_eq!(usdc_balance, usdc_amount - split_amount, "USDC balance should decrease");

	// 验证 token_0 余额增加
	let token_0_balance = env.get_user_balance(user_id, &token_0_id).await.expect("Failed to get token_0 balance");
	assert_eq!(token_0_balance, split_amount, "Token_0 balance should equal split amount");

	// 验证 token_1 余额增加
	let token_1_balance = env.get_user_balance(user_id, &token_1_id).await.expect("Failed to get token_1 balance");
	assert_eq!(token_1_balance, split_amount, "Token_1 balance should equal split amount");

	// Cleanup - skip cleanup to avoid connection pool issues
	// Each test uses unique IDs so no conflicts
	// env.cleanup_test_user(user_id).await.expect("Failed to cleanup test user");
	// env.cleanup_test_event(event_id).await.expect("Failed to cleanup test event");
}

/// 测试 Merge 操作（token_0 + token_1 -> USDC）
#[tokio::test]
async fn test_merge() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let event_id = generate_test_event_id();
	let market_id = 1i16;

	// 创建测试用户和市场
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");
	let (token_0_id, token_1_id) = env.create_test_event_and_market(event_id, market_id).await.expect("Failed to create event and market");

	// 存入 token_0 和 token_1
	let token_amount = parse_decimal("100.00");
	let deposit_trace_id_0 = uuid::Uuid::new_v4().to_string();
	let deposit_trace_id_1 = uuid::Uuid::new_v4().to_string();

	asset::handlers::handle_deposit(user_id, &token_0_id, token_amount, &deposit_trace_id_0, Some(event_id), Some(market_id), Some(privy_id.clone()), Some("Yes".to_string()), Some(env.pool.clone()))
		.await
		.expect("Token_0 deposit should succeed");

	asset::handlers::handle_deposit(user_id, &token_1_id, token_amount, &deposit_trace_id_1, Some(event_id), Some(market_id), Some(privy_id.clone()), Some("No".to_string()), Some(env.pool.clone()))
		.await
		.expect("Token_1 deposit should succeed");

	// 执行 Merge
	let merge_amount = parse_decimal("50.00");
	let tx_hash = format!("0x{:064x}", generate_test_user_id());
	let result =
		asset::handlers::handle_merge(user_id, event_id, market_id, &token_0_id, &token_1_id, merge_amount, merge_amount, merge_amount, &tx_hash, &privy_id, "Yes", "No", Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Merge should succeed: {:?}", result.err());

	// 验证 token_0 余额减少
	let token_0_balance = env.get_user_balance(user_id, &token_0_id).await.expect("Failed to get token_0 balance");
	assert_eq!(token_0_balance, token_amount - merge_amount, "Token_0 balance should decrease");

	// 验证 token_1 余额减少
	let token_1_balance = env.get_user_balance(user_id, &token_1_id).await.expect("Failed to get token_1 balance");
	assert_eq!(token_1_balance, token_amount - merge_amount, "Token_1 balance should decrease");

	// 验证 USDC 余额增加
	let usdc_balance = env.get_user_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC balance");
	assert_eq!(usdc_balance, merge_amount, "USDC balance should equal merge amount");

	// Cleanup - skip cleanup to avoid connection pool issues
	// Each test uses unique IDs so no conflicts
	// env.cleanup_test_user(user_id).await.expect("Failed to cleanup test user");
	// env.cleanup_test_event(event_id).await.expect("Failed to cleanup test event");
}

/// 测试 Split 余额不足
#[tokio::test]
async fn test_split_insufficient_balance() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let event_id = generate_test_event_id();
	let market_id = 1i16;

	// 创建测试用户和市场
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");
	let (token_0_id, token_1_id) = env.create_test_event_and_market(event_id, market_id).await.expect("Failed to create event and market");

	// 存入少量 USDC
	let usdc_amount = parse_decimal("10.00");
	let deposit_trace_id = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(user_id, USDC_TOKEN_ID, usdc_amount, &deposit_trace_id, None, None, Some(privy_id.clone()), None, Some(env.pool.clone()))
		.await
		.expect("USDC deposit should succeed");

	// 尝试 Split 超过余额的金额（应该成功，允许余额为负，因为用户掌握私钥）
	let split_amount = parse_decimal("50.00");
	let tx_hash = format!("0x{:064x}", generate_test_user_id());
	let result =
		asset::handlers::handle_split(user_id, event_id, market_id, split_amount, &token_0_id, &token_1_id, split_amount, split_amount, &tx_hash, &privy_id, "Yes", "No", Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_ok(), "Split should succeed even with insufficient balance");

	// 验证 USDC 余额变为负数
	let expected_balance = usdc_amount - split_amount; // 10 - 50 = -40
	let usdc_balance = env.get_user_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC balance");
	assert_eq!(usdc_balance, expected_balance, "USDC balance should be negative after split");

	// 验证 token 余额增加
	let token_0_balance = env.get_user_balance(user_id, &token_0_id).await.expect("Failed to get token_0 balance");
	let token_1_balance = env.get_user_balance(user_id, &token_1_id).await.expect("Failed to get token_1 balance");
	assert_eq!(token_0_balance, split_amount, "Token_0 balance should equal split amount");
	assert_eq!(token_1_balance, split_amount, "Token_1 balance should equal split amount");

	// Cleanup - skip cleanup to avoid connection pool issues
	// Each test uses unique IDs so no conflicts
	// env.cleanup_test_user(user_id).await.expect("Failed to cleanup test user");
	// env.cleanup_test_event(event_id).await.expect("Failed to cleanup test event");
}

/// 测试 Merge 余额不足
#[tokio::test]
async fn test_merge_insufficient_balance() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let event_id = generate_test_event_id();
	let market_id = 1i16;

	// 创建测试用户和市场
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");
	let (token_0_id, token_1_id) = env.create_test_event_and_market(event_id, market_id).await.expect("Failed to create event and market");

	// 只存入 token_0（没有 token_1）
	let token_amount = parse_decimal("100.00");
	let deposit_trace_id = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(user_id, &token_0_id, token_amount, &deposit_trace_id, Some(event_id), Some(market_id), Some(privy_id.clone()), Some("Yes".to_string()), Some(env.pool.clone()))
		.await
		.expect("Token_0 deposit should succeed");

	// 尝试 Merge（缺少 token_1）
	let merge_amount = parse_decimal("50.00");
	let tx_hash = format!("0x{:064x}", generate_test_user_id());
	let result =
		asset::handlers::handle_merge(user_id, event_id, market_id, &token_0_id, &token_1_id, merge_amount, merge_amount, merge_amount, &tx_hash, &privy_id, "Yes", "No", Some(env.pool.clone())).await;

	// 验证结果
	assert!(result.is_err(), "Merge should fail with insufficient token_1 balance");

	// 验证余额未变化
	let token_0_balance = env.get_user_balance(user_id, &token_0_id).await.expect("Failed to get token_0 balance");
	assert_eq!(token_0_balance, token_amount, "Token_0 balance should remain unchanged");

	// Cleanup - skip cleanup to avoid connection pool issues
	// Each test uses unique IDs so no conflicts
	// env.cleanup_test_user(user_id).await.expect("Failed to cleanup test user");
	// env.cleanup_test_event(event_id).await.expect("Failed to cleanup test event");
}

/// 测试 Split 然后 Merge（往返测试）
#[tokio::test]
async fn test_split_then_merge_roundtrip() {
	// Setup
	let env = TestEnv::setup().await.expect("Failed to setup test environment");

	// 生成测试数据
	let user_id = generate_test_user_id();
	let privy_id = format!("test_privy_{}", user_id);
	let event_id = generate_test_event_id();
	let market_id = 1i16;

	// 创建测试用户和市场
	env.create_test_user(user_id, &privy_id).await.expect("Failed to create test user");
	let (token_0_id, token_1_id) = env.create_test_event_and_market(event_id, market_id).await.expect("Failed to create event and market");

	// 存入 USDC
	let initial_usdc = parse_decimal("100.00");
	let deposit_trace_id = uuid::Uuid::new_v4().to_string();
	asset::handlers::handle_deposit(user_id, USDC_TOKEN_ID, initial_usdc, &deposit_trace_id, None, None, Some(privy_id.clone()), None, Some(env.pool.clone()))
		.await
		.expect("USDC deposit should succeed");

	// 执行 Split
	let split_amount = parse_decimal("50.00");
	let split_tx_hash = format!("0x{:064x}", generate_test_user_id());
	asset::handlers::handle_split(user_id, event_id, market_id, split_amount, &token_0_id, &token_1_id, split_amount, split_amount, &split_tx_hash, &privy_id, "Yes", "No", Some(env.pool.clone()))
		.await
		.expect("Split should succeed");

	// 执行 Merge（合并刚才分割的代币）
	let merge_tx_hash = format!("0x{:064x}", generate_test_user_id() + 1);
	asset::handlers::handle_merge(user_id, event_id, market_id, &token_0_id, &token_1_id, split_amount, split_amount, split_amount, &merge_tx_hash, &privy_id, "Yes", "No", Some(env.pool.clone()))
		.await
		.expect("Merge should succeed");

	// 验证 USDC 余额恢复到初始值
	let final_usdc = env.get_user_balance(user_id, USDC_TOKEN_ID).await.expect("Failed to get USDC balance");
	assert_eq!(final_usdc, initial_usdc, "USDC balance should be restored after split-merge roundtrip");

	// 验证 token 余额为 0
	let token_0_balance = env.get_user_balance(user_id, &token_0_id).await.expect("Failed to get token_0 balance");
	let token_1_balance = env.get_user_balance(user_id, &token_1_id).await.expect("Failed to get token_1 balance");
	assert_eq!(token_0_balance, Decimal::ZERO, "Token_0 balance should be zero");
	assert_eq!(token_1_balance, Decimal::ZERO, "Token_1 balance should be zero");

	// Cleanup - skip cleanup to avoid connection pool issues
	// Each test uses unique IDs so no conflicts
	// env.cleanup_test_user(user_id).await.expect("Failed to cleanup test user");
	// env.cleanup_test_event(event_id).await.expect("Failed to cleanup test event");
}
