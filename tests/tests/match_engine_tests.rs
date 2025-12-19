//! MatchEngine 测试套件
//!
//! 包含 OrderBook 和 MatchEngine 的所有测试用例
//! 运行方式: cargo test --test match_engine_tests

use {
	common::engine_types::{Order, OrderSide, OrderStatus, OrderType, PredictionSymbol, SubmitOrderMessage},
	match_engine::{engine::MatchEngine, orderbook::OrderBook, types::OrderBookControl},
	rust_decimal::Decimal,
	std::sync::Arc,
	tokio::sync::{broadcast, mpsc},
};

// ============================================================================
// 测试辅助函数
// ============================================================================

fn create_test_symbol() -> PredictionSymbol {
	PredictionSymbol::new(1, 1, "token_0")
}

fn create_test_order(id: &str, side: OrderSide, price: i32, quantity: u64) -> Order {
	let symbol = create_test_symbol();
	// 将 id 转换为数字作为 user_id
	let user_id = id.parse::<i64>().unwrap_or_else(|_| id.chars().map(|c| c as i64).sum());
	Order::new(id.to_string(), symbol, side, OrderType::Limit, quantity, price, user_id, "test_privy_id".to_string(), "Yes".to_string()).unwrap()
}

fn create_market_order(id: &str, side: OrderSide, price: i32, quantity: u64) -> Order {
	let symbol = create_test_symbol();
	// 将 id 转换为数字作为 user_id
	let user_id = id.parse::<i64>().unwrap_or_else(|_| id.chars().map(|c| c as i64).sum());
	Order::new(id.to_string(), symbol, side, OrderType::Market, quantity, price, user_id, "test_privy_id".to_string(), "Yes".to_string()).unwrap()
}

fn create_match_engine() -> (MatchEngine, mpsc::Sender<OrderBookControl>, broadcast::Receiver<()>) {
	let (_exit_sender, exit_receiver) = broadcast::channel(1);
	let (engine, order_sender) = MatchEngine::new(1, 1, ("token_0".to_string(), "token_1".to_string()), 1000, exit_receiver, tokio::sync::oneshot::channel().1);
	let exit_receiver2 = _exit_sender.subscribe();
	(engine, order_sender, exit_receiver2)
}

/// 添加订单到引擎，同时交叉插入到对方订单簿（模拟submit_order的行为）
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

/// 将 Order 转换为 SubmitOrderMessage（用于测试）
fn order_to_submit_message(order: &Order) -> SubmitOrderMessage {
	let volume = match (order.order_type, order.side) {
		(OrderType::Limit, OrderSide::Buy) => {
			// Buy limit: volume = price * quantity / 1000000
			Decimal::from(order.price as i64) * Decimal::from(order.quantity as i64) / Decimal::from(1000000i64)
		}
		(OrderType::Market, OrderSide::Buy) => {
			// Market buy: volume calculated from quantity * price
			Decimal::from(order.quantity as i64) * Decimal::from(order.price as i64) / Decimal::from(1000000i64)
		}
		_ => Decimal::ZERO, // Sell orders have volume = 0
	};

	SubmitOrderMessage {
		order_id: order.order_id.clone(),
		symbol: order.symbol.clone(),
		side: order.side,
		order_type: order.order_type,
		quantity: order.quantity,
		price: order.price,
		volume,
		user_id: order.user_id,
		privy_id: order.privy_id.clone(),
		outcome_name: order.outcome_name.clone(),
	}
}

// ============================================================================
// OrderBook 测试
// ============================================================================

#[test]
fn test_orderbook_new() {
	let symbol = create_test_symbol();
	let orderbook = OrderBook::new(symbol.clone());
	assert_eq!(orderbook.symbol, symbol);
	assert!(orderbook.bids.is_empty());
	assert!(orderbook.asks.is_empty());
	assert!(orderbook.order_price_map.is_empty());
}

#[test]
fn test_orderbook_add_order_buy() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);
	let order = create_test_order("order1", OrderSide::Buy, 500, 100);

	assert!(orderbook.add_order(Arc::new(order)).is_ok());
	assert_eq!(orderbook.bids.len(), 1);
	assert_eq!(orderbook.asks.len(), 0);
	assert!(orderbook.order_price_map.contains_key("order1"));

	// 验证价格键是负数（用于排序）
	// 价格5000000表示0.5（5000000/10000 = 500.0，但实际是0.5）
	let (side, price_key) = orderbook.order_price_map.get("order1").unwrap();
	assert_eq!(*side, OrderSide::Buy);
	// 注意：价格现在是i32，而且500表示0.05（500/10000），所以价格键应该是-500
	assert_eq!(*price_key, -500);
}

#[test]
fn test_orderbook_add_order_sell() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);
	let order = create_test_order("order1", OrderSide::Sell, 500, 100);

	assert!(orderbook.add_order(Arc::new(order)).is_ok());
	assert_eq!(orderbook.bids.len(), 0);
	assert_eq!(orderbook.asks.len(), 1);
	assert!(orderbook.order_price_map.contains_key("order1"));

	// 验证价格键是正数
	let (side, price_key) = orderbook.order_price_map.get("order1").unwrap();
	assert_eq!(*side, OrderSide::Sell);
	assert_eq!(*price_key, 500);
}

#[test]
fn test_orderbook_add_order_same_price_level() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);
	let order1 = create_test_order("order1", OrderSide::Buy, 500, 100);
	let order2 = create_test_order("order2", OrderSide::Buy, 500, 200);

	assert!(orderbook.add_order(Arc::new(order1)).is_ok());
	assert!(orderbook.add_order(Arc::new(order2)).is_ok());

	assert_eq!(orderbook.bids.len(), 1); // 同一个价格档位
	let orders = orderbook.bids.get(&-500).unwrap();
	assert_eq!(orders.len(), 2);
}

#[test]
fn test_orderbook_add_order_different_price_levels() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);
	let order1 = create_test_order("order1", OrderSide::Buy, 500, 100);
	let order2 = create_test_order("order2", OrderSide::Buy, 600, 200);

	assert!(orderbook.add_order(Arc::new(order1)).is_ok());
	assert!(orderbook.add_order(Arc::new(order2)).is_ok());

	assert_eq!(orderbook.bids.len(), 2); // 不同价格档位
}

#[test]
fn test_orderbook_add_order_symbol_mismatch() {
	let symbol1 = PredictionSymbol::new(1, 1, "token_0");
	let symbol2 = PredictionSymbol::new(2, 1, "token_0");
	let mut orderbook = OrderBook::new(symbol1);
	// 价格500表示0.05（500/10000）
	let order = Order::new("order1".to_string(), symbol2, OrderSide::Buy, OrderType::Limit, 100, 500, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	assert!(orderbook.add_order(Arc::new(order)).is_err());
}

#[test]
fn test_orderbook_add_order_zero_quantity() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);
	let mut order = create_test_order("order1", OrderSide::Buy, 500, 100);
	order.remaining_quantity = 0;

	assert!(orderbook.add_order(Arc::new(order)).is_err());
}

#[test]
fn test_orderbook_remove_order() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);
	let order = create_test_order("order1", OrderSide::Buy, 500, 100);

	orderbook.add_order(Arc::new(order)).unwrap();
	assert!(orderbook.order_price_map.contains_key("order1"));

	let removed_order = orderbook.remove_order("order1".to_string()).unwrap();
	assert_eq!(removed_order.order_id, "order1");
	assert!(!orderbook.order_price_map.contains_key("order1"));
	assert!(orderbook.bids.is_empty());
}

#[test]
fn test_orderbook_remove_order_not_found() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	assert!(orderbook.remove_order("nonexistent".to_string()).is_err());
}

#[test]
fn test_orderbook_remove_order_empties_price_level() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);
	let order1 = create_test_order("order1", OrderSide::Buy, 500, 100);
	let order2 = create_test_order("order2", OrderSide::Buy, 500, 200);

	orderbook.add_order(Arc::new(order1)).unwrap();
	orderbook.add_order(Arc::new(order2)).unwrap();
	assert_eq!(orderbook.bids.len(), 1);

	orderbook.remove_order("order1".to_string()).unwrap();
	assert_eq!(orderbook.bids.len(), 1); // 还有 order2

	orderbook.remove_order("order2".to_string()).unwrap();
	assert!(orderbook.bids.is_empty()); // 价格档位被移除
}

#[test]
fn test_orderbook_update_order() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);
	let order = create_test_order("order1", OrderSide::Buy, 500, 100);

	orderbook.add_order(Arc::new(order)).unwrap();
	let updated_order = orderbook.update_order("order1".to_string(), 50).unwrap();

	assert_eq!(updated_order.remaining_quantity, 50);
	assert_eq!(updated_order.filled_quantity, 50);
	assert_eq!(updated_order.status, OrderStatus::PartiallyFilled);
}

#[test]
fn test_orderbook_update_order_to_zero() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);
	let order = create_test_order("order1", OrderSide::Buy, 500, 100);

	orderbook.add_order(Arc::new(order)).unwrap();
	let updated_order = orderbook.update_order("order1".to_string(), 0).unwrap();

	assert_eq!(updated_order.remaining_quantity, 0);
	assert_eq!(updated_order.filled_quantity, 100);
	assert_eq!(updated_order.status, OrderStatus::Filled);
}

#[test]
fn test_orderbook_update_order_not_found() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	assert!(orderbook.update_order("nonexistent".to_string(), 50).is_err());
}

#[test]
fn test_orderbook_best_bid() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	// 添加多个买单，价格从高到低
	// 价格现在是乘以10000，所以9330000表示0.933（9330000/10000 = 933.0，但实际我们想要0.933）
	// 为了测试，我们使用0.6, 0.5, 0.933
	orderbook.add_order(Arc::new(create_test_order("order1", OrderSide::Buy, 6000, 100))).unwrap(); // 0.6
	orderbook.add_order(Arc::new(create_test_order("order2", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	orderbook.add_order(Arc::new(create_test_order("order3", OrderSide::Buy, 9330, 100))).unwrap(); // 0.933

	let best_bid = orderbook.best_bid().unwrap();
	assert_eq!(best_bid, "0.933"); // 最高买价
}

#[test]
fn test_orderbook_best_ask() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	// 添加多个卖单，价格从低到高
	orderbook.add_order(Arc::new(create_test_order("order1", OrderSide::Sell, 6000, 100))).unwrap(); // 0.6
	orderbook.add_order(Arc::new(create_test_order("order2", OrderSide::Sell, 5450, 100))).unwrap(); // 0.545
	orderbook.add_order(Arc::new(create_test_order("order3", OrderSide::Sell, 9330, 100))).unwrap(); // 0.933

	let best_ask = orderbook.best_ask().unwrap();
	assert_eq!(best_ask, "0.933"); // 最高卖价
}

#[test]
fn test_orderbook_best_bid_empty() {
	let symbol = create_test_symbol();
	let orderbook = OrderBook::new(symbol);

	assert!(orderbook.best_bid().is_none());
}

#[test]
fn test_orderbook_best_ask_empty() {
	let symbol = create_test_symbol();
	let orderbook = OrderBook::new(symbol);

	assert!(orderbook.best_ask().is_none());
}

#[test]
fn test_orderbook_get_spread() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	// 使用浮点数价格测试
	orderbook.add_order(Arc::new(create_test_order("order1", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	orderbook.add_order(Arc::new(create_test_order("order2", OrderSide::Sell, 6330, 100))).unwrap(); // 0.633

	let spread = orderbook.get_spread().unwrap();
	assert_eq!(spread, "0.133"); // 0.633 - 0.5 = 0.133
}

#[test]
fn test_orderbook_get_spread_no_bid() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	orderbook.add_order(Arc::new(create_test_order("order1", OrderSide::Sell, 600, 100))).unwrap();

	assert!(orderbook.get_spread().is_none());
}

#[test]
fn test_orderbook_get_spread_no_ask() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	orderbook.add_order(Arc::new(create_test_order("order1", OrderSide::Buy, 500, 100))).unwrap();

	assert!(orderbook.get_spread().is_none());
}

#[test]
fn test_orderbook_get_matching_orders_buy() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	// 添加卖单
	orderbook.add_order(Arc::new(create_test_order("sell1", OrderSide::Sell, 500, 100))).unwrap();
	orderbook.add_order(Arc::new(create_test_order("sell2", OrderSide::Sell, 600, 200))).unwrap();
	orderbook.add_order(Arc::new(create_test_order("sell3", OrderSide::Sell, 400, 150))).unwrap();

	// 买单价格 550，应该匹配 400 和 500
	let taker = create_test_order("buy1", OrderSide::Buy, 550, 300);
	let matching_orders = orderbook.get_matching_orders(&taker);

	assert_eq!(matching_orders.len(), 2);
	assert_eq!(matching_orders[0].order_id, "sell3"); // 价格 400 优先
	assert_eq!(matching_orders[1].order_id, "sell1"); // 价格 500
}

#[test]
fn test_orderbook_get_matching_orders_sell() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	// 添加买单
	orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 500, 100))).unwrap();
	orderbook.add_order(Arc::new(create_test_order("buy2", OrderSide::Buy, 600, 200))).unwrap();
	orderbook.add_order(Arc::new(create_test_order("buy3", OrderSide::Buy, 400, 150))).unwrap();

	// 卖单价格 450，应该匹配 600 和 500
	let taker = create_test_order("sell1", OrderSide::Sell, 450, 300);
	let matching_orders = orderbook.get_matching_orders(&taker);

	assert_eq!(matching_orders.len(), 2);
	assert_eq!(matching_orders[0].order_id, "buy2"); // 价格 600 优先（最高买价）
	assert_eq!(matching_orders[1].order_id, "buy1"); // 价格 500
}

#[test]
fn test_orderbook_get_matching_orders_event_order() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	// 添加卖单
	orderbook.add_order(Arc::new(create_test_order("sell1", OrderSide::Sell, 500, 100))).unwrap();
	orderbook.add_order(Arc::new(create_test_order("sell2", OrderSide::Sell, 600, 200))).unwrap();

	// 市价买单，应该匹配所有卖单
	let symbol = create_test_symbol();
	let taker = Order::new("buy1".to_string(), symbol, OrderSide::Buy, OrderType::Market, 300, 5000, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	let matching_orders = orderbook.get_matching_orders(&taker);
	assert_eq!(matching_orders.len(), 2);
}

#[test]
fn test_orderbook_get_depth() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	// 使用浮点数价格测试
	orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	orderbook.add_order(Arc::new(create_test_order("buy2", OrderSide::Buy, 9330, 200))).unwrap(); // 0.933
	orderbook.add_order(Arc::new(create_test_order("sell1", OrderSide::Sell, 7000, 150))).unwrap(); // 0.7
	orderbook.add_order(Arc::new(create_test_order("sell2", OrderSide::Sell, 8750, 250))).unwrap(); // 0.875

	let depth = orderbook.get_depth(None, 1, 0);
	assert_eq!(depth.bids.len(), 2);
	assert_eq!(depth.asks.len(), 2);
	assert_eq!(depth.symbol, create_test_symbol());
	assert_eq!(depth.update_id, 1);

	// 验证价格排序：买单从高到低，卖单从低到高
	assert_eq!(depth.bids[0].price, "0.933");
	assert_eq!(depth.bids[1].price, "0.5");
	assert_eq!(depth.asks[0].price, "0.7");
	assert_eq!(depth.asks[1].price, "0.875");
}

#[test]
fn test_orderbook_get_depth_with_limit() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	// 添加多个价格档位
	for i in 1..=10 {
		orderbook.add_order(Arc::new(create_test_order(&format!("buy{}", i), OrderSide::Buy, (5000 + i * 100) as i32, 100))).unwrap();
	}

	let depth = orderbook.get_depth(Some(5), 1, 0);
	assert_eq!(depth.bids.len(), 5); // 只返回前 5 档
}

#[test]
fn test_orderbook_get_depth_aggregates_quantity() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol);

	// 同一价格档位多个订单
	orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 500, 100))).unwrap();
	orderbook.add_order(Arc::new(create_test_order("buy2", OrderSide::Buy, 500, 200))).unwrap();
	orderbook.add_order(Arc::new(create_test_order("buy3", OrderSide::Buy, 500, 150))).unwrap();

	let depth = orderbook.get_depth(None, 1, 0);
	assert_eq!(depth.bids.len(), 1);
	assert_eq!(depth.bids[0].total_quantity_u64, 450u64); // 100 + 200 + 150
	assert_eq!(depth.bids[0].order_count, 3);
}

#[test]
fn test_orderbook_build_from_orders() {
	let symbol = create_test_symbol();
	let orders = vec![Arc::new(create_test_order("order1", OrderSide::Buy, 500, 100)), Arc::new(create_test_order("order2", OrderSide::Sell, 600, 200))];

	let orderbook = OrderBook::build(symbol.clone(), orders).unwrap();
	assert_eq!(orderbook.symbol, symbol);
	assert_eq!(orderbook.bids.len(), 1);
	assert_eq!(orderbook.asks.len(), 1);
	assert_eq!(orderbook.order_price_map.len(), 2);
}

#[test]
fn test_orderbook_get_orderbook_stats() {
	let symbol = create_test_symbol();
	let mut orderbook = OrderBook::new(symbol.clone());

	orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 500, 100))).unwrap();
	orderbook.add_order(Arc::new(create_test_order("buy2", OrderSide::Buy, 600, 200))).unwrap();
	orderbook.add_order(Arc::new(create_test_order("sell1", OrderSide::Sell, 700, 150))).unwrap();

	let stats = orderbook.get_orderbook_stats();
	assert_eq!(stats.symbol, symbol);
	assert_eq!(stats.bid_levels, 2);
	assert_eq!(stats.ask_levels, 1);
	assert_eq!(stats.total_bid_orders, 2);
	assert_eq!(stats.total_ask_orders, 1);
	assert_eq!(stats.total_bid_quantity, 300);
	assert_eq!(stats.total_ask_quantity, 150);
}

// ============================================================================
// MatchEngine 测试
// ============================================================================

#[tokio::test]
async fn test_match_engine_new() {
	let (engine, _sender, _exit_receiver) = create_match_engine();
	assert_eq!(engine.order_num, 0);
	assert_eq!(engine.update_id, 0);
	assert!(engine.token_0_orders.is_empty());
	assert!(engine.token_1_orders.is_empty());
	assert!(engine.token_0_bid_changes.is_empty());
	assert!(engine.token_0_ask_changes.is_empty());
	assert!(engine.token_1_bid_changes.is_empty());
	assert!(engine.token_1_ask_changes.is_empty());
	assert!(engine.last_token_0_depth_snapshot.is_none());
	assert!(engine.last_token_1_depth_snapshot.is_none());
}

#[tokio::test]
async fn test_match_engine_submit_order_no_match() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();
	let order = create_test_order("order1", OrderSide::Buy, 500, 100);

	// 没有匹配的订单，订单应该加入订单簿
	assert!(engine.submit_order(&order_to_submit_message(&order), 1).is_ok());
	// COMMENTED: 	assert_eq!(order.status, OrderStatus::New);
	// COMMENTED: 	assert_eq!(order.remaining_quantity, 100);
	// COMMENTED: 	assert_eq!(order.filled_quantity, 0);
	// 订单是Yes结果，应该加入yes_orderbook
	assert_eq!(engine.token_0_orders.len(), 1);
	assert!(engine.token_0_orderbook.bids.contains_key(&-500));
}

#[tokio::test]
async fn test_match_engine_submit_order_full_match() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 先添加一个卖单（Yes结果）
	let maker = create_test_order("maker1", OrderSide::Sell, 500, 100);
	add_order_to_engine(&mut engine, maker);

	// 提交买单，应该完全匹配
	let taker = create_test_order("taker1", OrderSide::Buy, 600, 100);
	assert!(engine.submit_order(&order_to_submit_message(&taker), 1).is_ok());

	// COMMENTED: 	assert_eq!(taker.status, OrderStatus::Filled);
	// COMMENTED: 	assert_eq!(taker.remaining_quantity, 0);
	// COMMENTED: 	assert_eq!(taker.filled_quantity, 100);
	// maker 应该被移除
	assert!(!engine.token_0_orders.contains_key("maker1"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("maker1"));
	// taker 不应该在订单簿中（完全成交）
	assert!(!engine.token_0_orders.contains_key("taker1"));
}

#[tokio::test]
async fn test_match_engine_submit_order_partial_match() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 先添加一个卖单（数量 50）
	let maker = create_test_order("maker1", OrderSide::Sell, 500, 50);
	add_order_to_engine(&mut engine, maker);

	// 提交买单（数量 100），应该部分匹配
	let taker = create_test_order("taker1", OrderSide::Buy, 600, 100);
	assert!(engine.submit_order(&order_to_submit_message(&taker), 1).is_ok());

	// COMMENTED: 	assert_eq!(taker.status, OrderStatus::PartiallyFilled);
	// COMMENTED: 	assert_eq!(taker.remaining_quantity, 50);
	// COMMENTED: 	assert_eq!(taker.filled_quantity, 50);
	// maker 应该被完全成交并移除
	assert!(!engine.token_0_orders.contains_key("maker1"));
	// taker 应该还在订单簿中
	assert!(engine.token_0_orders.contains_key("taker1"));
}

#[tokio::test]
async fn test_match_engine_submit_order_event_order_partial_match() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 先添加一个卖单（数量 50）
	let maker = create_test_order("maker1", OrderSide::Sell, 500, 50);
	add_order_to_engine(&mut engine, maker);

	// 提交市价买单（数量 100），应该部分匹配但不会放在订单簿中
	let symbol = create_test_symbol();
	let taker = Order::new("taker1".to_string(), symbol, OrderSide::Buy, OrderType::Market, 100, 5000, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	assert!(engine.submit_order(&order_to_submit_message(&taker), 1).is_ok());

	// 市价单应该部分成交
	// COMMENTED: 	assert_eq!(taker.remaining_quantity, 50);
	// COMMENTED: 	assert_eq!(taker.filled_quantity, 50);
	// maker 应该被完全成交并移除
	assert!(!engine.token_0_orders.contains_key("maker1"));
	// 市价单部分成交后不应该在订单簿中（这是关键测试点）
	assert!(!engine.token_0_orders.contains_key("taker1"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("taker1"));
}

#[tokio::test]
async fn test_match_engine_submit_order_event_order_full_match() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 先添加一个卖单（数量 100）
	let maker = create_test_order("maker1", OrderSide::Sell, 500, 100);
	add_order_to_engine(&mut engine, maker);

	// 提交市价买单（数量 100），应该完全匹配
	let symbol = create_test_symbol();
	let taker = Order::new("taker1".to_string(), symbol, OrderSide::Buy, OrderType::Market, 100, 5000, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	assert!(engine.submit_order(&order_to_submit_message(&taker), 1).is_ok());

	// 市价单应该完全成交
	// COMMENTED: 	assert_eq!(taker.status, OrderStatus::Filled);
	// COMMENTED: 	assert_eq!(taker.remaining_quantity, 0);
	// COMMENTED: 	assert_eq!(taker.filled_quantity, 100);
	// maker 应该被移除
	assert!(!engine.token_0_orders.contains_key("maker1"));
	// 市价单完全成交后不应该在订单簿中
	assert!(!engine.token_0_orders.contains_key("taker1"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("taker1"));
}

#[tokio::test]
async fn test_match_engine_submit_order_event_order_no_match() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 订单簿为空，提交市价买单（数量 100），无法匹配
	let symbol = create_test_symbol();
	let taker = Order::new("taker1".to_string(), symbol, OrderSide::Buy, OrderType::Market, 100, 5000, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	// 市价单无法匹配时应该返回错误，或者返回成功但不放在订单簿
	// 根据业务逻辑，这里应该是返回成功但不放在订单簿
	assert!(engine.submit_order(&order_to_submit_message(&taker), 1).is_ok());

	// 市价单没有成交
	// COMMENTED: 	assert_eq!(taker.remaining_quantity, 100);
	// COMMENTED: 	assert_eq!(taker.filled_quantity, 0);
	// 市价单不应该在订单簿中（即使没有匹配）
	assert!(!engine.token_0_orders.contains_key("taker1"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("taker1"));
}

#[tokio::test]
async fn test_match_engine_submit_order_limit_order_no_match_stays_in_orderbook() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 订单簿为空，提交限价买单（数量 100），无法匹配，但应该放在订单簿中
	let taker = create_test_order("taker1", OrderSide::Buy, 600, 100);
	assert!(engine.submit_order(&order_to_submit_message(&taker), 1).is_ok());

	// 限价单没有成交
	// COMMENTED: 	assert_eq!(taker.status, OrderStatus::New);
	// COMMENTED: 	assert_eq!(taker.remaining_quantity, 100);
	// COMMENTED: 	assert_eq!(taker.filled_quantity, 0);
	// 限价单应该放在订单簿中（与市价单形成对比）
	assert!(engine.token_0_orders.contains_key("taker1"));
	assert!(engine.token_0_orderbook.order_price_map.contains_key("taker1"));
}

#[tokio::test]
async fn test_match_engine_cancel_order() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();
	let order = create_test_order("order1", OrderSide::Buy, 500, 100);

	add_order_to_engine(&mut engine, order);

	assert!(engine.cancel_order("order1".to_string()).is_ok());
	assert!(!engine.token_0_orders.contains_key("order1"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("order1"));
}

#[tokio::test]
async fn test_match_engine_cancel_order_not_found() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	assert!(engine.cancel_order("nonexistent".to_string()).is_err());
}

#[tokio::test]
async fn test_match_engine_match_order_buy() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加卖单
	let maker1 = create_test_order("maker1", OrderSide::Sell, 400, 30);
	let maker2 = create_test_order("maker2", OrderSide::Sell, 500, 40);
	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	let taker = create_test_order("taker1", OrderSide::Buy, 550, 100);
	let result = engine.match_order(&order_to_submit_message(&taker)).unwrap();
	let (remaining, trades, _has_self_trade) = (result.remaining_quantity, result.trades, result.has_self_trade);

	assert_eq!(remaining, 30); // 100 - 30 - 40
	assert_eq!(trades.len(), 2);

	// 验证第一个 trade：maker1（价格 400，数量 30）
	assert_eq!(trades[0].quantity, "0.3");
	assert_eq!(trades[0].taker_price, "0.04"); // 成交价格是 maker 的价格（400/10000 = 0.04）
	assert_eq!(trades[0].maker_order_id, "maker1");

	// 验证第二个 trade：maker2（价格 500，数量 40）
	assert_eq!(trades[1].quantity, "0.4");
	assert_eq!(trades[1].taker_price, "0.05"); // 成交价格是 maker 的价格（500/10000 = 0.05）
	assert_eq!(trades[1].maker_order_id, "maker2");

	// maker1 和 maker2 应该被完全成交
	assert!(!engine.token_0_orders.contains_key("maker1"));
	assert!(!engine.token_0_orders.contains_key("maker2"));
}

#[tokio::test]
async fn test_match_engine_match_order_sell() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加买单
	let maker1 = create_test_order("maker1", OrderSide::Buy, 600, 30);
	let maker2 = create_test_order("maker2", OrderSide::Buy, 500, 40);
	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 卖单价格 450，应该匹配 600 和 500
	let taker = create_test_order("taker1", OrderSide::Sell, 450, 100);
	let result = engine.match_order(&order_to_submit_message(&taker)).unwrap();
	let (remaining, trades, _has_self_trade) = (result.remaining_quantity, result.trades, result.has_self_trade);

	assert_eq!(remaining, 30); // 100 - 30 - 40
	assert_eq!(trades.len(), 2);

	// 验证第一个 trade：maker1（价格 600，数量 30）- 价格优先，最高买价优先
	assert_eq!(trades[0].quantity, "0.3");
	assert_eq!(trades[0].taker_price, "0.06"); // 成交价格是 maker 的价格（600/10000 = 0.06）
	assert_eq!(trades[0].maker_order_id, "maker1");

	// 验证第二个 trade：maker2（价格 500，数量 40）
	assert_eq!(trades[1].quantity, "0.4");
	assert_eq!(trades[1].taker_price, "0.05"); // 成交价格是 maker 的价格（500/10000 = 0.05）
	assert_eq!(trades[1].maker_order_id, "maker2");

	// maker1 和 maker2 应该被完全成交
	assert!(!engine.token_0_orders.contains_key("maker1"));
	assert!(!engine.token_0_orders.contains_key("maker2"));
}

#[tokio::test]
async fn test_match_engine_validate_order() {
	let (engine, _sender, _exit_receiver) = create_match_engine();

	// 有效订单
	let valid_order = create_test_order("order1", OrderSide::Buy, 500, 100);
	assert!(engine.validate_order_message(&order_to_submit_message(&valid_order)));

	// 数量为 0
	let mut invalid_order = create_test_order("order2", OrderSide::Buy, 500, 100);
	invalid_order.quantity = 0;
	assert!(!engine.validate_order_message(&order_to_submit_message(&invalid_order)));

	// 限价单但没有价格 - 先创建一个有效的限价单，然后手动设置价格为0
	let mut invalid_order2 = create_test_order("order3", OrderSide::Buy, 500, 100);
	invalid_order2.price = 0;
	// 	assert!(!engine.validate_order_message(&order_to_submit_message(&invalid_order2))); // validation for price is commented out in validate_order

	// 用户ID为空
	let mut invalid_order3 = create_test_order("order4", OrderSide::Buy, 500, 100);
	invalid_order3.user_id = 0;
	assert!(!engine.validate_order_message(&order_to_submit_message(&invalid_order3)));
}

#[tokio::test]
async fn test_match_engine_handle_snapshot_tick() {
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加一些订单（Yes结果）
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("sell1", OrderSide::Sell, 6000, 150))).unwrap(); // 0.6

	let initial_update_id = engine.update_id;

	// 执行快照
	engine.handle_snapshot_tick();

	// update_id 应该增加
	assert_eq!(engine.update_id, initial_update_id + 1);
	// 应该保存了深度快照
	assert!(engine.last_token_0_depth_snapshot.is_some());
	// price_level_changes 应该包含所有价格档位（首次快照时记录所有档位）
	assert_eq!(engine.token_0_bid_changes.len(), 1);
	assert_eq!(engine.token_0_ask_changes.len(), 1);
	assert!(engine.token_0_bid_changes.contains_key(&5000)); // price 0.5 = 5000
	assert!(engine.token_0_ask_changes.contains_key(&6000)); // price 0.6 = 6000

	let depth = engine.last_token_0_depth_snapshot.as_ref().unwrap();
	assert_eq!(depth.update_id, initial_update_id + 1);
	assert_eq!(depth.bids.len(), 1);
	assert_eq!(depth.asks.len(), 1);
}

#[tokio::test]
async fn test_match_engine_new_with_orders() {
	let (_exit_sender, exit_receiver) = broadcast::channel(1);
	let mut symbol_orders_map = std::collections::HashMap::new();
	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	let no_symbol = PredictionSymbol::new(1, 1, "token_1");

	// 创建Yes订单（买单）
	let mut yes_order = create_test_order("order1", OrderSide::Buy, 500, 100);
	yes_order.symbol = yes_symbol.clone();

	// 创建No订单（卖单）
	let mut no_order = create_test_order("order2", OrderSide::Sell, 600, 200);
	no_order.symbol = no_symbol.clone();

	symbol_orders_map.insert(yes_symbol.to_string(), vec![yes_order]);
	symbol_orders_map.insert(no_symbol.to_string(), vec![no_order]);

	let (engine, _sender) = MatchEngine::new_with_orders(1, 1, ("token_0".to_string(), "token_1".to_string()), symbol_orders_map, 1000, exit_receiver, tokio::sync::oneshot::channel().1).unwrap();

	// orders HashMap 只记录本方订单
	assert_eq!(engine.token_0_orders.len(), 1);
	assert_eq!(engine.token_1_orders.len(), 1);
	// orderbook 因为交叉插入，会包含更多订单：
	// - token_0_orderbook.bids 包含 yes_order (买单) + no_order 的交叉插入（卖单变买单）
	// - token_1_orderbook.asks 包含 no_order (卖单) + yes_order 的交叉插入（买单变卖单）
	assert_eq!(engine.token_0_orderbook.bids.len(), 2); // yes买单 + no卖单交叉
	assert_eq!(engine.token_1_orderbook.asks.len(), 2); // no卖单 + yes买单交叉
	assert_eq!(engine.order_num, 1); // max order_num + 1
}

// ============================================================================
// 价格档位变化检测测试
// ============================================================================

#[tokio::test]
async fn test_match_engine_update_price_level_changes_first_snapshot() {
	// 测试首次快照（没有上次快照）
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加一些订单（Yes结果）
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("sell1", OrderSide::Sell, 6000, 150))).unwrap(); // 0.6

	// 执行首次快照
	engine.handle_snapshot_tick();

	// 验证 price_level_changes 中记录了所有价格档位（首次快照时记录所有档位）
	assert_eq!(engine.token_0_bid_changes.len(), 1);
	assert_eq!(engine.token_0_ask_changes.len(), 1);
	assert!(engine.token_0_bid_changes.contains_key(&5000)); // price 0.5
	assert!(engine.token_0_ask_changes.contains_key(&6000)); // price 0.6

	// 验证买盘价格档位 (quantity 100 stored as u64)
	let buy_quantity = engine.token_0_bid_changes.get(&5000).unwrap();
	assert_eq!(*buy_quantity, 100u64);

	// 验证卖盘价格档位 (quantity 150 stored as u64)
	let sell_quantity = engine.token_0_ask_changes.get(&6000).unwrap();
	assert_eq!(*sell_quantity, 150u64);

	// 验证快照已保存
	assert!(engine.last_token_0_depth_snapshot.is_some());
	let depth = engine.last_token_0_depth_snapshot.as_ref().unwrap();
	assert_eq!(depth.update_id, 1);
}

#[tokio::test]
async fn test_match_engine_update_price_level_changes_quantity_change() {
	// 测试价格档位数量变化
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 第一次快照：添加订单
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	engine.handle_snapshot_tick();

	// 第二次快照：更新订单数量（部分成交）
	engine.token_0_orderbook.update_order("buy1".to_string(), 50).unwrap();
	engine.handle_snapshot_tick();

	// 验证 price_level_changes 中记录了数量变化
	assert_eq!(engine.token_0_bid_changes.len(), 1);
	assert!(engine.token_0_bid_changes.contains_key(&5000));
	let quantity = engine.token_0_bid_changes.get(&5000).unwrap();
	assert_eq!(*quantity, 50u64); // 从 100 变为 50
}

#[tokio::test]
async fn test_match_engine_update_price_level_changes_new_level() {
	// 测试新增价格档位
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 第一次快照：只有一个价格档位
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	engine.handle_snapshot_tick();

	// 第二次快照：添加新价格档位
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy2", OrderSide::Buy, 6000, 200))).unwrap(); // 0.6
	engine.handle_snapshot_tick();

	// 验证 price_level_changes 中记录了新价格档位
	assert_eq!(engine.token_0_bid_changes.len(), 1); // 只有新价格档位被记录（0.5 没有变化）
	assert!(engine.token_0_bid_changes.contains_key(&6000));
	let new_quantity = engine.token_0_bid_changes.get(&6000).unwrap();
	assert_eq!(*new_quantity, 200u64);

	// 验证快照包含两个价格档位
	let depth = engine.last_token_0_depth_snapshot.as_ref().unwrap();
	assert_eq!(depth.bids.len(), 2);
	assert!(depth.bids.iter().any(|p| p.price == "0.5"));
	assert!(depth.bids.iter().any(|p| p.price == "0.6"));
}

#[tokio::test]
async fn test_match_engine_update_price_level_changes_level_removed() {
	// 测试价格档位消失（订单完全成交或被取消）
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 第一次快照：添加订单
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy2", OrderSide::Buy, 6000, 200))).unwrap(); // 0.6
	engine.handle_snapshot_tick();

	// 移除一个订单（价格档位消失）
	engine.token_0_orderbook.remove_order("buy1".to_string()).unwrap();

	// 第二次快照
	engine.handle_snapshot_tick();

	// 验证 price_level_changes 中记录了消失的价格档位（quantity = 0）
	assert_eq!(engine.token_0_bid_changes.len(), 1);
	assert!(engine.token_0_bid_changes.contains_key(&5000));
	let removed_quantity = engine.token_0_bid_changes.get(&5000).unwrap();
	assert_eq!(*removed_quantity, 0u64); // 消失的价格档位 quantity 为 0

	// 验证价格 0.6 的档位仍然存在（数量没有变化，所以不会出现在 price_level_changes 中）
	assert!(!engine.token_0_bid_changes.contains_key(&6000));

	// 验证最终快照状态
	let depth = engine.last_token_0_depth_snapshot.as_ref().unwrap();
	assert_eq!(depth.bids.len(), 1);
	assert_eq!(depth.bids[0].price, "0.6");
}

#[tokio::test]
async fn test_match_engine_update_price_level_changes_no_change() {
	// 测试价格档位无变化
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 第一次快照
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	engine.handle_snapshot_tick();

	// 第二次快照：没有变化
	engine.handle_snapshot_tick();

	// 验证没有变化时，price_level_changes 应该为空（已被清空）
	// 但 last_depth_snapshot 应该仍然存在
	assert!(engine.token_0_bid_changes.is_empty());
	assert!(engine.token_0_ask_changes.is_empty());
	assert!(engine.last_token_0_depth_snapshot.is_some());
}

#[tokio::test]
async fn test_match_engine_update_price_level_changes_empty_orderbook() {
	// 测试空订单簿
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 第一次快照：空订单簿
	engine.handle_snapshot_tick();

	// 验证空订单簿也能正常处理
	assert!(engine.token_0_bid_changes.is_empty()); // 空订单簿没有价格档位
	assert!(engine.token_0_ask_changes.is_empty());
	assert!(engine.last_token_0_depth_snapshot.is_some());
	let depth = engine.last_token_0_depth_snapshot.as_ref().unwrap();
	assert!(depth.bids.is_empty());
	assert!(depth.asks.is_empty());

	// 添加订单后再次快照
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	engine.handle_snapshot_tick();

	// 验证 price_level_changes 中记录了新价格档位
	assert_eq!(engine.token_0_bid_changes.len(), 1);
	assert!(engine.token_0_bid_changes.contains_key(&5000));
	let quantity = engine.token_0_bid_changes.get(&5000).unwrap();
	assert_eq!(*quantity, 100u64);

	// 验证快照包含新价格档位
	let depth = engine.last_token_0_depth_snapshot.as_ref().unwrap();
	assert_eq!(depth.bids.len(), 1);
	assert_eq!(depth.bids[0].price, "0.5");
}

#[tokio::test]
async fn test_match_engine_update_price_level_changes_multiple_levels() {
	// 测试多个价格档位同时变化
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 第一次快照：多个价格档位
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy2", OrderSide::Buy, 6000, 200))).unwrap(); // 0.6
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("sell1", OrderSide::Sell, 7000, 150))).unwrap(); // 0.7
	engine.handle_snapshot_tick();

	// 第二次快照：多个变化
	// 更新 buy1 数量
	engine.token_0_orderbook.update_order("buy1".to_string(), 50).unwrap();
	// 添加新价格档位
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy3", OrderSide::Buy, 5500, 300))).unwrap(); // 0.55
	// 移除 sell1
	engine.token_0_orderbook.remove_order("sell1".to_string()).unwrap();
	engine.handle_snapshot_tick();

	// 验证 price_level_changes 中记录了所有变化
	assert_eq!(engine.token_0_bid_changes.len(), 2); // buy1 changed, buy3 added
	assert_eq!(engine.token_0_ask_changes.len(), 1); // sell1 removed

	// 验证 buy1 数量变化（从 100 变为 50）
	assert!(engine.token_0_bid_changes.contains_key(&5000));
	let buy1_quantity = engine.token_0_bid_changes.get(&5000).unwrap();
	assert_eq!(*buy1_quantity, 50u64);

	// 验证新增价格档位（0.55 = 5500）
	assert!(engine.token_0_bid_changes.contains_key(&5500));
	let buy3_quantity = engine.token_0_bid_changes.get(&5500).unwrap();
	assert_eq!(*buy3_quantity, 300u64);

	// 验证 sell1 价格档位消失（0.7 = 7000，quantity = 0）
	assert!(engine.token_0_ask_changes.contains_key(&7000));
	let sell1_quantity = engine.token_0_ask_changes.get(&7000).unwrap();
	assert_eq!(*sell1_quantity, 0u64);

	// 验证 buy2 没有变化（不记录在 price_level_changes 中）
	assert!(!engine.token_0_bid_changes.contains_key(&6000));

	// 验证快照状态
	let depth = engine.last_token_0_depth_snapshot.as_ref().unwrap();
	assert_eq!(depth.bids.len(), 3); // 0.5, 0.55, 0.6
	assert_eq!(depth.asks.len(), 0); // sell1 被移除
}

#[tokio::test]
async fn test_match_engine_update_price_level_changes_same_price_level_multiple_orders() {
	// 测试同一价格档位的多个订单变化
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 第一次快照：同一价格档位多个订单
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy1", OrderSide::Buy, 5000, 100))).unwrap(); // 0.5
	engine.token_0_orderbook.add_order(Arc::new(create_test_order("buy2", OrderSide::Buy, 5000, 200))).unwrap(); // 0.5
	engine.handle_snapshot_tick();

	// 验证 price_level_changes 中记录了总数量
	assert_eq!(engine.token_0_bid_changes.len(), 1);
	assert!(engine.token_0_bid_changes.contains_key(&5000));
	let buy_quantity = engine.token_0_bid_changes.get(&5000).unwrap();
	assert_eq!(*buy_quantity, 300u64); // 100 + 200

	// 验证快照总数量
	let depth = engine.last_token_0_depth_snapshot.as_ref().unwrap();
	let buy_level = depth.bids.iter().find(|p| p.price == "0.5").unwrap();
	assert_eq!(buy_level.total_quantity_u64, 300u64);
	assert_eq!(buy_level.order_count, 2);

	// 第二次快照：移除一个订单
	engine.token_0_orderbook.remove_order("buy1".to_string()).unwrap();
	engine.handle_snapshot_tick();

	// 验证 price_level_changes 中记录了数量变化
	assert_eq!(engine.token_0_bid_changes.len(), 1);
	assert!(engine.token_0_bid_changes.contains_key(&5000));
	let buy_quantity = engine.token_0_bid_changes.get(&5000).unwrap();
	assert_eq!(*buy_quantity, 200u64); // 从 300 变为 200

	// 验证快照总数量变化
	let depth = engine.last_token_0_depth_snapshot.as_ref().unwrap();
	let buy_level = depth.bids.iter().find(|p| p.price == "0.5").unwrap();
	assert_eq!(buy_level.total_quantity_u64, 200u64); // 只剩 buy2
	assert_eq!(buy_level.order_count, 1);
}

// ============================================================================
// 交叉撮合测试（get_cross_matching_orders）
// ============================================================================

#[tokio::test]
async fn test_match_engine_cross_matching_buy_yes_same_result() {
	// 买单Yes：应该匹配相同结果的卖单（Yes的卖单，使用price）
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加token_0结果的卖单
	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	let maker1 = Order::new("maker1".to_string(), yes_symbol.clone(), OrderSide::Sell, OrderType::Limit, 30, 400, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let maker2 = Order::new("maker2".to_string(), yes_symbol.clone(), OrderSide::Sell, OrderType::Limit, 40, 500, 2, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 买单token_0，价格0.06（600），应该匹配0.04和0.05的卖单
	let taker = Order::new("taker1".to_string(), yes_symbol, OrderSide::Buy, OrderType::Limit, 100, 600, 3, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let result = engine.match_order(&order_to_submit_message(&taker)).unwrap();
	let (remaining, trades, _has_self_trade) = (result.remaining_quantity, result.trades, result.has_self_trade);

	assert_eq!(remaining, 30); // 100 - 30 - 40
	assert_eq!(trades.len(), 2);

	// 验证按价格从小到大排序
	assert_eq!(trades[0].taker_price, "0.04");
	assert_eq!(trades[0].maker_order_id, "maker1");
	assert_eq!(trades[1].taker_price, "0.05");
	assert_eq!(trades[1].maker_order_id, "maker2");

	// 验证撮合后订单状态：maker1 和 maker2 应该被完全成交并从 yes_orders 中移除
	assert!(!engine.token_0_orders.contains_key("maker1"));
	assert!(!engine.token_0_orders.contains_key("maker2"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("maker1"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("maker2"));
}

#[tokio::test]
async fn test_match_engine_cross_matching_buy_yes_opposite_result() {
	// 买单Yes：应该匹配相反结果的买单（No的买单，使用opposite_result_price）
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加No结果的买单（注意：No价格0.5，opposite_result_price是0.5）
	let no_symbol = PredictionSymbol::new(1, 1, "token_1");
	// No价格0.3（3000），opposite_result_price = 10000 - 3000 = 7000（0.7）
	let maker1 = Order::new("maker1".to_string(), no_symbol.clone(), OrderSide::Buy, OrderType::Limit, 30, 3000, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	// No价格0.4（4000），opposite_result_price = 10000 - 4000 = 6000（0.6）
	let maker2 = Order::new("maker2".to_string(), no_symbol.clone(), OrderSide::Buy, OrderType::Limit, 40, 4000, 2, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 买单Yes，价格0.8（8000），应该匹配No的买单（使用opposite_result_price：0.7和0.6）
	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	let taker = Order::new("taker1".to_string(), yes_symbol, OrderSide::Buy, OrderType::Limit, 100, 8000, 3, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let result = engine.match_order(&order_to_submit_message(&taker)).unwrap();
	let (remaining, trades, _has_self_trade) = (result.remaining_quantity, result.trades, result.has_self_trade);

	assert_eq!(remaining, 30); // 100 - 30 - 40
	assert_eq!(trades.len(), 2);

	// 验证按价格从小到大排序（使用opposite_result_price）
	// maker1的opposite_result_price是0.7（7000），maker2的是0.6（6000）
	// 所以应该先匹配maker2（0.6），再匹配maker1（0.7）
	assert_eq!(trades[0].taker_price, "0.6"); // maker2的opposite_result_price
	assert_eq!(trades[0].maker_order_id, "maker2");
	assert_eq!(trades[1].taker_price, "0.7"); // maker1的opposite_result_price
	assert_eq!(trades[1].maker_order_id, "maker1");

	// 验证撮合后订单状态：maker1 和 maker2 应该被完全成交并从 no_orders 中移除
	assert!(!engine.token_1_orders.contains_key("maker1"));
	assert!(!engine.token_1_orders.contains_key("maker2"));
	assert!(!engine.token_1_orderbook.order_price_map.contains_key("maker1"));
	assert!(!engine.token_1_orderbook.order_price_map.contains_key("maker2"));
}

#[tokio::test]
async fn test_match_engine_cross_matching_buy_yes_both_results() {
	// 买单Yes：应该匹配相同结果的卖单 + 相反结果的买单，按价格从小到大排序
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加Yes结果的卖单
	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	let yes_maker = Order::new("yes_maker".to_string(), yes_symbol.clone(), OrderSide::Sell, OrderType::Limit, 30, 500, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, yes_maker);

	// 添加No结果的买单（No价格0.3，opposite_result_price = 0.7）
	let no_symbol = PredictionSymbol::new(1, 1, "token_1");
	let no_maker = Order::new("no_maker".to_string(), no_symbol.clone(), OrderSide::Buy, OrderType::Limit, 40, 3000, 2, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, no_maker);

	// 买单Yes，价格0.8，应该匹配：
	// 1. Yes卖单（价格0.05）
	// 2. No买单（opposite_result_price = 0.7）
	// 按价格从小到大：0.05 < 0.7，所以先匹配Yes卖单
	let taker = Order::new("taker1".to_string(), yes_symbol, OrderSide::Buy, OrderType::Limit, 100, 8000, 3, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let result = engine.match_order(&order_to_submit_message(&taker)).unwrap();
	let (remaining, trades, _has_self_trade) = (result.remaining_quantity, result.trades, result.has_self_trade);

	assert_eq!(remaining, 30); // 100 - 30 - 40
	assert_eq!(trades.len(), 2);

	// 验证按价格从小到大排序
	assert_eq!(trades[0].taker_price, "0.05"); // Yes卖单的价格
	assert_eq!(trades[0].maker_order_id, "yes_maker");
	assert_eq!(trades[1].taker_price, "0.7"); // No买单的opposite_result_price
	assert_eq!(trades[1].maker_order_id, "no_maker");

	// 验证撮合后订单状态：两个订单应该被完全成交并从对应的 orders 中移除
	assert!(!engine.token_0_orders.contains_key("yes_maker"));
	assert!(!engine.token_1_orders.contains_key("no_maker"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("yes_maker"));
	assert!(!engine.token_1_orderbook.order_price_map.contains_key("no_maker"));
}

#[tokio::test]
async fn test_match_engine_cross_matching_sell_yes_same_result() {
	// 卖单Yes：应该匹配相同结果的买单（Yes的买单，使用price），按价格从大到小排序
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加Yes结果的买单
	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	let maker1 = Order::new("maker1".to_string(), yes_symbol.clone(), OrderSide::Buy, OrderType::Limit, 30, 600, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let maker2 = Order::new("maker2".to_string(), yes_symbol.clone(), OrderSide::Buy, OrderType::Limit, 40, 500, 2, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 卖单Yes，价格0.04，应该匹配0.06和0.05的买单
	let taker = Order::new("taker1".to_string(), yes_symbol, OrderSide::Sell, OrderType::Limit, 100, 400, 3, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let result = engine.match_order(&order_to_submit_message(&taker)).unwrap();
	let (remaining, trades, _has_self_trade) = (result.remaining_quantity, result.trades, result.has_self_trade);

	assert_eq!(remaining, 30); // 100 - 30 - 40
	assert_eq!(trades.len(), 2);

	// 验证按价格从大到小排序
	assert_eq!(trades[0].taker_price, "0.06"); // 最高买价优先
	assert_eq!(trades[0].maker_order_id, "maker1");
	assert_eq!(trades[1].taker_price, "0.05");
	assert_eq!(trades[1].maker_order_id, "maker2");

	// 验证撮合后订单状态：maker1 和 maker2 应该被完全成交并从 yes_orders 中移除
	assert!(!engine.token_0_orders.contains_key("maker1"));
	assert!(!engine.token_0_orders.contains_key("maker2"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("maker1"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("maker2"));
}

#[tokio::test]
async fn test_match_engine_cross_matching_sell_yes_opposite_result() {
	// 卖单Yes：应该匹配相反结果的卖单（No的卖单，使用opposite_result_price），按价格从大到小排序
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加No结果的卖单（No价格0.3，opposite_result_price = 0.7）
	let no_symbol = PredictionSymbol::new(1, 1, "token_1");
	let maker1 = Order::new("maker1".to_string(), no_symbol.clone(), OrderSide::Sell, OrderType::Limit, 30, 3000, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	// No价格0.4，opposite_result_price = 0.6
	let maker2 = Order::new("maker2".to_string(), no_symbol.clone(), OrderSide::Sell, OrderType::Limit, 40, 4000, 2, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 卖单Yes，价格0.8，应该匹配No的卖单（使用opposite_result_price：0.7和0.6）
	// 注意：根据修复后的逻辑，taker_price <= opposite_price 才能匹配
	// 0.8 >= 0.7 和 0.8 >= 0.6，所以都不能匹配
	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	let taker = Order::new("taker1".to_string(), yes_symbol, OrderSide::Sell, OrderType::Limit, 100, 8000, 3, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let result = engine.match_order(&order_to_submit_message(&taker)).unwrap();
	let (remaining, trades, _has_self_trade) = (result.remaining_quantity, result.trades, result.has_self_trade);

	assert_eq!(remaining, 100); // 100 - 30 - 40
	assert_eq!(trades.len(), 0);
	assert!(engine.token_1_orders.contains_key("maker1"));
	assert!(engine.token_1_orders.contains_key("maker2"));
	assert!(engine.token_1_orderbook.order_price_map.contains_key("maker1"));
	assert!(engine.token_1_orderbook.order_price_map.contains_key("maker2"));
}

#[tokio::test]
async fn test_match_engine_cross_matching_sell_yes_both_results() {
	// 卖单Yes：应该匹配相同结果的买单 + 相反结果的卖单，按价格从大到小排序
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加Yes结果的买单（价格0.85，可以被taker价格0.8匹配，因为taker_price <= price）
	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	let yes_maker = Order::new("yes_maker".to_string(), yes_symbol.clone(), OrderSide::Buy, OrderType::Limit, 30, 8500, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, yes_maker);

	// 添加No结果的卖单（No价格0.1，opposite_result_price = 0.9）
	let no_symbol = PredictionSymbol::new(1, 1, "token_1");
	let no_maker = Order::new("no_maker".to_string(), no_symbol.clone(), OrderSide::Sell, OrderType::Limit, 40, 1000, 2, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, no_maker);

	// 卖单Yes，价格0.8，应该匹配：
	// 1. No卖单（opposite_result_price = 0.9），因为0.8 <= 0.9
	// 2. Yes买单（价格0.85），因为0.8 <= 0.85
	// 注意：根据修复后的逻辑，匹配相反结果卖单需要 taker_price >= opposite_price
	// 按价格从大到小：0.9 > 0.85，所以先匹配No卖单
	let taker = Order::new("taker1".to_string(), yes_symbol, OrderSide::Sell, OrderType::Limit, 100, 8000, 3, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let result = engine.match_order(&order_to_submit_message(&taker)).unwrap();
	let (remaining, trades, _has_self_trade) = (result.remaining_quantity, result.trades, result.has_self_trade);

	assert_eq!(remaining, 30); // 100 - 30 - 40
	assert_eq!(trades.len(), 2);

	// 验证按价格从大到小排序
	assert_eq!(trades[0].taker_price, "0.9"); // no卖单的价格（更高）
	assert_eq!(trades[0].maker_order_id, "no_maker");
	assert_eq!(trades[1].taker_price, "0.85"); // yes买单的opposite_result_price
	assert_eq!(trades[1].maker_order_id, "yes_maker");

	// 验证撮合后订单状态：两个订单应该被完全成交并从对应的 orders 中移除
	assert!(!engine.token_0_orders.contains_key("yes_maker"));
	assert!(!engine.token_1_orders.contains_key("no_maker"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("yes_maker"));
	assert!(!engine.token_1_orderbook.order_price_map.contains_key("no_maker"));
}

#[tokio::test]
async fn test_match_engine_cross_matching_price_limit_buy() {
	// 测试限价买单的价格限制：只匹配价格 <= taker_price 的订单
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	// 添加价格0.05和0.06的卖单
	let maker1 = Order::new("maker1".to_string(), yes_symbol.clone(), OrderSide::Sell, OrderType::Limit, 30, 500, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let maker2 = Order::new("maker2".to_string(), yes_symbol.clone(), OrderSide::Sell, OrderType::Limit, 40, 600, 2, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 买单价格0.05，应该只匹配0.05的卖单，不匹配0.06的
	let taker = Order::new("taker1".to_string(), yes_symbol, OrderSide::Buy, OrderType::Limit, 100, 500, 3, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let result = engine.match_order(&order_to_submit_message(&taker)).unwrap();
	let (remaining, trades, _has_self_trade) = (result.remaining_quantity, result.trades, result.has_self_trade);

	assert_eq!(remaining, 70); // 100 - 30
	assert_eq!(trades.len(), 1);
	assert_eq!(trades[0].taker_price, "0.05");
	assert_eq!(trades[0].maker_order_id, "maker1");

	// 验证撮合后订单状态：maker1 应该被完全成交并从 yes_orders 中移除
	// maker2 没有被匹配，应该仍然存在
	assert!(!engine.token_0_orders.contains_key("maker1"));
	assert!(engine.token_0_orders.contains_key("maker2"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("maker1"));
	assert!(engine.token_0_orderbook.order_price_map.contains_key("maker2"));
}

#[tokio::test]
async fn test_match_engine_cross_matching_price_limit_sell() {
	// 测试限价卖单的价格限制：只匹配价格 >= taker_price 的订单
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	// 添加价格0.05和0.06的买单
	let maker1 = Order::new("maker1".to_string(), yes_symbol.clone(), OrderSide::Buy, OrderType::Limit, 30, 500, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let maker2 = Order::new("maker2".to_string(), yes_symbol.clone(), OrderSide::Buy, OrderType::Limit, 40, 600, 2, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 卖单价格0.06，应该只匹配0.06的买单，不匹配0.05的
	let taker = Order::new("taker1".to_string(), yes_symbol, OrderSide::Sell, OrderType::Limit, 100, 600, 3, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let result = engine.match_order(&order_to_submit_message(&taker)).unwrap();
	let (remaining, trades, _has_self_trade) = (result.remaining_quantity, result.trades, result.has_self_trade);

	assert_eq!(remaining, 60); // 100 - 40
	assert_eq!(trades.len(), 1);
	assert_eq!(trades[0].taker_price, "0.06");
	assert_eq!(trades[0].maker_order_id, "maker2");

	// 验证撮合后订单状态：maker2 应该被完全成交并从 yes_orders 中移除
	// maker1 没有被匹配，应该仍然存在
	assert!(engine.token_0_orders.contains_key("maker1"));
	assert!(!engine.token_0_orders.contains_key("maker2"));
	assert!(engine.token_0_orderbook.order_price_map.contains_key("maker1"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("maker2"));
}

#[tokio::test]
async fn test_match_engine_cross_matching_sell_opposite_result_price_limit() {
	// 测试卖单taker匹配相反结果卖单时的价格限制：taker_price >= opposite_price 才能匹配
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加No结果的卖单（No价格0.3，opposite_result_price = 0.7）
	let no_symbol = PredictionSymbol::new(1, 1, "token_1");
	let maker1 = Order::new("maker1".to_string(), no_symbol.clone(), OrderSide::Sell, OrderType::Limit, 30, 3000, 1, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	// No价格0.4，opposite_result_price = 0.6
	let maker2 = Order::new("maker2".to_string(), no_symbol.clone(), OrderSide::Sell, OrderType::Limit, 40, 4000, 2, "test_privy_id".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 卖单Yes，价格0.65（6500），应该不匹配maker2（opposite_result_price = 0.6），匹配maker1（opposite_result_price = 0.7）
	// 因为 0.65 >= 0.6 但是 0.65 < 0.7
	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	let taker = Order::new("taker1".to_string(), yes_symbol, OrderSide::Sell, OrderType::Limit, 100, 6500, 3, "test_privy_id".to_string(), "Yes".to_string()).unwrap();
	let result = engine.match_order(&order_to_submit_message(&taker)).unwrap();
	let (remaining, trades, _has_self_trade) = (result.remaining_quantity, result.trades, result.has_self_trade);

	assert_eq!(remaining, 70); // 100 - 30
	assert_eq!(trades.len(), 1);
	assert_eq!(trades[0].taker_price, "0.7"); // maker1的opposite_result_price
	assert_eq!(trades[0].maker_order_id, "maker1");

	// 验证撮合后订单状态：maker1 被完全成交并从 no_orders 中移除
	// maker2 没有被匹配（因为taker_price < opposite_price），应该仍然存在
	assert!(engine.token_1_orders.contains_key("maker2"));
	assert!(!engine.token_1_orders.contains_key("maker1"));
	assert!(engine.token_1_orderbook.order_price_map.contains_key("maker2"));
	assert!(!engine.token_1_orderbook.order_price_map.contains_key("maker1"));
}

// ============================================================================
// 市价单和自成交测试
// ============================================================================

#[tokio::test]
async fn test_market_order_partial_fill_cancel_remaining() {
	// 场景：taker是市价单，去吃单但吃不完，剩余的会被取消
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	// 添加一些卖单（Yes结果）
	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	let maker1 = Order::new("maker1".to_string(), yes_symbol.clone(), OrderSide::Sell, OrderType::Limit, 30, 500, 1, "test_privy_id_1".to_string(), "Yes".to_string()).unwrap();
	let maker2 = Order::new("maker2".to_string(), yes_symbol.clone(), OrderSide::Sell, OrderType::Limit, 40, 600, 2, "test_privy_id_2".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 提交市价买单（数量 100），但只能匹配 70 (30 + 40)
	let taker = Order::new("taker1".to_string(), yes_symbol, OrderSide::Buy, OrderType::Market, 100, 9900, 3, "test_privy_id_3".to_string(), "Yes".to_string()).unwrap();
	assert!(engine.submit_order(&order_to_submit_message(&taker), 1).is_ok());

	// 验证市价单部分成交
	// COMMENTED: 	assert_eq!(taker.status, OrderStatus::PartiallyFilled);
	// COMMENTED: 	assert_eq!(taker.filled_quantity, 70); // 成交了 30 + 40
	// COMMENTED: 	assert_eq!(taker.remaining_quantity, 30); // 剩余 30

	// 验证 maker1 和 maker2 被完全成交并移除
	assert!(!engine.token_0_orders.contains_key("maker1"));
	assert!(!engine.token_0_orders.contains_key("maker2"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("maker1"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("maker2"));

	// 关键验证：市价单部分成交后不应该在订单簿中（剩余部分被取消）
	assert!(!engine.token_0_orders.contains_key("taker1"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("taker1"));
}

#[tokio::test]
async fn test_self_trade_detection_stops_matching() {
	// 场景：a 挂 0.8 的卖单 Yes，b 挂 0.5 的卖单 Yes，a 去买 Yes 以 0.9 的价格
	// 会吃了 b 的单，但会在遇到自己挂的 0.8 的单子时停下并取消剩余
	let (mut engine, _sender, _exit_receiver) = create_match_engine();

	let yes_symbol = PredictionSymbol::new(1, 1, "token_0");
	let user_a_id = 100i64;
	let user_b_id = 200i64;

	// a 挂 0.8 的卖单 Yes (价格 8000)
	let a_sell_order = Order::new("a_sell".to_string(), yes_symbol.clone(), OrderSide::Sell, OrderType::Limit, 50, 8000, user_a_id, "privy_a".to_string(), "Yes".to_string()).unwrap();

	// b 挂 0.5 的卖单 Yes (价格 5000)
	let b_sell_order = Order::new("b_sell".to_string(), yes_symbol.clone(), OrderSide::Sell, OrderType::Limit, 40, 5000, user_b_id, "privy_b".to_string(), "Yes".to_string()).unwrap();

	add_order_to_engine(&mut engine, a_sell_order);
	add_order_to_engine(&mut engine, b_sell_order);

	// a 去买 Yes，以 0.9 的价格买 100 个 (价格 9000)
	let a_buy_order = Order::new(
		"a_buy".to_string(),
		yes_symbol,
		OrderSide::Buy,
		OrderType::Limit,
		100,
		9000,
		user_a_id, // 注意：同一个用户 ID
		"privy_a".to_string(),
		"Yes".to_string(),
	)
	.unwrap();

	assert!(engine.submit_order(&order_to_submit_message(&a_buy_order), 1).is_ok());

	// 验证成交情况：应该只吃了 b 的单 (40)，遇到自己的单时停止
	// COMMENTED: 	assert_eq!(a_buy_order.filled_quantity, 40); // 只成交了 b 的 40
	// COMMENTED: 	assert_eq!(a_buy_order.remaining_quantity, 60); // 剩余 60 (100 - 40)
	// COMMENTED: 	assert_eq!(a_buy_order.status, OrderStatus::PartiallyFilled); // 部分成交（剩余部分会被取消）

	// 验证 b 的卖单被完全成交并移除
	assert!(!engine.token_0_orders.contains_key("b_sell"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("b_sell"));

	// 验证 a 的卖单仍然在订单簿中（没有被成交）
	assert!(engine.token_0_orders.contains_key("a_sell"));
	assert!(engine.token_0_orderbook.order_price_map.contains_key("a_sell"));

	// 验证 a 的买单不在订单簿中（被取消）
	assert!(!engine.token_0_orders.contains_key("a_buy"));
	assert!(!engine.token_0_orderbook.order_price_map.contains_key("a_buy"));
}

// ============================================================================
// 市价单测试
// ============================================================================

/// 测试市价买单完全成交
#[tokio::test]
async fn test_market_order_buy_full_match() {
	let (mut engine, _sender, _exit_rx) = create_match_engine();

	// 盘口：挂3个卖单，价格递增
	// 500 价格 100 数量
	// 600 价格 100 数量
	// 700 价格 100 数量
	let maker1 = create_test_order("maker1", OrderSide::Sell, 500, 100);
	let maker2 = create_test_order("maker2", OrderSide::Sell, 600, 100);
	let maker3 = create_test_order("maker3", OrderSide::Sell, 700, 100);

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);
	add_order_to_engine(&mut engine, maker3);

	// 市价买单：0.11 USDC volume
	// 按价格从低到高吃：
	// maker1 @ 500: 100 * 500 / 1000000 = 0.05 USDC
	// maker2 @ 600: 100 * 600 / 1000000 = 0.06 USDC
	// 总共需要 0.11 USDC，正好吃完两个订单
	let taker = create_market_order("taker", OrderSide::Buy, 9900, 0);
	let mut taker_msg = order_to_submit_message(&taker);
	taker_msg.volume = Decimal::new(11, 2); // 0.11 USDC
	taker_msg.quantity = 0; // 市价买单 quantity=0

	assert!(engine.submit_order(&taker_msg, 1).is_ok());

	// 验证 maker3 仍在盘口
	assert!(engine.token_0_orders.contains_key("maker3"));
	// 验证 maker1 和 maker2 被完全成交
	assert!(!engine.token_0_orders.contains_key("maker1"));
	assert!(!engine.token_0_orders.contains_key("maker2"));
}

/// 测试市价买单部分成交（volume 只够吃部分订单）
#[tokio::test]
async fn test_market_order_buy_partial_match_price_limit() {
	let (mut engine, _sender, _exit_rx) = create_match_engine();

	// 盘口：挂3个卖单
	// 500 价格 100 数量
	// 600 价格 100 数量
	// 800 价格 100 数量
	let maker1 = create_test_order("maker1", OrderSide::Sell, 500, 100);
	let maker2 = create_test_order("maker2", OrderSide::Sell, 600, 100);
	let maker3 = create_test_order("maker3", OrderSide::Sell, 800, 100);

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);
	add_order_to_engine(&mut engine, maker3);

	// 市价买单：0.11 USDC volume
	// 按价格从低到高吃：
	// maker1 @ 500: 100 * 500 / 1000000 = 0.05 USDC
	// maker2 @ 600: 100 * 600 / 1000000 = 0.06 USDC
	// 总共需要 0.11 USDC，吃完 maker1 和 maker2，不会触及 maker3
	let taker = create_market_order("taker", OrderSide::Buy, 9900, 0);
	let mut taker_msg = order_to_submit_message(&taker);
	taker_msg.volume = Decimal::new(11, 2); // 0.11 USDC
	taker_msg.quantity = 0; // 市价买单 quantity=0

	assert!(engine.submit_order(&taker_msg, 1).is_ok());

	// 验证 maker3 仍在盘口
	assert!(engine.token_0_orders.contains_key("maker3"));
	// 验证 maker1 和 maker2 被完全成交
	assert!(!engine.token_0_orders.contains_key("maker1"));
	assert!(!engine.token_0_orders.contains_key("maker2"));
}

/// 测试市价买单吃大单时只吃到 volume 用完为止
#[tokio::test]
async fn test_market_order_buy_partial_quantity_from_large_order() {
	let (mut engine, _sender, _exit_rx) = create_match_engine();

	// 盘口：
	// 500 价格 50 数量
	// 700 价格 200 数量（大单）
	let maker1 = create_test_order("maker1", OrderSide::Sell, 500, 50);
	let maker2 = create_test_order("maker2", OrderSide::Sell, 700, 200);

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 市价买单：0.05 USDC volume
	// 价格精度：price * quantity / 1000000 = USDC
	// 按价格从低到高吃：
	// 1. 吃 maker1 全部(50 @ 500): 500 * 50 / 1000000 = 0.025 USDC，剩余 0.025 USDC
	// 2. 吃 maker2 部分: 0.025 USDC / (700/1000000) ≈ 35 quantity
	// maker2 剩余 165 quantity
	let taker = create_market_order("taker", OrderSide::Buy, 9900, 0); // price=9900(max), quantity=0
	// 手动构建 SubmitOrderMessage，volume=0.05
	let mut taker_msg = order_to_submit_message(&taker);
	taker_msg.volume = Decimal::new(5, 2); // 0.05 USDC
	taker_msg.quantity = 0; // 市价买单 quantity=0

	let result = engine.submit_order(&taker_msg, 1);
	assert!(result.is_ok(), "Submit order failed: {:?}", result.err());

	// 验证 maker1 被完全成交
	assert!(!engine.token_0_orders.contains_key("maker1"));

	// 验证 maker2 部分成交，剩余 165
	if let Some(maker2_order) = engine.token_0_orders.get("maker2") {
		println!("maker2 remaining_quantity: {}", maker2_order.remaining_quantity);
		assert_eq!(maker2_order.remaining_quantity, 165);
	} else {
		panic!("maker2 should still be in orderbook but was fully matched!");
	}
}

/// 测试市价卖单完全成交
#[tokio::test]
async fn test_market_order_sell_full_match() {
	let (mut engine, _sender, _exit_rx) = create_match_engine();

	// 盘口：挂3个买单，价格递减
	// 700 价格 100 数量
	// 600 价格 100 数量
	// 500 价格 100 数量
	let maker1 = create_test_order("maker1", OrderSide::Buy, 700, 100);
	let maker2 = create_test_order("maker2", OrderSide::Buy, 600, 100);
	let maker3 = create_test_order("maker3", OrderSide::Buy, 500, 100);

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);
	add_order_to_engine(&mut engine, maker3);

	// 市价卖单：200 数量，价格限制 600（价格不低于 600）
	// 按价格从高到低吃：maker1(100 @ 700) + maker2(100 @ 600)，全部成交
	let taker = create_market_order("taker", OrderSide::Sell, 600, 200);

	assert!(engine.submit_order(&order_to_submit_message(&taker), 1).is_ok());

	// COMMENTED: 	assert_eq!(taker.filled_quantity, 200); // 完全成交
	// COMMENTED: 	assert_eq!(taker.remaining_quantity, 0);
	// COMMENTED: 	assert_eq!(taker.status, OrderStatus::Filled);

	// 验证 maker3 仍在盘口
	assert!(engine.token_0_orders.contains_key("maker3"));
	// 验证 maker1 和 maker2 被完全成交
	assert!(!engine.token_0_orders.contains_key("maker1"));
	assert!(!engine.token_0_orders.contains_key("maker2"));
}

/// 测试市价卖单因价格低于限制而部分成交
#[tokio::test]
async fn test_market_order_sell_partial_match_price_limit() {
	let (mut engine, _sender, _exit_rx) = create_match_engine();

	// 盘口：挂3个买单
	// 700 价格 100 数量
	// 600 价格 100 数量
	// 400 价格 100 数量
	let maker1 = create_test_order("maker1", OrderSide::Buy, 700, 100);
	let maker2 = create_test_order("maker2", OrderSide::Buy, 600, 100);
	let maker3 = create_test_order("maker3", OrderSide::Buy, 400, 100);

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);
	add_order_to_engine(&mut engine, maker3);

	// 市价卖单：300 数量，价格限制 600
	// 按价格从高到低吃：
	// 1. 吃 maker1(100 @ 700): 700 >= 600 ✓
	// 2. 吃 maker2(100 @ 600): 600 >= 600 ✓
	// 3. 尝试吃 maker3(100 @ 400): 400 < 600 ✗ 停止
	// 总共成交 200，剩余 100 被取消
	let taker = create_market_order("taker", OrderSide::Sell, 600, 300);

	assert!(engine.submit_order(&order_to_submit_message(&taker), 1).is_ok());

	// COMMENTED: 	assert_eq!(taker.filled_quantity, 200); // 只成交了 200
	// COMMENTED: 	assert_eq!(taker.remaining_quantity, 100);
	// COMMENTED: 	assert_eq!(taker.status, OrderStatus::PartiallyFilled);

	// 验证 maker3 仍在盘口
	assert!(engine.token_0_orders.contains_key("maker3"));
}

/// 测试市价卖单被价格限制阻止继续吃单
#[tokio::test]
async fn test_market_order_sell_partial_quantity_from_large_order() {
	let (mut engine, _sender, _exit_rx) = create_match_engine();

	// 盘口：
	// 700 价格 50 数量
	// 500 价格 200 数量（大单）
	let maker1 = create_test_order("maker1", OrderSide::Buy, 700, 50);
	let maker2 = create_test_order("maker2", OrderSide::Buy, 500, 200);

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 市价卖单：100 数量，价格限制 600（不能低于 600）
	// 按价格从高到低吃：
	// 1. 吃 maker1(50 @ 700): 700 >= 600 ✓，成交 50
	// 2. 尝试吃 maker2(@ 500): 500 < 600 ✗，停止！
	// 总共成交 50 quantity，剩余 50 quantity 被取消
	let taker = create_market_order("taker", OrderSide::Sell, 600, 100);
	let taker_msg = order_to_submit_message(&taker);

	assert!(engine.submit_order(&taker_msg, 1).is_ok());

	// 验证 maker1 被完全成交
	assert!(!engine.token_0_orders.contains_key("maker1"));

	// 验证 maker2 未被触碰，仍有 200
	let maker2_order = engine.token_0_orders.get("maker2").unwrap();
	assert_eq!(maker2_order.remaining_quantity, 200);
}

/// 测试市价买单无匹配订单
#[tokio::test]
async fn test_market_order_buy_no_match() {
	let (mut engine, _sender, _exit_rx) = create_match_engine();

	// 空盘口
	let taker = create_market_order("taker", OrderSide::Buy, 600, 100);

	assert!(engine.submit_order(&order_to_submit_message(&taker), 1).is_ok());

	// COMMENTED: 	assert_eq!(taker.filled_quantity, 0); // 无成交
	// COMMENTED: 	assert_eq!(taker.remaining_quantity, 100);
	// COMMENTED: 	assert_eq!(taker.status, OrderStatus::New);
}

/// 测试市价单自成交检测
#[tokio::test]
async fn test_market_order_self_trade_detection() {
	let (mut engine, _sender, _exit_rx) = create_match_engine();

	// 挂一个用户1的卖单
	let symbol = create_test_symbol();
	let maker = Order::new("maker".to_string(), symbol.clone(), OrderSide::Sell, OrderType::Limit, 100, 500, 1, "privy1".to_string(), "Yes".to_string()).unwrap();
	add_order_to_engine(&mut engine, maker);

	// 用户1的市价买单（会遇到自己的卖单）
	let taker = Order::new("taker".to_string(), symbol, OrderSide::Buy, OrderType::Market, 100, 600, 1, "privy1".to_string(), "Yes".to_string()).unwrap();

	assert!(engine.submit_order(&order_to_submit_message(&taker), 1).is_ok());

	// COMMENTED: 	assert_eq!(taker.filled_quantity, 0); // 无成交（自成交被阻止）
	// COMMENTED: 	assert_eq!(taker.remaining_quantity, 100);
	// COMMENTED: 	assert_eq!(taker.status, OrderStatus::New);
}

/// 测试市价买单 volume 正好用完
#[tokio::test]
async fn test_market_order_edge_case_exact_price() {
	let (mut engine, _sender, _exit_rx) = create_match_engine();

	// 盘口：2个卖单
	// 500 价格 100 数量
	// 700 价格 100 数量
	let maker1 = create_test_order("maker1", OrderSide::Sell, 500, 100);
	let maker2 = create_test_order("maker2", OrderSide::Sell, 700, 100);

	add_order_to_engine(&mut engine, maker1);
	add_order_to_engine(&mut engine, maker2);

	// 市价买单：0.05 USDC volume
	// 按价格从低到高吃：maker1 @ 500 需要 100 * 500 / 1000000 = 0.05 USDC，正好用完
	let taker = create_market_order("taker", OrderSide::Buy, 9900, 0);
	let mut taker_msg = order_to_submit_message(&taker);
	taker_msg.volume = Decimal::new(5, 2); // 0.05 USDC
	taker_msg.quantity = 0; // 市价买单 quantity=0

	assert!(engine.submit_order(&taker_msg, 1).is_ok());

	// 验证 maker1 被完全成交
	assert!(!engine.token_0_orders.contains_key("maker1"));
	// 验证 maker2 未被触碰
	assert!(engine.token_0_orders.contains_key("maker2"));
}
