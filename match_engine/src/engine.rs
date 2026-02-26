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
		engine_types::{Order, OrderSide, OrderStatus, OrderType, PredictionSymbol, SubmitOrderMessage},
		processor_types::{OrderCancelled, OrderRejected, OrderSubmitted, OrderTraded, ProcessorMessage, Trade},
		store_types::OrderChangeEvent,
		websocket_types::{PriceLevelInfo, SingleTokenPriceInfo},
	},
	rust_decimal::{Decimal, prelude::ToPrimitive},
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

/// 匹配订单的结果（get_cross_matching_orders内部使用）
/// 限价买 有filled_quantity/remaining_quantiy/filled_volume/remaining_volume
/// 限价卖 有filled_quantity/remaining_quantiy/filled_volume,remaining_volume为0
/// 市价买 有filled_quantity/filled_volume/remaining_volume,remaining_quantity为0
/// 市价卖 有filled_quantity/remaining_quantity/filled_volume,remaining_volume为0
pub struct CrossMatchingResult {
	pub matched_orders: Vec<(Arc<Order>, i32, u64)>, // (订单, 匹配价格, 成交数量)
	pub filled_quantity: u64,                        //本次撮合实际成交的数量
	pub remaining_quantity: u64,                     //市价买单这个为0
	pub filled_volume: Decimal,                      //本次撮合实际成交的usdc金额
	pub remaining_volume: Decimal, //只有市价买单才有这个字段响应,如果市价买单吃不完那么会取消 或者由于某一个价格档位 单个数量tick的usdc交易额超过了market volume,那么也会剩一点点的也会被取消
	pub has_self_trade: bool,      //是否遇到自成交,是的话剩下的会直接取消 不管是限价单还是市价单
}

/// 匹配订单的最终结果（submit_order使用）
pub struct MatchingResult {
	pub trades: Vec<Trade>,        // 成交记录
	pub filled_quantity: u64,      //本次撮合实际成交的数量
	pub remaining_quantity: u64,   //市价买单这个为0
	pub filled_volume: Decimal,    //本次撮合实际成交的usdc金额
	pub remaining_volume: Decimal, //只有市价买单才有这个字段响应
	pub has_self_trade: bool,      //是否遇到自成交
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
	pub exit_signal: broadcast::Sender<()>,                             // 全局退出信号（用于整个event关闭）
	pub market_exit_signals: HashMap<String, tokio::sync::oneshot::Sender<()>>, // 每个market的独立退出信号（oneshot）
	pub end_timestamp: Option<chrono::DateTime<Utc>>,                   // 市场结束时间戳
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
	pub exit_receiver: broadcast::Receiver<()>,                           // 全局退出信号（event关闭）
	pub market_exit_receiver: Option<tokio::sync::oneshot::Receiver<()>>, // market级别的退出信号（oneshot，在run时取出）
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
	pub fn new(
		event_id: i64,
		market_id: i16,
		token_ids: (String, String),
		max_order_count: u64,
		exit_receiver: broadcast::Receiver<()>,
		market_exit_receiver: tokio::sync::oneshot::Receiver<()>,
	) -> (Self, mpsc::Sender<OrderBookControl>) {
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
			market_exit_receiver: Some(market_exit_receiver),
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
		market_exit_receiver: tokio::sync::oneshot::Receiver<()>,
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
			market_exit_receiver: Some(market_exit_receiver),
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
	pub fn get_cross_matching_orders(&self, taker: &SubmitOrderMessage) -> CrossMatchingResult {
		let mut result = Vec::new();
		let mut accumulated_quantity = 0u64;
		let mut accumulated_volume = Decimal::ZERO;
		let mut has_self_trade = false;
		let taker_price = taker.price;
		let target_quantity = taker.quantity;
		let target_volume = taker.volume;
		// 用于市价买单跟踪剩余volume（外部作用域，避免局部变量丢失）
		let mut market_buy_remaining_volume = target_volume;

		// 选择taker对应的orderbook
		let orderbook = if taker.symbol.token_id == self.token_0_id { &self.token_0_orderbook } else { &self.token_1_orderbook };

		match taker.order_type {
			OrderType::Limit => {
				// 限价单：根据买卖方向跟taker的price比较
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
									let filled_quantity = accumulated_quantity;
									let filled_volume = accumulated_volume;
									let remaining_quantity = target_quantity - accumulated_quantity;
									let remaining_volume = checked_sub(target_volume, accumulated_volume, "limit buy remaining_volume").unwrap_or(Decimal::ZERO);
									return CrossMatchingResult { matched_orders: result, filled_quantity, remaining_quantity, filled_volume, remaining_volume, has_self_trade };
								}
								// 计算本次能吃的数量
								let remaining_needed = target_quantity - accumulated_quantity;
								let match_quantity = order.remaining_quantity.min(remaining_needed);
								let match_cost = checked_mul(price_decimal(*price_key), quantity_decimal(match_quantity), "limit buy match_cost").unwrap_or(Decimal::ZERO);
								accumulated_quantity += match_quantity;
								accumulated_volume = checked_add(accumulated_volume, match_cost, "limit buy accumulated_volume").unwrap_or(accumulated_volume);
								result.push((Arc::clone(order), *price_key, match_quantity));
								if accumulated_quantity >= target_quantity {
									let filled_quantity = accumulated_quantity;
									let filled_volume = accumulated_volume;
									let remaining_quantity = 0;
									let remaining_volume = checked_sub(target_volume, accumulated_volume, "limit buy final remaining_volume").unwrap_or(Decimal::ZERO);
									return CrossMatchingResult { matched_orders: result, filled_quantity, remaining_quantity, filled_volume, remaining_volume, has_self_trade };
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
									let filled_quantity = accumulated_quantity;
									let filled_volume = accumulated_volume;
									let remaining_quantity = target_quantity - accumulated_quantity;
									// 限价卖单：remaining_volume为0，不需要计算
									return CrossMatchingResult { matched_orders: result, filled_quantity, remaining_quantity, filled_volume, remaining_volume: Decimal::ZERO, has_self_trade };
								}
								// 计算本次能吃的数量
								let remaining_needed = target_quantity - accumulated_quantity;
								let match_quantity = order.remaining_quantity.min(remaining_needed);
								let match_income = checked_mul(price_decimal(price), quantity_decimal(match_quantity), "limit sell match_income").unwrap_or(Decimal::ZERO);
								accumulated_quantity += match_quantity;
								accumulated_volume = checked_add(accumulated_volume, match_income, "limit sell accumulated_volume").unwrap_or(accumulated_volume);
								result.push((Arc::clone(order), price, match_quantity));
								if accumulated_quantity >= target_quantity {
									let filled_quantity = accumulated_quantity;
									let filled_volume = accumulated_volume;
									let remaining_quantity = 0;
									// 限价卖单：remaining_volume为0，不需要计算
									return CrossMatchingResult { matched_orders: result, filled_quantity, remaining_quantity, filled_volume, remaining_volume: Decimal::ZERO, has_self_trade };
								}
							}
						}
					}
				}
			}
			OrderType::Market => {
				// 市价单：直接用taker的price比较，不计算平均价
				match taker.side {
					OrderSide::Buy => {
						// 市价买单：使用volume预算而非quantity目标，remaining_quantity固定为0
						// 买单：遍历asks，价格从小到大
						for (price_key, orders) in orderbook.asks.iter() {
							let price = *price_key;
							// 价格超出taker限价，停止（跟限价单一样）
							if price > taker_price {
								break;
							}

							for order in orders {
								// 自成交检测
								if order.user_id == taker.user_id {
									has_self_trade = true;
									let filled_quantity = accumulated_quantity;
									let filled_volume = accumulated_volume;
									return CrossMatchingResult {
										matched_orders: result,
										filled_quantity,
										remaining_quantity: 0,
										filled_volume,
										remaining_volume: market_buy_remaining_volume,
										has_self_trade,
									};
								}

								// 计算本单的最大可购买量: min(order剩余量, 剩余预算/价格)
								let max_quantity_by_volume = {
									let price_dec = price_decimal(price);
									let qty_dec = quantity_decimal(order.remaining_quantity);
									let cost = checked_mul(price_dec, qty_dec, "market buy cost").unwrap_or(Decimal::MAX);
									if cost > market_buy_remaining_volume {
										// 预算不足以吃完整个订单，计算能吃多少
										if price_dec > Decimal::ZERO {
											// affordable_qty_dec = remaining_volume / price_dec
											let affordable_qty_dec = market_buy_remaining_volume.checked_div(price_dec).unwrap_or(Decimal::ZERO);
											// 转换回u64 (乘以100)
											let result = affordable_qty_dec.checked_mul(Decimal::ONE_HUNDRED).unwrap_or(Decimal::ZERO);
											result.to_u64().unwrap_or(0)
										} else {
											0
										}
									} else {
										order.remaining_quantity
									}
								};

								if max_quantity_by_volume == 0 {
									// 预算用完
									let filled_quantity = accumulated_quantity;
									let filled_volume = accumulated_volume;
									return CrossMatchingResult {
										matched_orders: result,
										filled_quantity,
										remaining_quantity: 0,
										filled_volume,
										remaining_volume: market_buy_remaining_volume,
										has_self_trade,
									};
								}

								let match_quantity = order.remaining_quantity.min(max_quantity_by_volume);

								// 计算本次成交的USDC成本
								let match_cost = checked_mul(price_decimal(price), quantity_decimal(match_quantity), "market buy match_cost").unwrap_or(Decimal::ZERO);

								accumulated_quantity += match_quantity;
								accumulated_volume = checked_add(accumulated_volume, match_cost, "market buy accumulated_volume").unwrap_or(accumulated_volume); // 累加 volume
								market_buy_remaining_volume = checked_sub(market_buy_remaining_volume, match_cost, "market buy remaining_volume").unwrap_or(Decimal::ZERO);
								result.push((Arc::clone(order), price, match_quantity));

								if market_buy_remaining_volume <= Decimal::ZERO {
									let filled_quantity = accumulated_quantity;
									let filled_volume = accumulated_volume;
									return CrossMatchingResult { matched_orders: result, filled_quantity, remaining_quantity: 0, filled_volume, remaining_volume: Decimal::ZERO, has_self_trade };
								}
							}
						}
					}
					OrderSide::Sell => {
						// 市价卖单：使用quantity目标，remaining_volume固定为0
						// 卖单：遍历bids，价格从大到小
						for (price_key, orders) in orderbook.bids.iter() {
							let price = -*price_key;
							// 价格低于taker限价，停止（跟限价单一样）
							if price < taker_price {
								break;
							}

							for order in orders {
								// 自成交检测
								if order.user_id == taker.user_id {
									has_self_trade = true;
									let filled_quantity = accumulated_quantity;
									let filled_volume = accumulated_volume;
									let remaining_quantity = target_quantity - accumulated_quantity;
									return CrossMatchingResult { matched_orders: result, filled_quantity, remaining_quantity, filled_volume, remaining_volume: Decimal::ZERO, has_self_trade };
								}

								// 计算本次能吃的数量（不能超过 target_quantity）
								let remaining_needed = target_quantity - accumulated_quantity;
								let match_quantity = order.remaining_quantity.min(remaining_needed);

								// 市价卖单也要累积volume
								let match_income = checked_mul(price_decimal(price), quantity_decimal(match_quantity), "market sell match_income").unwrap_or(Decimal::ZERO);
								accumulated_quantity += match_quantity;
								accumulated_volume = checked_add(accumulated_volume, match_income, "market sell accumulated_volume").unwrap_or(accumulated_volume);
								result.push((Arc::clone(order), price, match_quantity));

								if accumulated_quantity >= target_quantity {
									let filled_quantity = accumulated_quantity;
									let filled_volume = accumulated_volume;
									return CrossMatchingResult { matched_orders: result, filled_quantity, remaining_quantity: 0, filled_volume, remaining_volume: Decimal::ZERO, has_self_trade };
								}
							}
						}
					}
				}
			}
		}

		// 如果遍历完所有订单后还有剩余
		let remaining_quantity = if taker.order_type == OrderType::Market && taker.side == OrderSide::Buy {
			0 // 市价买单没有quantity目标
		} else {
			target_quantity - accumulated_quantity
		};

		let remaining_volume = match (taker.order_type, taker.side) {
			(OrderType::Market, OrderSide::Buy) => market_buy_remaining_volume, // 市价买单使用循环中维护的值
			(OrderType::Market, OrderSide::Sell) => Decimal::ZERO,              // 市价卖单：remaining_volume为0
			(OrderType::Limit, OrderSide::Buy) => checked_sub(target_volume, accumulated_volume, "limit buy final remaining_volume").unwrap_or(Decimal::ZERO), // 限价买单
			(OrderType::Limit, OrderSide::Sell) => Decimal::ZERO,               // 限价卖单：remaining_volume为0，不需要计算
		};

		let filled_quantity = accumulated_quantity;
		let filled_volume = accumulated_volume;

		CrossMatchingResult { matched_orders: result, filled_quantity, remaining_quantity, filled_volume, remaining_volume, has_self_trade }
	}

	pub fn match_order(&mut self, taker: &SubmitOrderMessage) -> anyhow::Result<MatchingResult> {
		let mut trades = Vec::new();
		// let mut taker_total_usdc_amount = Decimal::ZERO;
		// 使用交叉撮合获取匹配订单
		let cross_matching_result = self.get_cross_matching_orders(taker);
		let timestamp = Utc::now().timestamp_millis();

		for (maker_order, match_price, matched_quantity) in cross_matching_result.matched_orders {
			// 禁止自成交：taker 不能吃自己的单（理论上在 get_cross_matching_orders 已检测）
			if maker_order.user_id == taker.user_id {
				continue;
			}

			// 直接使用get_cross_matching_orders返回的matched_quantity
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
			// let taker_usdc_decimal = Decimal::from_str_exact(&taker_usdc).map_err(|e| anyhow::anyhow!("Invalid taker_usdc_amount: {}", e))?;
			// taker_total_usdc_amount = checked_add(taker_total_usdc_amount, taker_usdc_decimal, "taker_total_usdc_amount")?;

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
		}

		// Construct final MatchingResult with trades
		Ok(MatchingResult {
			trades,
			filled_quantity: cross_matching_result.filled_quantity,
			remaining_quantity: cross_matching_result.remaining_quantity,
			filled_volume: cross_matching_result.filled_volume,
			remaining_volume: cross_matching_result.remaining_volume,
			has_self_trade: cross_matching_result.has_self_trade,
		})
	}

	/// 验证订单消息
	pub fn validate_order_message(&self, order_msg: &SubmitOrderMessage) -> bool {
		if self.event_id != order_msg.symbol.event_id || self.market_id != order_msg.symbol.market_id {
			return false;
		}

		// quantity=0 只对市价买单有效
		if order_msg.quantity == 0 && !(order_msg.order_type == OrderType::Market && order_msg.side == OrderSide::Buy) {
			return false;
		}

		if order_msg.user_id == 0 {
			return false;
		}

		// 价格范围验证
		if !(10..=9990).contains(&order_msg.price) {
			return false;
		}

		true
	}

	//订单变化要推到mq 给ws/processor/redis
	pub fn submit_order(&mut self, taker_msg: &SubmitOrderMessage, order_num: u64) -> anyhow::Result<()> {
		// 完整验证
		if !self.validate_order_message(taker_msg) {
			return Err(anyhow::anyhow!("Invalid order"));
		}

		let matching_result = self.match_order(taker_msg)?;
		let remaining_quantity = matching_result.remaining_quantity;
		let remaining_volume = matching_result.remaining_volume;
		let filled_quantity = matching_result.filled_quantity;
		let _filled_volume = matching_result.filled_volume;
		let trades = matching_result.trades;
		let has_self_trade = matching_result.has_self_trade;

		// 更新最新成交价格
		if let Some(last_trade) = trades.last() {
			let taker_price = &last_trade.taker_price;
			// 计算另一个token的价格: 1 - taker_price
			let other_price = (rust_decimal::Decimal::ONE - rust_decimal::Decimal::from_str_exact(taker_price).unwrap_or(rust_decimal::Decimal::ZERO)).normalize().to_string();
			if taker_msg.symbol.token_id == self.token_0_id {
				self.token_0_latest_trade_price = taker_price.clone();
				self.token_1_latest_trade_price = other_price;
			} else {
				self.token_1_latest_trade_price = taker_price.clone();
				self.token_0_latest_trade_price = other_price;
			}
		}

		// 有成交就推送成交消息
		if filled_quantity > 0 {
			// 推送 Processor 消息（部分成交）
			if let Err(e) = publish_to_processor(ProcessorMessage::OrderTraded(OrderTraded {
				taker_symbol: taker_msg.symbol.clone(),
				taker_id: taker_msg.user_id,
				taker_privy_user_id: taker_msg.privy_id.clone(),
				taker_outcome_name: taker_msg.outcome_name.clone(),
				taker_order_id: taker_msg.order_id.clone(),
				taker_order_side: taker_msg.side,
				taker_token_id: taker_msg.symbol.token_id.clone(),
				trades,
			})) {
				error!("Failed to publish order traded event to processor: {}", e);
			}
		}

		// 如果检测到自成交，取消剩余订单（无论是限价单还是市价单）
		if has_self_trade {
			// cancelled_volume直接使用remaining_volume
			let cancelled = OrderCancelled {
				order_id: taker_msg.order_id.clone(),
				symbol: taker_msg.symbol.clone(),
				user_id: taker_msg.user_id,
				privy_id: taker_msg.privy_id.clone(),
				cancelled_quantity: format_quantity(remaining_quantity),
				cancelled_volume: remaining_volume.normalize().to_string(),
			};
			if let Err(e) = publish_to_processor(ProcessorMessage::OrderCancelled(cancelled)) {
				error!("Failed to publish order cancelled event to processor: {}", e);
			}
			return Ok(());
		}

		//然后看剩余等于0啥也没有表示吃完了 否则看类型 限价单就是挂单 市价单就是吃不完就取消

		//只能是限价单才能放在订单簿当作maker并发送订单创建消息 否则取消
		if taker_msg.order_type == OrderType::Limit {
			if remaining_quantity > 0 {
				let submitted = OrderSubmitted {
					order_id: taker_msg.order_id.clone(),
					symbol: taker_msg.symbol.clone(),
					side: taker_msg.side,
					order_type: taker_msg.order_type,
					quantity: format_quantity(taker_msg.quantity),
					price: format_price(taker_msg.price),
					filled_quantity: format_quantity(filled_quantity),
					user_id: taker_msg.user_id,
					privy_id: taker_msg.privy_id.clone(),
					outcome_name: taker_msg.outcome_name.clone(),
				};
				if let Err(e) = publish_to_processor(ProcessorMessage::OrderSubmitted(submitted)) {
					error!("Failed to publish order submitted event to processor: {}", e);
				}

				// 从SubmitOrderMessage构造Order（限价单）
				let mut limit_order = Order::new(
					taker_msg.order_id.clone(),
					taker_msg.symbol.clone(),
					taker_msg.side,
					taker_msg.order_type,
					taker_msg.quantity,
					taker_msg.price,
					taker_msg.user_id,
					taker_msg.privy_id.clone(),
					taker_msg.outcome_name.clone(),
				)?;
				// 设置撮合结果
				limit_order.filled_quantity = filled_quantity;
				limit_order.remaining_quantity = remaining_quantity;
				limit_order.order_num = order_num;
				limit_order.status = if filled_quantity > 0 { OrderStatus::PartiallyFilled } else { OrderStatus::New };

				// 创建Arc<Order>，只clone一次Order，后续使用Arc::clone
				let taker_arc = Arc::new(limit_order.clone());

				// 根据token_id路由到对应的OrderBook，同时交叉插入到对方OrderBook
				if taker_msg.symbol.token_id == self.token_0_id {
					self.token_0_orderbook.add_order(Arc::clone(&taker_arc))?;
					self.token_1_orderbook.add_cross_order(Arc::clone(&taker_arc))?; // 交叉插入
					self.token_0_orders.insert(taker_msg.order_id.clone(), Arc::clone(&taker_arc));
				} else {
					self.token_1_orderbook.add_order(Arc::clone(&taker_arc))?;
					self.token_0_orderbook.add_cross_order(Arc::clone(&taker_arc))?; // 交叉插入
					self.token_1_orders.insert(taker_msg.order_id.clone(), Arc::clone(&taker_arc));
				}
				// 推送订单创建事件到 store（新订单加入订单簿）
				if let Err(e) = publish_order_change_to_store(OrderChangeEvent::OrderCreated(limit_order)) {
					error!("Failed to publish order created event to store: {}", e);
				}
			}
		} else {
			//市价单吃不完就取消剩余的那部分 要发送到Processor
			// cancelled_volume直接使用remaining_volume
			if taker_msg.order_type == OrderType::Market && ((taker_msg.side == OrderSide::Buy && remaining_volume.gt(&Decimal::ZERO)) || (taker_msg.side == OrderSide::Sell && remaining_quantity > 0))
			{
				let cancelled = OrderCancelled {
					order_id: taker_msg.order_id.clone(),
					symbol: taker_msg.symbol.clone(),
					user_id: taker_msg.user_id,
					privy_id: taker_msg.privy_id.clone(),
					cancelled_quantity: format_quantity(remaining_quantity),    //sell
					cancelled_volume: remaining_volume.normalize().to_string(), //buy
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

	// 提取退出逻辑为独立方法
	async fn shutdown_and_cancel_all_orders(&mut self) {
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
	}

	pub async fn run(&mut self) {
		let mut snapshot_interval = interval(Duration::from_secs(1));
		snapshot_interval.tick().await;

		// 取出 market_exit_receiver（需要从 self 中取出避免在 select! 中 borrow 冲突）
		// 使用 &mut 引用避免在 loop 中被 move
		let mut market_exit_rx = self.market_exit_receiver.take().expect("market_exit_receiver should be Some");

		loop {
			select! {
				Some(control) = self.order_receiver.recv() => {
					match control {
						OrderBookControl::SubmitOrder(order_msg) => {
							let order_num = self.order_num;
							self.order_num += 1;
							if let Err(e) = self.submit_order(&order_msg, order_num) {
								// 撮合失败，发送拒绝消息
								let rejected = OrderRejected {
									order_id: order_msg.order_id.clone(),
									symbol: order_msg.symbol.clone(),
									user_id: order_msg.user_id,
									privy_id: order_msg.privy_id.clone(),
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
					info!("{}/{} MatchEngine received global exit signal, cancelling all orders", self.event_id, self.market_id);
					self.shutdown_and_cancel_all_orders().await;
					return;
				}
				_ = &mut market_exit_rx => {
					info!("{}/{} MatchEngine received market exit signal, cancelling all orders", self.event_id, self.market_id);
					self.shutdown_and_cancel_all_orders().await;
					return;
				}
			}
		}
	}
}
