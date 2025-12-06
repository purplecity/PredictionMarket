use {
	crate::{
		consts::CANCEL_ORDERS_BATCH_SIZE,
		helper::{format_price, format_quantity, get_usdc_amount, price_decimal, quantity_decimal},
		orderbook::OrderBook,
		output::{publish_order_change_to_store, publish_to_processor},
		types::{OrderBookControl, OrderBookDepth},
	},
	chrono::Utc,
	common::{
		engine_types::{Order, OrderSide, OrderStatus, OrderType, PredictionSymbol},
		processor_types::{OrderCancelled, OrderRejected, OrderSubmitted, OrderTraded, ProcessorMessage, Trade},
		store_types::OrderChangeEvent,
		websocket_types::{PriceLevelInfo, SingleTokenPriceInfo},
	},
	rust_decimal::Decimal,
	rustc_hash::FxHashMap,
	std::{collections::HashMap, sync::Arc},
	tokio::{
		select,
		sync::{OnceCell, RwLock, broadcast, mpsc},
		time::{Duration, interval},
	},
	tracing::{error, info},
};

// 辅助函数：checked 算术运算
fn checked_add(a: Decimal, b: Decimal, msg: &str) -> anyhow::Result<Decimal> {
	a.checked_add(b).ok_or_else(|| anyhow::anyhow!("Decimal addition overflow: {} ({} + {})", msg, a, b))
}

fn checked_sub(a: Decimal, b: Decimal, msg: &str) -> anyhow::Result<Decimal> {
	a.checked_sub(b).ok_or_else(|| anyhow::anyhow!("Decimal subtraction overflow: {} ({} - {})", msg, a, b))
}

fn checked_mul(a: Decimal, b: Decimal, msg: &str) -> anyhow::Result<Decimal> {
	a.checked_mul(b).ok_or_else(|| anyhow::anyhow!("Decimal multiplication overflow: {} ({} * {})", msg, a, b))
}

static G_MANAGER: OnceCell<RwLock<Manager>> = OnceCell::const_new();

static G_SHUTDOWN: OnceCell<broadcast::Sender<()>> = OnceCell::const_new();

/// 初始化 manager
pub fn init_manager() {
	let _ = G_MANAGER.set(RwLock::new(Manager::new()));
}

/// 获取 manager
pub fn get_manager() -> &'static RwLock<Manager> {
	G_MANAGER.get().expect("G_MANAGER not initialized")
}

/// 初始化 shutdown sender
pub fn init_shutdown() {
	let (tx, _) = broadcast::channel(1);
	let _ = G_SHUTDOWN.set(tx);
}

/// 获取 shutdown sender
pub fn get_shutdown() -> &'static broadcast::Sender<()> {
	G_SHUTDOWN.get().expect("G_SHUTDOWN not initialized")
}

/// 获取 shutdown receiver
pub fn get_shutdown_receiver() -> broadcast::Receiver<()> {
	get_shutdown().subscribe()
}

pub struct Manager {
	pub event_managers: Arc<RwLock<HashMap<i64, EventManager>>>,
	pub stop: bool, // 用于维护撮合引擎是否暂停
}

impl Default for Manager {
	fn default() -> Self {
		Self::new()
	}
}

impl Manager {
	pub fn new() -> Self {
		Self { event_managers: Arc::new(RwLock::new(HashMap::new())), stop: false }
	}
}

pub struct EventManager {
	pub event_id: i64,
	pub order_senders: HashMap<String, mpsc::Sender<OrderBookControl>>, // key是event_id和market_id的组合，格式: event_id分隔符market_id
	pub exit_signal: broadcast::Sender<()>,
	pub end_timestamp: Option<chrono::DateTime<Utc>>, // 市场结束时间戳
}

pub struct MatchEngine {
	pub event_id: i64,
	pub market_id: i16,
	pub token_0_id: String,
	pub token_1_id: String,
	pub token_0_orderbook: OrderBook,
	pub token_1_orderbook: OrderBook,
	pub token_0_orders: HashMap<String, Arc<Order>>, //取消订单的时候需要用到，使用Arc共享订单数据
	pub token_1_orders: HashMap<String, Arc<Order>>, //取消订单的时候需要用到，使用Arc共享订单数据
	pub order_receiver: mpsc::Receiver<OrderBookControl>,
	pub exit_receiver: broadcast::Receiver<()>,
	pub order_num: u64,
	pub update_id: u64,
	// 缓存价格档位变化 price -> PriceLevelChange (分开bid和ask)
	pub token_0_bid_changes: FxHashMap<i32, u64>,
	pub token_0_ask_changes: FxHashMap<i32, u64>,
	pub token_1_bid_changes: FxHashMap<i32, u64>,
	pub token_1_ask_changes: FxHashMap<i32, u64>,
	// 上一秒的深度快照，用于对比价格档位变化
	pub last_token_0_depth_snapshot: Option<OrderBookDepth>,
	pub last_token_1_depth_snapshot: Option<OrderBookDepth>,
	// 最新成交价格 (字符串格式，已除以10000)
	pub token_0_latest_trade_price: String,
	pub token_1_latest_trade_price: String,
}

impl MatchEngine {
	//新创建一个撮合引擎 并返回order_sender
	// 现在一个MatchEngine处理一个market（event_id + market_id），维护token_0和token_1两个OrderBook
	pub fn new(event_id: i64, market_id: i16, token_ids: (String, String), max_order_count: u64, exit_receiver: broadcast::Receiver<()>) -> (Self, mpsc::Sender<OrderBookControl>) {
		let (order_sender, order_receiver) = mpsc::channel(max_order_count as usize);
		let token_0_symbol = PredictionSymbol::new(event_id, market_id, &token_ids.0);
		let token_1_symbol = PredictionSymbol::new(event_id, market_id, &token_ids.1);
		let engine = Self {
			event_id,
			market_id,
			token_0_id: token_ids.0.clone(),
			token_1_id: token_ids.1.clone(),
			token_0_orderbook: OrderBook::new(token_0_symbol),
			token_1_orderbook: OrderBook::new(token_1_symbol),
			token_0_orders: HashMap::new(),
			token_1_orders: HashMap::new(),
			order_receiver,
			exit_receiver,
			order_num: 0,
			update_id: 0,
			token_0_bid_changes: FxHashMap::default(),
			token_0_ask_changes: FxHashMap::default(),
			token_1_bid_changes: FxHashMap::default(),
			token_1_ask_changes: FxHashMap::default(),
			last_token_0_depth_snapshot: None,
			last_token_1_depth_snapshot: None,
			token_0_latest_trade_price: String::new(),
			token_1_latest_trade_price: String::new(),
		};
		(engine, order_sender)
	}

	//从订单列表创建撮合引擎 并返回order_sender
	// symbol_orders_map: key是symbol字符串，value是对应的订单列表
	pub fn new_with_orders(
		event_id: i64,
		market_id: i16,
		token_ids: (String, String),
		symbol_orders_map: HashMap<String, Vec<Order>>,
		max_order_count: u64,
		exit_receiver: broadcast::Receiver<()>,
	) -> anyhow::Result<(Self, mpsc::Sender<OrderBookControl>)> {
		let (order_sender, order_receiver) = mpsc::channel(max_order_count as usize);

		let token_0_symbol = PredictionSymbol::new(event_id, market_id, &token_ids.0);
		let token_1_symbol = PredictionSymbol::new(event_id, market_id, &token_ids.1);
		let token_0_symbol_key = token_0_symbol.to_string();
		let token_1_symbol_key = token_1_symbol.to_string();

		let token_0_orders_vec = symbol_orders_map.get(&token_0_symbol_key).cloned().unwrap_or_default();
		let token_1_orders_vec = symbol_orders_map.get(&token_1_symbol_key).cloned().unwrap_or_default();

		let max_order_num = token_0_orders_vec.iter().chain(token_1_orders_vec.iter()).map(|o| o.order_num).max().unwrap_or(0);

		// 将订单转换为Arc并保存到HashMap
		let token_0_arc_orders: Vec<Arc<Order>> = token_0_orders_vec.into_iter().map(Arc::new).collect();
		let token_1_arc_orders: Vec<Arc<Order>> = token_1_orders_vec.into_iter().map(Arc::new).collect();

		// 构建orderbook：先用build构建本方订单，再交叉插入对方订单
		let mut token_0_orderbook = OrderBook::build(token_0_symbol, token_0_arc_orders.to_vec())?;
		let mut token_1_orderbook = OrderBook::build(token_1_symbol, token_1_arc_orders.to_vec())?;

		// 交叉插入：token_0订单插入到token_1_orderbook，token_1订单插入到token_0_orderbook
		for order in &token_0_arc_orders {
			token_1_orderbook.add_cross_order(Arc::clone(order))?;
		}
		for order in &token_1_arc_orders {
			token_0_orderbook.add_cross_order(Arc::clone(order))?;
		}

		let token_0_orders_map: HashMap<String, Arc<Order>> = token_0_arc_orders.into_iter().map(|o| (o.order_id.clone(), o)).collect();
		let token_1_orders_map: HashMap<String, Arc<Order>> = token_1_arc_orders.into_iter().map(|o| (o.order_id.clone(), o)).collect();

		let engine = Self {
			event_id,
			market_id,
			token_0_id: token_ids.0.clone(),
			token_1_id: token_ids.1.clone(),
			token_0_orderbook,
			token_1_orderbook,
			token_0_orders: token_0_orders_map,
			token_1_orders: token_1_orders_map,
			order_receiver,
			exit_receiver,
			order_num: max_order_num + 1,
			update_id: 0,
			token_0_bid_changes: FxHashMap::default(),
			token_0_ask_changes: FxHashMap::default(),
			token_1_bid_changes: FxHashMap::default(),
			token_1_ask_changes: FxHashMap::default(),
			last_token_0_depth_snapshot: None,
			last_token_1_depth_snapshot: None,
			token_0_latest_trade_price: String::new(),
			token_1_latest_trade_price: String::new(),
		};
		Ok((engine, order_sender))
	}

	// 对比上一秒的深度快照，更新价格档位变化缓存
	pub fn update_price_level_changes(&mut self, current_depth: &OrderBookDepth) {
		let is_token_0 = current_depth.symbol.token_id == self.token_0_id;

		// 根据token_id选择对应的bid/ask changes和last_depth_snapshot
		let (bid_changes, ask_changes, last_depth_snapshot) = if is_token_0 {
			(&mut self.token_0_bid_changes, &mut self.token_0_ask_changes, &mut self.last_token_0_depth_snapshot)
		} else {
			(&mut self.token_1_bid_changes, &mut self.token_1_ask_changes, &mut self.last_token_1_depth_snapshot)
		};

		// 构建当前深度的价格档位映射 price_i32 -> total_quantity_u64
		let mut current_bids: FxHashMap<i32, u64> = FxHashMap::default();
		let mut current_asks: FxHashMap<i32, u64> = FxHashMap::default();
		for price_level in &current_depth.bids {
			current_bids.insert(price_level.price_i32, price_level.total_quantity_u64);
		}
		for price_level in &current_depth.asks {
			current_asks.insert(price_level.price_i32, price_level.total_quantity_u64);
		}

		// 如果有上一秒的快照，对比变化
		if let Some(last_depth) = last_depth_snapshot.as_ref() {
			// 构建上一秒深度的价格档位映射
			let mut last_bids: FxHashMap<i32, u64> = FxHashMap::default();
			let mut last_asks: FxHashMap<i32, u64> = FxHashMap::default();
			for price_level in &last_depth.bids {
				last_bids.insert(price_level.price_i32, price_level.total_quantity_u64);
			}
			for price_level in &last_depth.asks {
				last_asks.insert(price_level.price_i32, price_level.total_quantity_u64);
			}

			// 检查bid变化
			for (price_i32, current_quantity) in &current_bids {
				let last_quantity = last_bids.get(price_i32).copied().unwrap_or(0);
				if *current_quantity != last_quantity {
					bid_changes.insert(*price_i32, *current_quantity);
				}
			}
			// 检查消失的bid
			for (price_i32, last_quantity) in &last_bids {
				if !current_bids.contains_key(price_i32) && *last_quantity > 0 {
					bid_changes.insert(*price_i32, 0);
				}
			}

			// 检查ask变化
			for (price_i32, current_quantity) in &current_asks {
				let last_quantity = last_asks.get(price_i32).copied().unwrap_or(0);
				if *current_quantity != last_quantity {
					ask_changes.insert(*price_i32, *current_quantity);
				}
			}
			// 检查消失的ask
			for (price_i32, last_quantity) in &last_asks {
				if !current_asks.contains_key(price_i32) && *last_quantity > 0 {
					ask_changes.insert(*price_i32, 0);
				}
			}
		} else {
			// 没有上一秒的快照，记录所有当前价格档位
			for (price_i32, quantity) in current_bids {
				bid_changes.insert(price_i32, quantity);
			}
			for (price_i32, quantity) in current_asks {
				ask_changes.insert(price_i32, quantity);
			}
		}

		// 保存当前深度快照用于下次对比
		*last_depth_snapshot = Some(current_depth.clone());
	}

	// 每秒快照处理
	pub fn handle_snapshot_tick(&mut self) {
		// 先清空上一次的价格档位变化缓存
		self.token_0_bid_changes.clear();
		self.token_0_ask_changes.clear();
		self.token_1_bid_changes.clear();
		self.token_1_ask_changes.clear();

		// update_id 加1
		self.update_id += 1;
		let timestamp = Utc::now().timestamp_millis();

		// 推送 update_id 到 store，用于快照保存
		if let Err(e) = publish_order_change_to_store(OrderChangeEvent::MarketUpdateId { event_id: self.event_id, market_id: self.market_id, update_id: self.update_id }) {
			error!("Failed to publish update_id to store: {}", e);
		}

		// 为token_0和token_1两个OrderBook分别生成深度快照
		// 由于订单在submit时已交叉插入，orderbook已包含完整深度，无需额外处理
		let token_0_depth = self.token_0_orderbook.get_depth(None, self.update_id, timestamp);
		let token_1_depth = self.token_1_orderbook.get_depth(None, self.update_id, timestamp);

		// 对比上一秒的深度，更新价格档位变化缓存（内部会保存快照）
		self.update_price_level_changes(&token_0_depth);
		self.update_price_level_changes(&token_1_depth);

		// 推送深度快照到 cache topic（两个token一起推送）
		let _ = crate::output::publish_cache_depth(
			self.event_id,
			self.market_id,
			self.update_id,
			timestamp,
			&token_0_depth,
			&token_1_depth,
			self.token_0_latest_trade_price.clone(),
			self.token_1_latest_trade_price.clone(),
		);

		// 推送价格档位变化到 websocket topic（失败不影响继续执行）
		// 两个token共享同一个update_id，一起推送
		let has_token_0_changes = !self.token_0_bid_changes.is_empty() || !self.token_0_ask_changes.is_empty();
		let has_token_1_changes = !self.token_1_bid_changes.is_empty() || !self.token_1_ask_changes.is_empty();

		if has_token_0_changes || has_token_1_changes {
			let mut changes = HashMap::new();

			if has_token_0_changes {
				let bids: Vec<PriceLevelInfo> = self
					.token_0_bid_changes
					.iter()
					.map(|(price_i32, quantity)| {
						let price_str = format_price(*price_i32);
						let quantity_str = format_quantity(*quantity);
						let total_size = (price_decimal(*price_i32).checked_mul(quantity_decimal(*quantity)).unwrap_or_default()).normalize().to_string();
						PriceLevelInfo { price: price_str, total_quantity: quantity_str, total_size }
					})
					.collect();
				let asks: Vec<PriceLevelInfo> = self
					.token_0_ask_changes
					.iter()
					.map(|(price_i32, quantity)| {
						let price_str = format_price(*price_i32);
						let quantity_str = format_quantity(*quantity);
						let total_size = (price_decimal(*price_i32).checked_mul(quantity_decimal(*quantity)).unwrap_or_default()).normalize().to_string();
						PriceLevelInfo { price: price_str, total_quantity: quantity_str, total_size }
					})
					.collect();
				changes.insert(self.token_0_id.clone(), SingleTokenPriceInfo { latest_trade_price: self.token_0_latest_trade_price.clone(), bids, asks });
			}

			if has_token_1_changes {
				let bids: Vec<PriceLevelInfo> = self
					.token_1_bid_changes
					.iter()
					.map(|(price_i32, quantity)| {
						let price_str = format_price(*price_i32);
						let quantity_str = format_quantity(*quantity);
						let total_size = (price_decimal(*price_i32).checked_mul(quantity_decimal(*quantity)).unwrap_or_default()).normalize().to_string();
						PriceLevelInfo { price: price_str, total_quantity: quantity_str, total_size }
					})
					.collect();
				let asks: Vec<PriceLevelInfo> = self
					.token_1_ask_changes
					.iter()
					.map(|(price_i32, quantity)| {
						let price_str = format_price(*price_i32);
						let quantity_str = format_quantity(*quantity);
						let total_size = (price_decimal(*price_i32).checked_mul(quantity_decimal(*quantity)).unwrap_or_default()).normalize().to_string();
						PriceLevelInfo { price: price_str, total_quantity: quantity_str, total_size }
					})
					.collect();
				changes.insert(self.token_1_id.clone(), SingleTokenPriceInfo { latest_trade_price: self.token_1_latest_trade_price.clone(), bids, asks });
			}

			let _ = crate::output::publish_websocket_price_changes(self.event_id, self.market_id, self.update_id, timestamp, changes);
		}
	}

	// 获取匹配订单（简化版：因为订单已交叉插入，只需遍历单个方向）
	// 买单taker：遍历本方orderbook的asks（已包含相同结果卖单 + 相反结果买单的交叉插入）
	// 卖单taker：遍历本方orderbook的bids（已包含相同结果买单 + 相反结果卖单的交叉插入）
	//
	// 由于交叉插入时保持了order_num顺序，同价格档位内自然按时间优先
	// 返回: (匹配的订单列表(Arc<Order>, 匹配价格), 是否遇到自成交)
	fn get_cross_matching_orders(&self, taker: &Order) -> (Vec<(Arc<Order>, i32)>, bool) {
		let mut result = Vec::new();
		let mut accumulated_quantity = 0u64;
		let mut has_self_trade = false;
		let taker_price = taker.price;
		let target_quantity = taker.remaining_quantity;

		// 选择taker对应的orderbook
		let orderbook = if taker.symbol.token_id == self.token_0_id { &self.token_0_orderbook } else { &self.token_1_orderbook };

		match taker.side {
			OrderSide::Buy => {
				// 买单：遍历asks，价格从小到大
				for (price_key, orders) in orderbook.asks.iter() {
					if *price_key > taker_price {
						break; // 价格超出taker限价，停止
					}
					for order in orders {
						// 自成交检测
						if order.user_id == taker.user_id {
							has_self_trade = true;
							return (result, has_self_trade);
						}
						accumulated_quantity += order.remaining_quantity;
						result.push((Arc::clone(order), *price_key));
						if accumulated_quantity >= target_quantity {
							return (result, has_self_trade);
						}
					}
				}
			}
			OrderSide::Sell => {
				// 卖单：遍历bids，价格从大到小（bids的key是负数，从小到大遍历即价格从大到小）
				for (price_key, orders) in orderbook.bids.iter() {
					let price = -*price_key;
					if price < taker_price {
						break; // 价格低于taker限价，停止
					}
					for order in orders {
						// 自成交检测
						if order.user_id == taker.user_id {
							has_self_trade = true;
							return (result, has_self_trade);
						}
						accumulated_quantity += order.remaining_quantity;
						result.push((Arc::clone(order), price));
						if accumulated_quantity >= target_quantity {
							return (result, has_self_trade);
						}
					}
				}
			}
		}

		(result, has_self_trade)
	}

	pub fn match_order(&mut self, taker: &Order) -> anyhow::Result<(u64, Vec<Trade>, rust_decimal::Decimal, bool)> {
		let mut remaining_quantity = taker.quantity;
		let mut trades = Vec::new();
		let mut taker_total_usdc_amount = rust_decimal::Decimal::ZERO;
		// 使用交叉撮合获取匹配订单，返回(Arc<Order>, 匹配价格)的列表，以及是否遇到自成交
		let (maker_orders_with_price, has_self_trade) = self.get_cross_matching_orders(taker);
		let timestamp = Utc::now().timestamp_millis();

		for (maker_order, match_price) in maker_orders_with_price {
			// 禁止自成交：taker 不能吃自己的单（理论上在 get_cross_matching_orders 已检测）
			if maker_order.user_id == taker.user_id {
				continue;
			}

			let matched_quantity = remaining_quantity.min(maker_order.remaining_quantity);
			let maker_new_quantity = maker_order.remaining_quantity - matched_quantity;
			let maker_price = maker_order.price;
			let maker_order_id = &maker_order.order_id; // 使用引用避免 clone

			// 创建 Trade
			let taker_usdc = get_usdc_amount(price_decimal(match_price), quantity_decimal(matched_quantity));
			let trade = Trade {
				timestamp,
				event_id: self.event_id,
				market_id: self.market_id,
				quantity: format_quantity(matched_quantity),
				taker_usdc_amount: taker_usdc.clone(),
				taker_price: format_price(match_price), //注意这里不同结果的转化为了相反方向的价格
				maker_id: maker_order.user_id,
				maker_privy_user_id: maker_order.privy_id.clone(),
				maker_outcome_name: maker_order.outcome_name.clone(),
				maker_order_id: maker_order_id.clone(),
				maker_order_side: maker_order.side,
				maker_token_id: maker_order.symbol.token_id.clone(),
				maker_usdc_amount: get_usdc_amount(price_decimal(maker_price), quantity_decimal(matched_quantity)),
				maker_price: format_price(maker_price),                                            //这里是maker order的原始价格 交叉撮合maker的价格不一样
				maker_filled_quantity: format_quantity(maker_order.quantity - maker_new_quantity), //没办法filled_quantity要在后续更新maker_order得时候才会更新 这里要先推送
				maker_quantity: format_quantity(maker_order.quantity),
			};

			// 累加 taker_total_usdc_amount
			let taker_usdc_decimal = Decimal::from_str_exact(&taker_usdc).map_err(|e| anyhow::anyhow!("Invalid taker_usdc_amount: {}", e))?;
			taker_total_usdc_amount = checked_add(taker_total_usdc_amount, taker_usdc_decimal, "taker_total_usdc_amount")?;

			trades.push(trade);

			// 判断是 token_0 还是 token_1 的订单
			let is_token_0 = maker_order.symbol.token_id == self.token_0_id;

			//完全成交 从orders和orderbook中直接删除（两个orderbook都要删除）
			if maker_new_quantity == 0 {
				// 根据maker订单的token_id从两个OrderBook中移除
				if is_token_0 {
					self.token_0_orderbook.remove_order(maker_order_id.clone())?;
					self.token_1_orderbook.remove_order(maker_order_id.clone())?; // 移除交叉订单
					self.token_0_orders.remove(maker_order_id);
				} else {
					self.token_1_orderbook.remove_order(maker_order_id.clone())?;
					self.token_0_orderbook.remove_order(maker_order_id.clone())?; // 移除交叉订单
					self.token_1_orders.remove(maker_order_id);
				}
				// 推送 maker 订单完全成交事件到 store
				if let Err(e) = publish_order_change_to_store(OrderChangeEvent::OrderFilled { order_id: maker_order_id.clone(), symbol: maker_order.symbol.clone() }) {
					error!("Failed to publish maker order filled event to store: {}", e);
				}
			} else {
				// 根据maker订单的token_id更新两个OrderBook
				let updated_order = if is_token_0 {
					self.token_1_orderbook.update_order(maker_order_id.clone(), maker_new_quantity)?; // 更新交叉订单
					self.token_0_orderbook.update_order(maker_order_id.clone(), maker_new_quantity)?
				} else {
					self.token_0_orderbook.update_order(maker_order_id.clone(), maker_new_quantity)?; // 更新交叉订单
					self.token_1_orderbook.update_order(maker_order_id.clone(), maker_new_quantity)?
				};
				// 更新对应的orders HashMap（Arc clone）
				let order_id_for_insert = updated_order.order_id.clone();
				if is_token_0 {
					self.token_0_orders.insert(order_id_for_insert, Arc::clone(&updated_order));
				} else {
					self.token_1_orders.insert(order_id_for_insert, Arc::clone(&updated_order));
				}
				// 推送 maker 订单更新事件到 store（部分成交）
				if let Err(e) = publish_order_change_to_store(OrderChangeEvent::OrderUpdated((*updated_order).clone())) {
					error!("Failed to publish maker order updated event to store: {}", e);
				}
			}

			remaining_quantity -= matched_quantity;

			if remaining_quantity == 0 {
				break;
			}
		}

		Ok((remaining_quantity, trades, taker_total_usdc_amount, has_self_trade))
	}

	//订单变化要推到mq 给ws/processor/redis
	pub fn submit_order(&mut self, taker: &mut Order) -> anyhow::Result<()> {
		if !self.validate_order(taker) {
			return Err(anyhow::anyhow!("Invalid order"));
		}

		let (remaining_quantity, trades, taker_total_usdc_amount, has_self_trade) = self.match_order(taker)?;
		taker.remaining_quantity = remaining_quantity;
		taker.filled_quantity = taker.quantity - remaining_quantity;

		// 更新最新成交价格
		if let Some(last_trade) = trades.last() {
			let taker_price = &last_trade.taker_price;
			// 计算另一个token的价格: 1 - taker_price
			let other_price = (rust_decimal::Decimal::ONE - rust_decimal::Decimal::from_str_exact(taker_price).unwrap_or(rust_decimal::Decimal::ZERO)).normalize().to_string();
			if taker.symbol.token_id == self.token_0_id {
				self.token_0_latest_trade_price = taker_price.clone();
				self.token_1_latest_trade_price = other_price;
			} else {
				self.token_1_latest_trade_price = taker_price.clone();
				self.token_0_latest_trade_price = other_price;
			}
		}

		// 有成交就推送成交消息
		if taker.filled_quantity > 0 {
			//不然就是默认的new
			taker.status = OrderStatus::PartiallyFilled;
			if remaining_quantity == 0 {
				taker.status = OrderStatus::Filled;
			}
			// 推送 Processor 消息（部分成交）
			if let Err(e) = publish_to_processor(ProcessorMessage::OrderTraded(OrderTraded {
				taker_symbol: taker.symbol.clone(),
				taker_id: taker.user_id,
				taker_privy_user_id: taker.privy_id.clone(),
				taker_outcome_name: taker.outcome_name.clone(),
				taker_order_id: taker.order_id.clone(),
				taker_order_side: taker.side,
				taker_token_id: taker.symbol.token_id.clone(),
				trades,
			})) {
				error!("Failed to publish order traded event to processor: {}", e);
			}
		}

		//然后看剩余等于0啥也没有表示吃完了 否则看类型 限价单就是挂单 市价单就是吃不完就取消
		if remaining_quantity > 0 {
			// 如果检测到自成交，取消剩余订单（无论是限价单还是市价单）
			if has_self_trade {
				// cancelled_volume = price * quantity - taker_total_usdc_amount (总冻结值 - 已成交总价值)
				let total_freeze = checked_mul(price_decimal(taker.price), quantity_decimal(taker.quantity), "self-trade total_freeze")?;
				let cancelled_volume = checked_sub(total_freeze, taker_total_usdc_amount, "self-trade cancelled_volume")?;
				let cancelled = OrderCancelled {
					order_id: taker.order_id.clone(),
					symbol: taker.symbol.clone(),
					user_id: taker.user_id,
					privy_id: taker.privy_id.clone(),
					cancelled_quantity: format_quantity(taker.remaining_quantity),
					cancelled_volume: cancelled_volume.normalize().to_string(),
				};
				if let Err(e) = publish_to_processor(ProcessorMessage::OrderCancelled(cancelled)) {
					error!("Failed to publish order cancelled event to processor: {}", e);
				}
				return Ok(());
			}

			//只能是限价单才能放在订单簿当作maker并发送订单创建消息 否则取消
			if taker.order_type == OrderType::Limit {
				let submitted = OrderSubmitted {
					order_id: taker.order_id.clone(),
					symbol: taker.symbol.clone(),
					side: taker.side,
					order_type: taker.order_type,
					quantity: format_quantity(taker.quantity),
					price: format_price(taker.price),
					filled_quantity: format_quantity(taker.filled_quantity),
					user_id: taker.user_id,
					privy_id: taker.privy_id.clone(),
					outcome_name: taker.outcome_name.clone(),
				};
				if let Err(e) = publish_to_processor(ProcessorMessage::OrderSubmitted(submitted)) {
					error!("Failed to publish order submitted event to processor: {}", e);
				}

				// 创建Arc<Order>，只clone一次Order，后续使用Arc::clone
				let taker_arc = Arc::new(taker.clone());

				// 根据token_id路由到对应的OrderBook，同时交叉插入到对方OrderBook
				if taker.symbol.token_id == self.token_0_id {
					self.token_0_orderbook.add_order(Arc::clone(&taker_arc))?;
					self.token_1_orderbook.add_cross_order(Arc::clone(&taker_arc))?; // 交叉插入
					self.token_0_orders.insert(taker.order_id.clone(), Arc::clone(&taker_arc));
				} else {
					self.token_1_orderbook.add_order(Arc::clone(&taker_arc))?;
					self.token_0_orderbook.add_cross_order(Arc::clone(&taker_arc))?; // 交叉插入
					self.token_1_orders.insert(taker.order_id.clone(), Arc::clone(&taker_arc));
				}
				// 推送订单创建事件到 store（新订单加入订单簿）
				if let Err(e) = publish_order_change_to_store(OrderChangeEvent::OrderCreated(taker.clone())) {
					error!("Failed to publish order created event to store: {}", e);
				}
			} else {
				//市价单吃不完就取消剩余的那部分 要发送到Processor
				// cancelled_volume = price * quantity - taker_total_usdc_amount (总冻结值 - 已成交总价值)
				let total_freeze = checked_mul(price_decimal(taker.price), quantity_decimal(taker.quantity), "market order total_freeze")?;
				let cancelled_volume = checked_sub(total_freeze, taker_total_usdc_amount, "market order cancelled_volume")?;
				let cancelled = OrderCancelled {
					order_id: taker.order_id.clone(),
					symbol: taker.symbol.clone(),
					user_id: taker.user_id,
					privy_id: taker.privy_id.clone(),
					cancelled_quantity: format_quantity(taker.remaining_quantity),
					cancelled_volume: cancelled_volume.normalize().to_string(),
				};
				if let Err(e) = publish_to_processor(ProcessorMessage::OrderCancelled(cancelled)) {
					error!("Failed to publish order cancelled event to processor: {}", e);
				}
				return Ok(());
			}
		}

		Ok(())
	}

	pub fn cancel_order(&mut self, order_id: String) -> anyhow::Result<()> {
		// 先尝试从token_0订单簿中查找
		let (user_id, privy_id, symbol, remaining_quantity, price) = if let Some(order) = self.token_0_orders.get(&order_id) {
			(order.user_id, order.privy_id.clone(), order.symbol.clone(), order.remaining_quantity, order.price)
		} else if let Some(order) = self.token_1_orders.get(&order_id) {
			(order.user_id, order.privy_id.clone(), order.symbol.clone(), order.remaining_quantity, order.price)
		} else {
			return Err(anyhow::anyhow!("Order not found"));
		};

		// 根据token_id从两个OrderBook中移除（本方+交叉方）
		if symbol.token_id == self.token_0_id {
			self.token_0_orders.remove(&order_id);
			self.token_0_orderbook.remove_order(order_id.clone())?;
			self.token_1_orderbook.remove_order(order_id.clone())?; // 移除交叉订单
		} else {
			self.token_1_orders.remove(&order_id);
			self.token_1_orderbook.remove_order(order_id.clone())?;
			self.token_0_orderbook.remove_order(order_id.clone())?; // 移除交叉订单
		}

		// 推送订单取消事件到 store
		if let Err(e) = publish_order_change_to_store(OrderChangeEvent::OrderCancelled { order_id: order_id.clone(), symbol: symbol.clone() }) {
			error!("Failed to publish order cancelled event to store: {}", e);
		}

		// 推送 Processor 消息（取消成功）
		// 限价单的 cancelled_volume = price * cancelled_quantity
		let cancelled_volume = checked_mul(price_decimal(price), quantity_decimal(remaining_quantity), "limit order cancelled_volume")?;
		let cancelled = OrderCancelled { order_id, symbol, user_id, privy_id, cancelled_quantity: format_quantity(remaining_quantity), cancelled_volume: cancelled_volume.normalize().to_string() };
		if let Err(e) = publish_to_processor(ProcessorMessage::OrderCancelled(cancelled)) {
			error!("Failed to publish order cancelled event to processor: {}", e);
		}

		Ok(())
	}

	pub fn validate_order(&self, order: &Order) -> bool {
		if self.event_id != order.symbol.event_id || self.market_id != order.symbol.market_id {
			return false;
		}

		if order.quantity == 0 {
			return false;
		}
		// if order.order_type == OrderType::Limit && order.price.is_none() {
		// 	return false;
		// }

		if order.user_id == 0 {
			return false;
		}
		true
	}

	/// 分批次取消所有订单
	async fn cancel_all_orders_in_batches(&self, orders: Vec<(String, i64, String, PredictionSymbol, u64, i32)>) -> anyhow::Result<()> {
		use common::consts::{PROCESSOR_MSG_KEY, PROCESSOR_STREAM};

		let total_orders = orders.len();
		let batch_count = total_orders.div_ceil(CANCEL_ORDERS_BATCH_SIZE);

		// 获取 Redis 连接（整个过程只获取一次）
		let mut conn = common::redis_pool::get_engine_output_mq_connection().await?;

		for (batch_index, chunk) in orders.chunks(CANCEL_ORDERS_BATCH_SIZE).enumerate() {
			info!("{}/{} Processing cancel batch {}/{} with {} orders", self.event_id, self.market_id, batch_index + 1, batch_count, chunk.len());

			// 创建 pipeline
			let mut pipe = redis::pipe();

			for (order_id, user_id, privy_id, symbol, remaining_quantity, price) in chunk {
				// 创建取消消息并发送到 processor
				// 限价单的 cancelled_volume = price * cancelled_quantity
				let cancelled_volume = checked_mul(price_decimal(*price), quantity_decimal(*remaining_quantity), "batch cancel cancelled_volume")?;
				let cancelled = OrderCancelled {
					order_id: order_id.clone(),
					symbol: symbol.clone(),
					user_id: *user_id,
					privy_id: privy_id.clone(),
					cancelled_quantity: format_quantity(*remaining_quantity),
					cancelled_volume: cancelled_volume.normalize().to_string(),
				};
				let processor_msg = ProcessorMessage::OrderCancelled(cancelled);
				let processor_json = serde_json::to_string(&processor_msg)?;
				pipe.xadd(PROCESSOR_STREAM, "*", &[(PROCESSOR_MSG_KEY, processor_json.as_str())]);

				// 由市场移除去操作
				// 发送订单取消事件到 store
				// let store_event = OrderChangeEvent::OrderCancelled { order_id: order_id.clone(), symbol: symbol.clone() };
				// let store_json = serde_json::to_string(&store_event)?;
				// pipe.xadd(STORE_STREAM, "*", &[(STORE_MSG_KEY, store_json.as_str())]);
			}

			// 批量执行 pipeline
			let _: Vec<String> = pipe.query_async(&mut conn).await?;

			info!("{}/{} Completed cancel batch {}/{}", self.event_id, self.market_id, batch_index + 1, batch_count);
		}

		Ok(())
	}

	pub async fn run(&mut self) {
		let mut snapshot_interval = interval(Duration::from_secs(1));
		snapshot_interval.tick().await;

		loop {
			select! {
				Some(control) = self.order_receiver.recv() => {
					match control {
						OrderBookControl::SubmitOrder(order) => {
							let mut new_order = order.clone();
							new_order.order_num = self.order_num;
							self.order_num += 1;
							if let Err(e) = self.submit_order(&mut new_order) {
								// 撮合失败，发送拒绝消息
								let rejected = OrderRejected {
									order_id: new_order.order_id.clone(),
									symbol: new_order.symbol.clone(),
									user_id: new_order.user_id,
									privy_id: new_order.privy_id.clone(),
									reason: e.to_string(),
								};
								let _ = publish_to_processor(ProcessorMessage::OrderRejected(rejected));
							}
						}
						OrderBookControl::CancelOrder(order_id) => {
							let _ = self.cancel_order(order_id);
						}
					}
				}
				_ = snapshot_interval.tick() => {
					// 每秒触发快照
					self.handle_snapshot_tick();
				}
				_ = self.exit_receiver.recv() => {
					info!("{}/{} MatchEngine received exit signal, cancelling all orders", self.event_id, self.market_id);

					// 收集所有需要取消的订单
					let mut all_orders: Vec<(String, i64, String, PredictionSymbol, u64, i32)> = Vec::new();

					for (order_id, order) in self.token_0_orders.iter() {
						all_orders.push((order_id.clone(), order.user_id, order.privy_id.clone(), order.symbol.clone(), order.remaining_quantity, order.price));
					}

					for (order_id, order) in self.token_1_orders.iter() {
						all_orders.push((order_id.clone(), order.user_id, order.privy_id.clone(), order.symbol.clone(), order.remaining_quantity, order.price));
					}

					if !all_orders.is_empty() {
						info!("{}/{} Cancelling {} orders in batches", self.event_id, self.market_id, all_orders.len());

						// 分批次取消订单
						if let Err(e) = self.cancel_all_orders_in_batches(all_orders).await {
							error!("{}/{} Failed to cancel all orders: {}", self.event_id, self.market_id, e);
						}
					}

					info!("{}/{} MatchEngine shutting down", self.event_id, self.market_id);
					return;
				}
			}
		}
	}
}
