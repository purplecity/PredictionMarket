use {
	chrono::Utc,
	common::{
		consts::{STORE_STREAM, *},
		engine_types::{Order, PredictionSymbol},
		event_types::EngineMQEventCreate,
		redis_pool,
		store_types::{AllOrdersSnapshot, OrderChangeEvent, OrderSnapshot},
	},
	redis::AsyncCommands,
	std::{
		collections::HashMap,
		fs::{self, File},
		io::Write,
		path::Path,
		str::FromStr,
		sync::Arc,
	},
	tokio::sync::RwLock,
	tracing::{error, info},
};

/// 订单存储数据（所有字段在同一把锁下，保证原子性）
struct OrderStorageData {
	/// 内存中的订单：symbol -> order_id -> Order
	orders: HashMap<String, HashMap<String, Order>>,
	/// 市场信息：event_id -> EngineMQEventCreate
	events: HashMap<i64, EngineMQEventCreate>,
	/// 每个 market 的最新 update_id：(event_id, market_id) -> update_id
	market_update_ids: HashMap<(i64, i16), u64>,
	/// 已处理的最新消息 ID
	last_message_id: Option<String>,
}

/// 订单存储管理器
pub struct OrderStorage {
	/// 所有数据在同一把锁下，保证消息ID和orders/events更新的原子性
	data: Arc<RwLock<OrderStorageData>>,
}

impl Default for OrderStorage {
	fn default() -> Self {
		Self::new()
	}
}

impl OrderStorage {
	pub fn new() -> Self {
		// 确保数据目录存在
		if let Err(e) = fs::create_dir_all(STORE_DATA_DIR) {
			error!("Failed to create data directory {}: {}", STORE_DATA_DIR, e);
		}

		Self { data: Arc::new(RwLock::new(OrderStorageData { orders: HashMap::new(), events: HashMap::new(), market_update_ids: HashMap::new(), last_message_id: None })) }
	}

	/// 获取已处理的最新消息 ID
	pub async fn get_last_message_id(&self) -> Option<String> {
		let data = self.data.read().await;
		data.last_message_id.clone()
	}

	/// 处理订单变化事件
	/// 除了添加市场移除市场之外 不要打日志 否则可能会爆盘
	/// 所有操作在同一把锁下，保证消息ID和orders/events更新的原子性
	pub async fn handle_order_change(&self, event: OrderChangeEvent, message_id: String) {
		let mut data = self.data.write().await;

		match event {
			OrderChangeEvent::OrderCreated(order) => {
				//info!("Processing OrderCreated event: {}", order);
				let symbol_key = order.symbol.to_string();
				data.orders.entry(symbol_key).or_insert_with(HashMap::new).insert(order.order_id.clone(), order);
			}
			OrderChangeEvent::OrderUpdated(order) => {
				//info!("Processing OrderUpdated event: {}", order);
				let symbol_key = order.symbol.to_string();
				if let Some(symbol_orders) = data.orders.get_mut(&symbol_key) {
					symbol_orders.insert(order.order_id.clone(), order);
				} else {
					// 如果 symbol 不存在，创建它
					let mut symbol_orders = HashMap::new();
					symbol_orders.insert(order.order_id.clone(), order.clone());
					data.orders.insert(symbol_key, symbol_orders);
				}
			}

			OrderChangeEvent::OrderCancelled { order_id, symbol } | OrderChangeEvent::OrderFilled { order_id, symbol } => {
				//info!("Processing OrderCancelled or OrderFilled event: order_id={}, symbol={}", order_id, symbol);
				// 订单取消或者完全成交，从存储中删除
				let symbol_key = symbol.to_string();
				if let Some(symbol_orders) = data.orders.get_mut(&symbol_key) {
					symbol_orders.remove(&order_id);
					// 如果该 symbol 没有订单了，删除 symbol
					if symbol_orders.is_empty() {
						data.orders.remove(&symbol_key);
					}
				}
			}
			OrderChangeEvent::EventAdded(event_create) => {
				info!("Processing EventAdded event: event_id={}, markets={:?}, end_timestamp={:?}", event_create.event_id, event_create.markets, event_create.end_date);
				// 添加市场信息
				data.events.insert(event_create.event_id, event_create);
			}
			OrderChangeEvent::EventRemoved(event_id) => {
				info!("Processing EventRemoved event: event_id={}", event_id);
				// 移除市场并清理该市场的所有订单
				let event_prefix = format!("{}{}", event_id, SYMBOL_SEPARATOR);

				// 收集需要删除的 symbol key
				let mut symbols_to_remove = Vec::new();
				for symbol_key in data.orders.keys() {
					if symbol_key.starts_with(&event_prefix) {
						symbols_to_remove.push(symbol_key.clone());
					}
				}

				// 删除所有属于该市场的订单
				let mut total_orders_removed = 0;
				for symbol_key in symbols_to_remove {
					if let Some(symbol_orders) = data.orders.remove(&symbol_key) {
						total_orders_removed += symbol_orders.len();
					}
				}

				// 移除该 event 的所有 market update_ids
				data.market_update_ids.retain(|(eid, _), _| *eid != event_id);

				// 然后移除市场信息
				data.events.remove(&event_id);

				if total_orders_removed > 0 {
					info!("Event {} removed: {} orders cleaned up", event_id, total_orders_removed);
				} else {
					info!("Event {} removed", event_id);
				}
			}
			OrderChangeEvent::MarketUpdateId { event_id, market_id, update_id } => {
				// 更新该 market 的最新 update_id
				data.market_update_ids.insert((event_id, market_id), update_id);
			}
		}

		// 处理完事件后，更新已处理的最新消息 ID（在同一把锁下，保证原子性）
		data.last_message_id = Some(message_id);
	}

	/// 保存快照到文件
	pub async fn save_snapshot(&self) -> anyhow::Result<()> {
		let data = self.data.read().await;

		let current_timestamp = Utc::now().timestamp_millis();

		// 构建快照数据
		let mut snapshots = Vec::new();
		let mut skipped_expired_events = 0;
		let mut skipped_expired_orders = 0;

		for (symbol_str, symbol_orders) in data.orders.iter() {
			// 只保存活跃订单（New 和 PartiallyFilled）
			use common::engine_types::OrderStatus;
			let active_orders: Vec<Order> = symbol_orders.values().filter(|o| matches!(o.status, OrderStatus::New | OrderStatus::PartiallyFilled)).cloned().collect();

			if !active_orders.is_empty() {
				// 解析 symbol
				match PredictionSymbol::from_str(symbol_str) {
					Ok(symbol) => {
						// 检查市场是否已过期
						if let Some(event_create) = data.events.get(&symbol.event_id) {
							if let Some(end_date) = event_create.end_date
								&& end_date.timestamp_millis() < current_timestamp
							{
								// 市场已过期，跳过该市场的订单
								skipped_expired_events += 1;
								skipped_expired_orders += active_orders.len();
								continue;
							}
						} else {
							// 市场信息不存在，跳过（不应该发生，但为了安全）
							continue;
						}

						snapshots.push(OrderSnapshot { symbol, orders: active_orders, timestamp: Utc::now().timestamp_millis() });
					}
					Err(e) => {
						error!("Failed to parse symbol {}: {}", symbol_str, e);
						continue;
					}
				}
			}
		}

		// 只保存未过期的市场信息，并转换为 SnapshotEvent 类型（带 update_id）
		let events_snapshot: HashMap<i64, common::store_types::SnapshotEvent> = data
			.events
			.iter()
			.filter(|(_, event_create)| {
				// 如果市场没有结束时间，或者结束时间未到，则保存
				event_create.end_date.map(|dt| dt.timestamp_millis() >= current_timestamp).unwrap_or(true)
			})
			.map(|(event_id, event_create)| {
				// 将 EngineMQEventCreate 转换为 SnapshotEvent
				let markets: HashMap<String, common::store_types::SnapshotMarket> = event_create
					.markets
					.iter()
					.map(|(market_id_str, engine_market)| {
						// 从 market_update_ids 中获取该 market 的最新 update_id，默认为 0
						let market_id: i16 = market_id_str.parse().unwrap_or(0);
						let update_id = data.market_update_ids.get(&(*event_id, market_id)).copied().unwrap_or(0);

						let snapshot_market =
							common::store_types::SnapshotMarket { market_id: engine_market.market_id, outcomes: engine_market.outcomes.clone(), token_ids: engine_market.token_ids.clone(), update_id };
						(market_id_str.clone(), snapshot_market)
					})
					.collect();

				let snapshot_event = common::store_types::SnapshotEvent { event_id: *event_id, markets, end_date: event_create.end_date };

				(*event_id, snapshot_event)
			})
			.collect();

		// 获取已处理的最新消息 ID（在同一把锁下读取）
		let last_message_id = data.last_message_id.clone();

		// 释放读锁，避免后续操作阻塞其他任务
		drop(data);

		let all_snapshot = AllOrdersSnapshot { snapshots, events: events_snapshot, timestamp: Utc::now().timestamp_millis(), last_message_id: last_message_id.clone() };

		// 保存到文件并刷盘确保已写入
		let file_path = Path::new(STORE_DATA_DIR).join(ORDERS_SNAPSHOT_FILE);
		let json = serde_json::to_string_pretty(&all_snapshot)?;
		let mut file = File::create(&file_path)?;
		file.write_all(json.as_bytes())?;
		file.sync_all()?; // 刷盘确保数据已写入磁盘

		// 如果有消息 ID,清理 Redis stream 中的旧消息
		// 不阻塞快照保存的返回，清理失败也不影响快照保存的成功
		if let Some(msg_id) = last_message_id
			&& let Err(e) = cleanup_old_messages(&msg_id).await
		{
			error!("Failed to cleanup old messages after snapshot: {}", e);
		}

		if skipped_expired_orders > 0 {
			info!(
				"Saved snapshot to {} ({} symbols, {} total orders, {} events), skipped {} orders from {} expired events",
				file_path.display(),
				all_snapshot.snapshots.len(),
				all_snapshot.snapshots.iter().map(|s| s.orders.len()).sum::<usize>(),
				all_snapshot.events.len(),
				skipped_expired_orders,
				skipped_expired_events
			);
		} else {
			info!(
				"Saved snapshot to {} ({} symbols, {} total orders, {} events)",
				file_path.display(),
				all_snapshot.snapshots.len(),
				all_snapshot.snapshots.iter().map(|s| s.orders.len()).sum::<usize>(),
				all_snapshot.events.len()
			);
		}

		Ok(())
	}

	/// 从文件加载快照并恢复到内存
	pub async fn load_snapshot(&self) -> anyhow::Result<Option<String>> {
		let file_path = Path::new(STORE_DATA_DIR).join(ORDERS_SNAPSHOT_FILE);

		if !file_path.exists() {
			info!("Snapshot file not found: {}, starting with empty storage", file_path.display());
			return Ok(None);
		}

		let json = fs::read_to_string(&file_path)?;
		let all_snapshot: AllOrdersSnapshot = serde_json::from_str(&json)?;

		// 在同一把锁下加载所有数据，保证原子性
		let mut data = self.data.write().await;

		// 恢复消息 ID
		data.last_message_id = all_snapshot.last_message_id.clone();

		// 加载市场信息并恢复 market_update_ids
		data.events.clear();
		data.market_update_ids.clear();
		let events_count = {
			for (event_id, snapshot_event) in all_snapshot.events {
				// 恢复每个 market 的 update_id
				for snapshot_market in snapshot_event.markets.values() {
					data.market_update_ids.insert((event_id, snapshot_market.market_id), snapshot_market.update_id);
				}

				// 将 SnapshotEvent 转换回 EngineMQEventCreate 用于内存存储
				let markets: HashMap<String, common::event_types::EngineMQEventMarket> = snapshot_event
					.markets
					.into_iter()
					.map(|(market_id_str, snapshot_market)| {
						let engine_market = common::event_types::EngineMQEventMarket { market_id: snapshot_market.market_id, outcomes: snapshot_market.outcomes, token_ids: snapshot_market.token_ids };
						(market_id_str, engine_market)
					})
					.collect();

				let engine_event = EngineMQEventCreate { event_id, markets, end_date: snapshot_event.end_date };

				data.events.insert(event_id, engine_event);
			}
			data.events.len()
		};

		// 然后加载订单，使用市场信息过滤过期市场
		let current_timestamp = Utc::now().timestamp_millis();
		let mut total_orders = 0;
		let mut skipped_orders = 0;
		let mut skipped_events = 0;

		for snapshot in all_snapshot.snapshots {
			// 检查市场是否已过期
			if let Some(event_create) = data.events.get(&snapshot.symbol.event_id) {
				if let Some(end_date) = event_create.end_date
					&& end_date.timestamp_millis() < current_timestamp
				{
					// 市场已过期，跳过该市场的所有订单
					skipped_events += 1;
					skipped_orders += snapshot.orders.len();
					continue;
				}
			} else {
				// 市场信息不存在（快照中可能没有该市场信息，但订单存在）
				// 这种情况不应该发生，因为保存时应该同时保存市场信息
				// 但为了安全，还是跳过
				skipped_events += 1;
				skipped_orders += snapshot.orders.len();
				continue;
			}

			// 加载订单
			let symbol_key = snapshot.symbol.to_string();
			let mut symbol_orders = HashMap::new();
			for order in snapshot.orders {
				symbol_orders.insert(order.order_id.clone(), order);
				total_orders += 1;
			}
			if !symbol_orders.is_empty() {
				data.orders.insert(symbol_key, symbol_orders);
			}
		}

		let orders_count = data.orders.len();

		if skipped_orders > 0 {
			info!("Loaded {} events and {} orders from snapshot ({} symbols), skipped {} orders from {} expired events", events_count, total_orders, orders_count, skipped_orders, skipped_events);
		} else {
			info!("Loaded {} events and {} orders from snapshot ({} symbols)", events_count, total_orders, orders_count);
		}

		Ok(all_snapshot.last_message_id)
	}

	/// 启动定期保存任务
	pub fn start_periodic_save(&self) {
		let storage = Arc::new(self.clone());

		tokio::spawn(async move {
			let mut interval_timer = tokio::time::interval(tokio::time::Duration::from_secs(SNAPSHOT_INTERVAL_SECONDS));
			interval_timer.tick().await; // 第一次立即执行
			loop {
				interval_timer.tick().await;
				if let Err(e) = storage.save_snapshot().await {
					error!("Failed to save snapshot: {}", e);
				}
			}
		});
	}
}

// 用于定时存储
impl Clone for OrderStorage {
	fn clone(&self) -> Self {
		Self { data: Arc::clone(&self.data) }
	}
}

/// 删除 Redis stream 中小于等于指定消息 ID 的所有消息
async fn cleanup_old_messages(last_message_id: &str) -> anyhow::Result<()> {
	let mut conn = redis_pool::get_engine_output_mq_connection().await?;

	// 使用 XTRIM 删除小于等于 last_message_id 的消息
	// XTRIM 的 MINID 选项会删除 ID 小于等于指定 ID 的消息
	let options = redis::streams::StreamTrimOptions::minid(redis::streams::StreamTrimmingMode::Exact, last_message_id);
	let deleted_count: redis::RedisResult<usize> = conn.xtrim_options(STORE_STREAM, &options).await;

	match deleted_count {
		Ok(count) => {
			info!("Cleaned up {} messages up to {} in stream {}", count, last_message_id, STORE_STREAM);
		}
		Err(e) => {
			error!("Failed to cleanup old messages: {}", e);
			return Err(e.into());
		}
	}

	// 单独删除 last_message_id 这条消息本身
	let del_result: redis::RedisResult<i64> = conn.xdel(STORE_STREAM, &[last_message_id]).await;
	match del_result {
		Ok(del_count) => {
			if del_count > 0 {
				info!("Deleted last_message_id {} itself from stream {}", last_message_id, STORE_STREAM);
			}
		}
		Err(e) => {
			error!("Failed to delete last_message_id {}: {}", last_message_id, e);
			return Err(e.into());
		}
	}

	Ok(())
}
