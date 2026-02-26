//! 深度测试：get_cross_matching_orders 核心撮合逻辑
//!
//! 直接测试撮合引擎的核心匹配算法，不触发 Redis 消息发送
//! 覆盖所有边缘情况和异常场景

use {
	common::engine_types::{Order, OrderSide, OrderType, PredictionSymbol, SubmitOrderMessage},
	match_engine::engine::MatchEngine,
	rust_decimal::Decimal,
	std::{str::FromStr, sync::Arc},
	tokio::sync::broadcast,
};

// ============================================================================
// 测试辅助函数
// ============================================================================

/// 创建测试用的 MatchEngine
fn create_match_engine() -> MatchEngine {
	let (_exit_tx, exit_rx1) = broadcast::channel(1);
	let (_market_exit_tx, market_exit_rx) = tokio::sync::oneshot::channel();
	let (engine, _sender) = MatchEngine::new(1, 1, ("token1".to_string(), "token2".to_string()), 1000, exit_rx1, market_exit_rx);
	engine
}

/// 创建测试 Symbol (token1)
fn create_test_symbol() -> PredictionSymbol {
	PredictionSymbol::new(1, 1, "token1")
}

/// 创建测试订单
fn create_test_order(order_id: &str, side: OrderSide, price: i32, quantity: u64, user_id: i64) -> Order {
	let symbol = create_test_symbol();
	Order::new(order_id.to_string(), symbol, side, OrderType::Limit, quantity, price, user_id, format!("privy{}", user_id), "Yes".to_string()).unwrap()
}

/// 创建测试的 SubmitOrderMessage
fn create_submit_message(order_id: &str, side: OrderSide, order_type: OrderType, price: i32, quantity: u64, volume_str: &str, user_id: i64) -> SubmitOrderMessage {
	SubmitOrderMessage {
		order_id: order_id.to_string(),
		symbol: create_test_symbol(),
		side,
		order_type,
		price,
		quantity,
		volume: Decimal::from_str(volume_str).unwrap(),
		user_id,
		privy_id: format!("privy{}", user_id),
		outcome_name: "Yes".to_string(),
	}
}

/// 添加订单到引擎，同时交叉插入到对方订单簿
fn add_order_to_engine(engine: &mut MatchEngine, order: Order) {
	let order_id = order.order_id.clone();
	let order_arc = Arc::new(order);
	if order_arc.symbol.token_id == engine.token_0_id {
		engine.token_0_orderbook.add_order(Arc::clone(&order_arc)).unwrap();
		engine.token_1_orderbook.add_cross_order(Arc::clone(&order_arc)).unwrap();
		engine.token_0_orders.insert(order_id, order_arc);
	} else {
		engine.token_1_orderbook.add_order(Arc::clone(&order_arc)).unwrap();
		engine.token_0_orderbook.add_cross_order(Arc::clone(&order_arc)).unwrap();
		engine.token_1_orders.insert(order_id, order_arc);
	}
}

// ============================================================================
// 限价买单测试
// ============================================================================

#[tokio::test]
async fn test_limit_buy_full_match_single_order() {
	let mut engine = create_match_engine();

	// 盘口：一个卖单 @ 600, 数量 100
	let maker = create_test_order("maker1", OrderSide::Sell, 600, 100, 2);
	add_order_to_engine(&mut engine, maker);

	// 限价买单 @ 600, 数量 100
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 600, 100, "0.06", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 100);
	assert_eq!(result.remaining_quantity, 0);
	assert_eq!(result.filled_volume, Decimal::from_str("0.06").unwrap()); // 100 * 600 / 1000000
	assert_eq!(result.matched_orders.len(), 1);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_limit_buy_full_match_multiple_orders() {
	let mut engine = create_match_engine();

	// 盘口：三个卖单
	let maker1 = create_test_order("maker1", OrderSide::Sell, 500, 50, 2);
	let maker2 = create_test_order("maker2", OrderSide::Sell, 550, 30, 3);
	let maker3 = create_test_order("maker3", OrderSide::Sell, 600, 40, 4);
	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);
	add_order_to_engine(&mut engine, maker3);

	// 限价买单 @ 600, 数量 120
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 600, 120, "0.0665", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 120);
	assert_eq!(result.remaining_quantity, 0);
	// 50*500 + 30*550 + 40*600 = 25000 + 16500 + 24000 = 65500 / 1000000 = 0.0655
	assert_eq!(result.filled_volume, Decimal::from_str("0.0655").unwrap());
	assert_eq!(result.matched_orders.len(), 3);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_limit_buy_partial_match() {
	let mut engine = create_match_engine();

	// 盘口：一个卖单 @ 600, 数量 50
	let maker = create_test_order("maker1", OrderSide::Sell, 600, 50, 2);
	add_order_to_engine(&mut engine, maker);

	// 限价买单 @ 600, 数量 100
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 600, 100, "0.06", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 50);
	assert_eq!(result.remaining_quantity, 50);
	assert_eq!(result.filled_volume, Decimal::from_str("0.03").unwrap()); // 50 * 600 / 1000000
	assert_eq!(result.matched_orders.len(), 1);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_limit_buy_no_match_price_too_low() {
	let mut engine = create_match_engine();

	// 盘口：一个卖单 @ 600
	let maker = create_test_order("maker1", OrderSide::Sell, 600, 100, 2);
	add_order_to_engine(&mut engine, maker);

	// 限价买单 @ 500（低于卖单价格）
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 500, 100, "0.05", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 0);
	assert_eq!(result.remaining_quantity, 100);
	assert_eq!(result.filled_volume, Decimal::ZERO);
	assert_eq!(result.matched_orders.len(), 0);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_limit_buy_self_trade_detection() {
	let mut engine = create_match_engine();

	// 盘口：同一用户的卖单
	let maker = create_test_order("maker1", OrderSide::Sell, 600, 100, 1);
	add_order_to_engine(&mut engine, maker);

	// 限价买单，相同用户
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 600, 100, "0.06", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert!(result.has_self_trade);
	assert_eq!(result.filled_quantity, 0);
	assert_eq!(result.remaining_quantity, 100);
}

#[tokio::test]
async fn test_limit_buy_self_trade_after_partial_match() {
	let mut engine = create_match_engine();

	// 盘口：其他用户的卖单 + 同一用户的卖单
	let maker1 = create_test_order("maker1", OrderSide::Sell, 500, 50, 2);
	let maker2 = create_test_order("maker2", OrderSide::Sell, 600, 100, 1); // 同一用户
	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 限价买单
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 600, 150, "0.09", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert!(result.has_self_trade);
	assert_eq!(result.filled_quantity, 50); // 只匹配了第一个订单
	assert_eq!(result.remaining_quantity, 100);
	assert_eq!(result.matched_orders.len(), 1);
}

#[tokio::test]
async fn test_limit_buy_price_time_priority() {
	let mut engine = create_match_engine();

	// 盘口：相同价格的两个卖单（时间优先）
	let maker1 = create_test_order("maker1", OrderSide::Sell, 600, 50, 2);
	let maker2 = create_test_order("maker2", OrderSide::Sell, 600, 50, 3);
	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 限价买单，只能匹配 80
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 600, 80, "0.048", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 80);
	assert_eq!(result.matched_orders.len(), 2);
	// 第一个订单应该完全成交（50），第二个订单部分成交（30）
	assert_eq!(result.matched_orders[0].2, 50); // maker1 完全成交
	assert_eq!(result.matched_orders[1].2, 30); // maker2 部分成交
}

// ============================================================================
// 限价卖单测试
// ============================================================================

#[tokio::test]
async fn test_limit_sell_full_match_single_order() {
	let mut engine = create_match_engine();

	// 盘口：一个买单 @ 600, 数量 100
	let maker = create_test_order("maker1", OrderSide::Buy, 600, 100, 2);
	add_order_to_engine(&mut engine, maker);

	// 限价卖单 @ 600, 数量 100
	let taker = create_submit_message("taker", OrderSide::Sell, OrderType::Limit, 600, 100, "0.06", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 100);
	assert_eq!(result.remaining_quantity, 0);
	assert_eq!(result.filled_volume, Decimal::from_str("0.06").unwrap());
	assert_eq!(result.matched_orders.len(), 1);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_limit_sell_full_match_multiple_orders() {
	let mut engine = create_match_engine();

	// 盘口：三个买单（价格从高到低）
	let maker1 = create_test_order("maker1", OrderSide::Buy, 700, 50, 2);
	let maker2 = create_test_order("maker2", OrderSide::Buy, 650, 30, 3);
	let maker3 = create_test_order("maker3", OrderSide::Buy, 600, 40, 4);
	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);
	add_order_to_engine(&mut engine, maker3);

	// 限价卖单 @ 600, 数量 120
	let taker = create_submit_message("taker", OrderSide::Sell, OrderType::Limit, 600, 120, "0.0795", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 120);
	assert_eq!(result.remaining_quantity, 0);
	// 50*700 + 30*650 + 40*600 = 35000 + 19500 + 24000 = 78500 / 1000000 = 0.0785
	assert_eq!(result.filled_volume, Decimal::from_str("0.0785").unwrap());
	assert_eq!(result.matched_orders.len(), 3);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_limit_sell_partial_match() {
	let mut engine = create_match_engine();

	// 盘口：一个买单 @ 600, 数量 50
	let maker = create_test_order("maker1", OrderSide::Buy, 600, 50, 2);
	add_order_to_engine(&mut engine, maker);

	// 限价卖单 @ 600, 数量 100
	let taker = create_submit_message("taker", OrderSide::Sell, OrderType::Limit, 600, 100, "0.06", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 50);
	assert_eq!(result.remaining_quantity, 50);
	assert_eq!(result.filled_volume, Decimal::from_str("0.03").unwrap());
	assert_eq!(result.matched_orders.len(), 1);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_limit_sell_no_match_price_too_high() {
	let mut engine = create_match_engine();

	// 盘口：一个买单 @ 600
	let maker = create_test_order("maker1", OrderSide::Buy, 600, 100, 2);
	add_order_to_engine(&mut engine, maker);

	// 限价卖单 @ 700（高于买单价格）
	let taker = create_submit_message("taker", OrderSide::Sell, OrderType::Limit, 700, 100, "0.07", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 0);
	assert_eq!(result.remaining_quantity, 100);
	assert_eq!(result.filled_volume, Decimal::ZERO);
	assert_eq!(result.matched_orders.len(), 0);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_limit_sell_self_trade_detection() {
	let mut engine = create_match_engine();

	// 盘口：同一用户的买单
	let maker = create_test_order("maker1", OrderSide::Buy, 600, 100, 1);
	add_order_to_engine(&mut engine, maker);

	// 限价卖单，相同用户
	let taker = create_submit_message("taker", OrderSide::Sell, OrderType::Limit, 600, 100, "0.06", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert!(result.has_self_trade);
	assert_eq!(result.filled_quantity, 0);
	assert_eq!(result.remaining_quantity, 100);
}

// ============================================================================
// 市价买单测试
// ============================================================================

#[tokio::test]
async fn test_market_buy_full_match_single_order() {
	let mut engine = create_match_engine();

	// 盘口：一个卖单 @ 600, 数量 100
	let maker = create_test_order("maker1", OrderSide::Sell, 600, 100, 2);
	add_order_to_engine(&mut engine, maker);

	// 市价买单，volume = 0.06 USDC（可以买 100 个 @ 600）
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Market, 1000, 0, "0.06", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 100);
	assert_eq!(result.remaining_quantity, 0);
	assert_eq!(result.filled_volume, Decimal::from_str("0.06").unwrap());
	assert_eq!(result.remaining_volume, Decimal::ZERO);
	assert_eq!(result.matched_orders.len(), 1);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_market_buy_full_match_multiple_orders() {
	let mut engine = create_match_engine();

	// 盘口：三个卖单
	let maker1 = create_test_order("maker1", OrderSide::Sell, 500, 50, 2);
	let maker2 = create_test_order("maker2", OrderSide::Sell, 600, 100, 3);
	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 市价买单，volume = 0.085 USDC
	// 50 * 500 = 25000, 剩余 60000
	// 100 * 600 = 60000
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Market, 1000, 0, "0.085", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 150);
	assert_eq!(result.remaining_quantity, 0);
	assert_eq!(result.filled_volume, Decimal::from_str("0.085").unwrap());
	assert_eq!(result.matched_orders.len(), 2);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_market_buy_partial_match_volume_limit() {
	let mut engine = create_match_engine();

	// 盘口：一个卖单 @ 600, 数量 100
	let maker = create_test_order("maker1", OrderSide::Sell, 600, 100, 2);
	add_order_to_engine(&mut engine, maker);

	// 市价买单，volume = 0.03 USDC（只能买 50 个）
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Market, 1000, 0, "0.03", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 50);
	assert_eq!(result.filled_volume, Decimal::from_str("0.03").unwrap());
	assert_eq!(result.remaining_volume, Decimal::ZERO);
	assert_eq!(result.matched_orders.len(), 1);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_market_buy_no_match_empty_book() {
	let engine = create_match_engine();

	// 空盘口
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Market, 1000, 0, "0.1", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 0);
	assert_eq!(result.filled_volume, Decimal::ZERO);
	assert_eq!(result.remaining_volume, Decimal::from_str("0.1").unwrap());
	assert_eq!(result.matched_orders.len(), 0);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_market_buy_self_trade_detection() {
	let mut engine = create_match_engine();

	// 盘口：同一用户的卖单
	let maker = create_test_order("maker1", OrderSide::Sell, 600, 100, 1);
	add_order_to_engine(&mut engine, maker);

	// 市价买单，相同用户
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Market, 1000, 0, "0.1", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert!(result.has_self_trade);
	assert_eq!(result.filled_quantity, 0);
	assert_eq!(result.filled_volume, Decimal::ZERO);
}

#[tokio::test]
async fn test_market_buy_rounding_edge_case() {
	let mut engine = create_match_engine();

	// 盘口：一个卖单 @ 333
	let maker = create_test_order("maker1", OrderSide::Sell, 333, 1000, 2);
	add_order_to_engine(&mut engine, maker);

	// 市价买单，volume = 0.1 USDC
	// 0.1 USDC / (333/1000000) = 300.3 个，向下取整应该是 300
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Market, 1000, 0, "0.1", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 300);
	assert_eq!(result.filled_volume, Decimal::from_str("0.0999").unwrap()); // 300 * 333 / 1000000
	assert_eq!(result.matched_orders.len(), 1);
}

// ============================================================================
// 市价卖单测试
// ============================================================================

#[tokio::test]
async fn test_market_sell_full_match_single_order() {
	let mut engine = create_match_engine();

	// 盘口：一个买单 @ 600, 数量 100
	let maker = create_test_order("maker1", OrderSide::Buy, 600, 100, 2);
	add_order_to_engine(&mut engine, maker);

	// 市价卖单，quantity = 100, price = 500（价格下限）
	let taker = create_submit_message("taker", OrderSide::Sell, OrderType::Market, 500, 100, "0.05", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 100);
	assert_eq!(result.remaining_quantity, 0);
	assert_eq!(result.filled_volume, Decimal::from_str("0.06").unwrap()); // 100 * 600 / 1000000
	assert_eq!(result.matched_orders.len(), 1);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_market_sell_full_match_multiple_orders() {
	let mut engine = create_match_engine();

	// 盘口：三个买单（价格从高到低）
	let maker1 = create_test_order("maker1", OrderSide::Buy, 700, 50, 2);
	let maker2 = create_test_order("maker2", OrderSide::Buy, 600, 100, 3);
	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 市价卖单，quantity = 150, price = 500（价格下限）
	let taker = create_submit_message("taker", OrderSide::Sell, OrderType::Market, 500, 150, "0.075", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 150);
	assert_eq!(result.remaining_quantity, 0);
	// 50*700 + 100*600 = 35000 + 60000 = 95000 / 1000000 = 0.095
	assert_eq!(result.filled_volume, Decimal::from_str("0.095").unwrap());
	assert_eq!(result.matched_orders.len(), 2);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_market_sell_partial_match() {
	let mut engine = create_match_engine();

	// 盘口：一个买单 @ 600, 数量 50
	let maker = create_test_order("maker1", OrderSide::Buy, 600, 50, 2);
	add_order_to_engine(&mut engine, maker);

	// 市价卖单，quantity = 100, price = 500
	let taker = create_submit_message("taker", OrderSide::Sell, OrderType::Market, 500, 100, "0.05", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 50);
	assert_eq!(result.remaining_quantity, 50);
	assert_eq!(result.filled_volume, Decimal::from_str("0.03").unwrap());
	assert_eq!(result.matched_orders.len(), 1);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_market_sell_price_floor_enforcement() {
	let mut engine = create_match_engine();

	// 盘口：两个买单
	let maker1 = create_test_order("maker1", OrderSide::Buy, 600, 50, 2);
	let maker2 = create_test_order("maker2", OrderSide::Buy, 400, 100, 3); // 低于价格下限
	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 市价卖单，quantity = 150, price = 500（价格下限）
	let taker = create_submit_message("taker", OrderSide::Sell, OrderType::Market, 500, 150, "0.075", 1);

	let result = engine.get_cross_matching_orders(&taker);

	// 只应该匹配第一个订单（600 >= 500），第二个订单被价格下限拒绝
	assert_eq!(result.filled_quantity, 50);
	assert_eq!(result.remaining_quantity, 100);
	assert_eq!(result.filled_volume, Decimal::from_str("0.03").unwrap());
	assert_eq!(result.matched_orders.len(), 1);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_market_sell_no_match_empty_book() {
	let engine = create_match_engine();

	// 空盘口
	let taker = create_submit_message("taker", OrderSide::Sell, OrderType::Market, 500, 100, "0.05", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 0);
	assert_eq!(result.remaining_quantity, 100);
	assert_eq!(result.filled_volume, Decimal::ZERO);
	assert_eq!(result.matched_orders.len(), 0);
	assert!(!result.has_self_trade);
}

#[tokio::test]
async fn test_market_sell_self_trade_detection() {
	let mut engine = create_match_engine();

	// 盘口：同一用户的买单
	let maker = create_test_order("maker1", OrderSide::Buy, 600, 100, 1);
	add_order_to_engine(&mut engine, maker);

	// 市价卖单，相同用户
	let taker = create_submit_message("taker", OrderSide::Sell, OrderType::Market, 500, 100, "0.05", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert!(result.has_self_trade);
	assert_eq!(result.filled_quantity, 0);
	assert_eq!(result.remaining_quantity, 100);
}

// ============================================================================
// 边缘情况测试
// ============================================================================

#[tokio::test]
async fn test_edge_case_zero_quantity() {
	let engine = create_match_engine();

	// 限价买单，数量为 0
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 600, 0, "0", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 0);
	assert_eq!(result.remaining_quantity, 0);
	assert_eq!(result.matched_orders.len(), 0);
}

#[tokio::test]
async fn test_edge_case_zero_volume_market_buy() {
	let mut engine = create_match_engine();

	// 盘口有订单
	let maker = create_test_order("maker1", OrderSide::Sell, 600, 100, 2);
	add_order_to_engine(&mut engine, maker);

	// 市价买单，volume = 0
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Market, 1000, 0, "0", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 0);
	assert_eq!(result.filled_volume, Decimal::ZERO);
	assert_eq!(result.matched_orders.len(), 0);
}

#[tokio::test]
async fn test_edge_case_very_small_volume() {
	let mut engine = create_match_engine();

	// 盘口：一个卖单 @ 600
	let maker = create_test_order("maker1", OrderSide::Sell, 600, 100, 2);
	add_order_to_engine(&mut engine, maker);

	// 市价买单，volume = 0.0001 USDC（只能买不到 1 个）
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Market, 1000, 0, "0.0001", 1);

	let result = engine.get_cross_matching_orders(&taker);

	// volume / price = 0.0001 / 0.0006 = 0.1666... 向下取整 = 0
	assert_eq!(result.filled_quantity, 0);
	assert_eq!(result.remaining_volume, Decimal::from_str("0.0001").unwrap());
}

#[tokio::test]
async fn test_edge_case_exact_volume_match() {
	let mut engine = create_match_engine();

	// 盘口：一个卖单 @ 600, 数量 100
	let maker = create_test_order("maker1", OrderSide::Sell, 600, 100, 2);
	add_order_to_engine(&mut engine, maker);

	// 市价买单，volume 正好等于 100 * 600 / 1000000
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Market, 1000, 0, "0.06", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 100);
	assert_eq!(result.remaining_volume, Decimal::ZERO);
	assert_eq!(result.filled_volume, Decimal::from_str("0.06").unwrap());
}

#[tokio::test]
async fn test_edge_case_large_numbers() {
	let mut engine = create_match_engine();

	// 盘口：大数量订单
	let maker = create_test_order("maker1", OrderSide::Sell, 999, 1_000_000, 2);
	add_order_to_engine(&mut engine, maker);

	// 限价买单，大数量
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 999, 1_000_000, "999", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 1_000_000);
	assert_eq!(result.remaining_quantity, 0);
	assert_eq!(result.filled_volume, Decimal::from_str("999").unwrap()); // 1000000 * 999 / 1000000
	assert_eq!(result.matched_orders.len(), 1);
}

#[tokio::test]
async fn test_edge_case_price_boundary_max() {
	let mut engine = create_match_engine();

	// 盘口：价格 = 9990（最大值）
	let maker = create_test_order("maker1", OrderSide::Sell, 9990, 100, 2);
	add_order_to_engine(&mut engine, maker);

	// 限价买单 @ 9990
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 9990, 100, "0.999", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 100);
	assert_eq!(result.filled_volume, Decimal::from_str("0.999").unwrap()); // 100 * 9990 / 1000000
	assert_eq!(result.matched_orders.len(), 1);
}

#[tokio::test]
async fn test_edge_case_price_boundary_min() {
	let mut engine = create_match_engine();

	// 盘口：价格 = 10（最小值）
	let maker = create_test_order("maker1", OrderSide::Sell, 10, 100_000, 2);
	add_order_to_engine(&mut engine, maker);

	// 限价买单 @ 10
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 10, 100_000, "1", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 100_000);
	assert_eq!(result.filled_volume, Decimal::from_str("1").unwrap()); // 100000 * 10 / 1000000 = 1
	assert_eq!(result.matched_orders.len(), 1);
}

#[tokio::test]
async fn test_multiple_orders_same_price_time_priority() {
	let mut engine = create_match_engine();

	// 盘口：五个相同价格的卖单
	for i in 1..=5 {
		let maker = create_test_order(&format!("maker{}", i), OrderSide::Sell, 600, 20, i + 1);
		add_order_to_engine(&mut engine, maker);
	}

	// 限价买单，只能匹配 70 个
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 600, 70, "0.042", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert_eq!(result.filled_quantity, 70);
	assert_eq!(result.matched_orders.len(), 4);
	// 前三个订单完全成交（20+20+20=60），第四个订单部分成交（10）
	assert_eq!(result.matched_orders[0].2, 20);
	assert_eq!(result.matched_orders[1].2, 20);
	assert_eq!(result.matched_orders[2].2, 20);
	assert_eq!(result.matched_orders[3].2, 10);
}

#[tokio::test]
async fn test_complex_scenario_mixed_prices_and_users() {
	let mut engine = create_match_engine();

	// 盘口：复杂场景
	// 不同价格、不同用户、包含潜在自成交
	let maker1 = create_test_order("maker1", OrderSide::Sell, 500, 30, 2);
	let maker2 = create_test_order("maker2", OrderSide::Sell, 550, 40, 3);
	let maker3 = create_test_order("maker3", OrderSide::Sell, 600, 50, 1); // 同一用户
	let maker4 = create_test_order("maker4", OrderSide::Sell, 650, 60, 4);
	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);
	add_order_to_engine(&mut engine, maker3);
	add_order_to_engine(&mut engine, maker4);

	// 限价买单 @ 600，应该匹配到 maker3 时停止（自成交）
	let taker = create_submit_message("taker", OrderSide::Buy, OrderType::Limit, 600, 200, "0.12", 1);

	let result = engine.get_cross_matching_orders(&taker);

	assert!(result.has_self_trade);
	assert_eq!(result.filled_quantity, 70); // maker1(30) + maker2(40)
	assert_eq!(result.remaining_quantity, 130);
	assert_eq!(result.matched_orders.len(), 2);
	// 30*500 + 40*550 = 15000 + 22000 = 37000 / 1000000 = 0.037
	assert_eq!(result.filled_volume, Decimal::from_str("0.037").unwrap());
}
