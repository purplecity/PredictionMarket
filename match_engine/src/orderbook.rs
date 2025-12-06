use {
	crate::{
		helper::{format_price, format_quantity},
		types::{OrderBookDepth, OrderBookStats, PriceLevel},
	},
	anyhow::Ok,
	common::engine_types::{Order, OrderSide, OrderStatus, PredictionSymbol},
	smallvec::SmallVec,
	std::{
		collections::{BTreeMap, HashMap},
		sync::Arc,
	},
	tokio::sync::RwLock,
	tracing::debug,
};

/// 每个价格档位的订单列表，使用 SmallVec 避免小数量时的堆分配
pub type OrderVec = SmallVec<[Arc<Order>; 16]>;

#[derive(Debug)]
pub struct OrderBook {
	pub symbol: PredictionSymbol,
	pub bids: BTreeMap<i32, OrderVec>, // SmallVec中是按照时间有序的 因为是channel中过来的 所以不用排序 就算时间戳相同也不影响
	pub asks: BTreeMap<i32, OrderVec>,
	pub order_price_map: HashMap<String, (OrderSide, i32)>,
}

impl OrderBook {
	pub fn new(symbol: PredictionSymbol) -> Self {
		Self { symbol, bids: BTreeMap::new(), asks: BTreeMap::new(), order_price_map: HashMap::new() }
	}

	/// 从订单列表构建订单簿
	pub fn build(symbol: PredictionSymbol, orders: Vec<Arc<Order>>) -> anyhow::Result<Self> {
		let mut orderbook = Self::new(symbol);
		for order in orders {
			orderbook.add_order(order)?;
		}
		Ok(orderbook)
	}

	pub fn add_order(&mut self, order: Arc<Order>) -> anyhow::Result<()> {
		if self.symbol != order.symbol {
			return Err(anyhow::anyhow!(format!("Symbol mismatch: expected {}, got {}", self.symbol, order.symbol)));
		}

		if order.remaining_quantity == 0 {
			return Err(anyhow::anyhow!("Order quantity must be positive"));
		}

		let price_key = order.price;
		let order_id = order.order_id.clone();
		match order.side {
			OrderSide::Buy => {
				let price_key = -price_key;
				self.order_price_map.insert(order.order_id.clone(), (OrderSide::Buy, price_key));
				self.bids.entry(price_key).or_default().push(order);
			}
			OrderSide::Sell => {
				self.order_price_map.insert(order.order_id.clone(), (OrderSide::Sell, price_key));
				self.asks.entry(price_key).or_default().push(order);
			}
		}

		debug!("Added order: {} to orderbook for {}", order_id, self.symbol);
		Ok(())
	}

	/// 添加交叉订单：将订单以相反方向、相反价格插入到本订单簿
	/// 用于交叉撮合：买Yes的订单会以opposite_result_price作为卖单插入到No的订单簿
	/// - 买单 -> 插入为卖单 (asks)，价格使用 opposite_result_price
	/// - 卖单 -> 插入为买单 (bids)，价格使用 opposite_result_price
	pub fn add_cross_order(&mut self, order: Arc<Order>) -> anyhow::Result<()> {
		if order.remaining_quantity == 0 {
			return Err(anyhow::anyhow!("Order quantity must be positive"));
		}

		let order_id = order.order_id.clone();
		let cross_price = order.opposite_result_price;

		match order.side {
			OrderSide::Buy => {
				// 买单交叉插入为卖单
				self.order_price_map.insert(order.order_id.clone(), (OrderSide::Sell, cross_price));
				self.asks.entry(cross_price).or_default().push(order);
			}
			OrderSide::Sell => {
				// 卖单交叉插入为买单
				let price_key = -cross_price;
				self.order_price_map.insert(order.order_id.clone(), (OrderSide::Buy, price_key));
				self.bids.entry(price_key).or_default().push(order);
			}
		}

		debug!("Added cross order: {} to orderbook for {}", order_id, self.symbol);
		Ok(())
	}

	pub fn remove_order(&mut self, order_id: String) -> anyhow::Result<Arc<Order>> {
		let (side, price_key) = self.order_price_map.remove(&order_id).ok_or(anyhow::anyhow!("Order not found"))?;
		let orderbook = match side {
			OrderSide::Buy => &mut self.bids,
			OrderSide::Sell => &mut self.asks,
		};

		let orders = orderbook.get_mut(&price_key).ok_or(anyhow::anyhow!("Price level not found"))?;
		let index = orders.iter().position(|o| o.order_id == order_id).ok_or(anyhow::anyhow!("Order not found"))?;
		let order = orders.remove(index);
		if orders.is_empty() {
			orderbook.remove(&price_key);
		}

		debug!("Removed order: {} from orderbook for {}", order_id, self.symbol);
		Ok(order)
	}

	/// 更新订单数量，返回更新后的订单引用
	/// 注意：由于订单存在多个 Arc 引用（orderbook + token_X_orders + 交叉orderbook），
	/// 无法使用 Arc::get_mut 原地修改，必须 clone 后替换
	pub fn update_order(&mut self, order_id: String, new_quantity: u64) -> anyhow::Result<Arc<Order>> {
		let (side, price_key) = self.order_price_map.get(&order_id).ok_or(anyhow::anyhow!("Order not found"))?;
		let orderbook = match side {
			OrderSide::Buy => &mut self.bids,
			OrderSide::Sell => &mut self.asks,
		};

		let orders = orderbook.get_mut(price_key).ok_or(anyhow::anyhow!("Price level not found"))?;
		let index = orders.iter().position(|o| o.order_id == order_id).ok_or(anyhow::anyhow!("Order not found"))?;

		// Clone 并更新订单
		let mut updated = (*orders[index]).clone();
		let old_quantity = updated.remaining_quantity;
		updated.remaining_quantity = new_quantity;
		updated.filled_quantity = updated.quantity - new_quantity;

		if new_quantity == 0 {
			updated.status = OrderStatus::Filled;
		} else if updated.filled_quantity > 0 {
			updated.status = OrderStatus::PartiallyFilled;
		}

		orders[index] = Arc::new(updated);

		debug!("Updated order: {} quantity from {} to {}", order_id, old_quantity, new_quantity);
		Ok(Arc::clone(&orders[index]))
	}

	/// 仅使用与相同结果的撮合 现已不用
	pub fn get_matching_orders(&self, taker: &Order) -> Vec<Arc<Order>> {
		let mut matching_orders = Vec::new();
		let mut matched_quantity = 0;
		match taker.side {
			OrderSide::Buy => {
				let max_price_level = taker.price;
				'outer: for (_, orders) in self.asks.range(..max_price_level + 1) {
					for order in orders.iter() {
						matched_quantity += order.remaining_quantity;
						matching_orders.push(Arc::clone(order));
						if matched_quantity >= taker.quantity {
							break 'outer;
						}
					}
				}
			}
			OrderSide::Sell => {
				let max_price_level = -taker.price;
				'outer: for (_, orders) in self.bids.range(..max_price_level - 1) {
					for order in orders.iter() {
						matched_quantity += order.remaining_quantity;
						matching_orders.push(Arc::clone(order));
						if matched_quantity >= taker.quantity {
							break 'outer;
						}
					}
				}
			}
		}
		matching_orders
	}

	//一些统计
	pub fn best_bid(&self) -> Option<String> {
		self.bids.first_key_value().map(|(price, _)| format_price(-*price))
	}

	pub fn best_bid_i32(&self) -> Option<i32> {
		self.bids.first_key_value().map(|(price, _)| -*price)
	}

	pub fn best_ask(&self) -> Option<String> {
		self.asks.last_key_value().map(|(price, _)| format_price(*price))
	}

	pub fn best_ask_i32(&self) -> Option<i32> {
		self.asks.last_key_value().map(|(price, _)| *price)
	}

	pub fn get_spread(&self) -> Option<String> {
		if let (Some(best_bid_i32), Some(best_ask_i32)) = (self.best_bid_i32(), self.best_ask_i32()) { Some(format_price(best_ask_i32 - best_bid_i32)) } else { None }
	}

	//订单簿深度 可以选择多少档
	pub fn get_depth(&self, max_depth: Option<usize>, update_id: u64, timestamp: i64) -> OrderBookDepth {
		let depth = max_depth.unwrap_or(10);
		let mut bids = Vec::new();
		let mut asks = Vec::new();

		for (price, orders) in self.bids.iter().take(depth) {
			let total_quantity_u64 = orders.iter().map(|o| o.remaining_quantity).sum::<u64>();
			let total_quantity = format_quantity(total_quantity_u64);
			bids.push(PriceLevel { price: format_price(-*price), price_i32: -*price, total_quantity, total_quantity_u64, order_count: orders.len() });
		}

		for (price, orders) in self.asks.iter().take(depth) {
			let total_quantity_u64 = orders.iter().map(|o| o.remaining_quantity).sum::<u64>();
			let total_quantity = format_quantity(total_quantity_u64);
			asks.push(PriceLevel { price: format_price(*price), price_i32: *price, total_quantity, total_quantity_u64, order_count: orders.len() });
		}
		OrderBookDepth { symbol: self.symbol.clone(), bids, asks, timestamp, update_id }
	}

	pub fn get_orderbook_stats(&self) -> OrderBookStats {
		OrderBookStats {
			symbol: self.symbol.clone(),
			bid_levels: self.bids.len(),
			ask_levels: self.asks.len(),
			total_bid_orders: self.bids.values().map(|orders| orders.len()).sum(),
			total_ask_orders: self.asks.values().map(|orders| orders.len()).sum(),
			total_bid_quantity: self.bids.values().flat_map(|orders| orders.iter()).map(|o| o.remaining_quantity).sum(),
			total_ask_quantity: self.asks.values().flat_map(|orders| orders.iter()).map(|o| o.remaining_quantity).sum(),
		}
	}
}

#[derive(Debug, Clone)]
pub struct SafeOrderBook {
	inner: Arc<RwLock<OrderBook>>,
}

impl SafeOrderBook {
	pub fn new(symbol: PredictionSymbol) -> Self {
		Self { inner: Arc::new(RwLock::new(OrderBook::new(symbol))) }
	}

	pub async fn add_order(&self, order: Arc<Order>) -> anyhow::Result<()> {
		self.inner.write().await.add_order(order)
	}

	pub async fn remove_order(&self, order_id: String) -> anyhow::Result<Arc<Order>> {
		self.inner.write().await.remove_order(order_id)
	}

	pub async fn update_order(&self, order_id: String, new_quantity: u64) -> anyhow::Result<Arc<Order>> {
		self.inner.write().await.update_order(order_id, new_quantity)
	}

	pub async fn get_matching_orders(&self, taker: &Order) -> Vec<Arc<Order>> {
		self.inner.read().await.get_matching_orders(taker)
	}

	pub async fn best_bid(&self) -> Option<String> {
		self.inner.read().await.best_bid()
	}

	pub async fn best_bid_i32(&self) -> Option<i32> {
		self.inner.read().await.best_bid_i32()
	}

	pub async fn best_ask(&self) -> Option<String> {
		self.inner.read().await.best_ask()
	}

	pub async fn best_ask_i32(&self) -> Option<i32> {
		self.inner.read().await.best_ask_i32()
	}

	pub async fn get_spread(&self) -> Option<String> {
		self.inner.read().await.get_spread()
	}

	pub async fn get_depth(&self, max_depth: Option<usize>, update_id: u64, timestamp: i64) -> OrderBookDepth {
		self.inner.read().await.get_depth(max_depth, update_id, timestamp)
	}

	pub async fn get_orderbook_stats(&self) -> OrderBookStats {
		self.inner.read().await.get_orderbook_stats()
	}
}
