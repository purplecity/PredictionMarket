use {
	crate::{
		config::get_config,
		engine::{EventManager, MatchEngine, get_manager},
	},
	chrono::Utc,
	common::{
		consts::*,
		engine_types::{Order, OrderStatus, PredictionSymbol},
		redis_pool,
		store_types::AllOrdersSnapshot,
	},
	redis::AsyncCommands,
	serde_json,
	std::{collections::HashMap, fs, path::Path},
	tokio::{sync::broadcast, task::JoinSet, time::Duration},
	tracing::{error, info, warn},
};

/// 等待 STORE_STREAM 消息处理完成（消息长度为0）
async fn wait_for_store_stream_empty() -> anyhow::Result<()> {
	let pool = redis_pool::get_engine_output_mq_pool()?;

	info!("Waiting for STORE_STREAM to be empty before loading snapshot...");

	loop {
		let mut conn = pool.get().await?;

		// 使用 XLEN 命令获取 stream 长度
		let stream_len: usize = conn.xlen(STORE_STREAM).await?;

		if stream_len == 0 {
			info!("STORE_STREAM is empty, proceeding to load snapshot");
			break;
		}

		warn!("STORE_STREAM still has {} messages, waiting 5 seconds...", stream_len);
		tokio::time::sleep(Duration::from_secs(5)).await;
	}

	// STORE_STREAM 为空后，再等待一个快照间隔时间，确保 store 服务完成快照保存
	info!("Waiting {} seconds for store service to complete snapshot...", SNAPSHOT_INTERVAL_SECONDS);
	tokio::time::sleep(Duration::from_secs(SNAPSHOT_INTERVAL_SECONDS)).await;

	Ok(())
}

/// 从快照文件加载数据并恢复市场
pub async fn load() -> anyhow::Result<()> {
	// 首先等待 STORE_STREAM 消息处理完成
	wait_for_store_stream_empty().await?;

	let file_path = Path::new(STORE_DATA_DIR).join(ORDERS_SNAPSHOT_FILE);

	if !file_path.exists() {
		info!("Snapshot file not found: {}, starting with empty events", file_path.display());
		return Ok(());
	}

	let json = fs::read_to_string(&file_path)?;
	let all_snapshot: AllOrdersSnapshot = serde_json::from_str(&json)?;

	let current_timestamp = Utc::now().timestamp_millis();

	// 1. 过滤出有效的市场（结束时间大于当前时间）
	let valid_events: HashMap<i64, common::store_types::SnapshotEvent> =
		all_snapshot.events.into_iter().filter(|(_, event_msg)| event_msg.end_date.map(|d| d.timestamp_millis() > current_timestamp).unwrap_or(true)).collect();

	if valid_events.is_empty() {
		info!("No valid events found in snapshot");
		return Ok(());
	}

	info!("Found {} valid events in snapshot", valid_events.len());

	// 2. 遍历 snapshots，过滤掉市场不在有效列表中的
	// 然后按市场分组，为每个 symbol 创建订单列表映射
	let mut event_symbol_orders_map: HashMap<i64, HashMap<String, Vec<Order>>> = HashMap::new();

	for snapshot in all_snapshot.snapshots {
		let event_id = snapshot.symbol.event_id;

		// 检查市场是否在有效列表中
		if !valid_events.contains_key(&event_id) {
			continue;
		}

		let symbol_key = snapshot.symbol.to_string();

		// 只保留活跃订单（New 和 PartiallyFilled），并按价格和 order_num 排序
		let mut active_orders: Vec<Order> = snapshot.orders.into_iter().filter(|o| matches!(o.status, OrderStatus::New | OrderStatus::PartiallyFilled)).collect();

		// 按照价格从小到大排序，如果价格相同则按照 order_num 从小到大排序
		active_orders.sort_by(|a, b| {
			let price_a = a.price;
			let price_b = b.price;
			match price_a.cmp(&price_b) {
				std::cmp::Ordering::Equal => a.order_num.cmp(&b.order_num),
				other => other,
			}
		});

		if !active_orders.is_empty() {
			event_symbol_orders_map.entry(event_id).or_default().insert(symbol_key, active_orders);
		}
	}

	// 3. 对于每个有效的市场，调用 init_add_event
	for (event_id, event_msg) in &valid_events {
		// 获取该市场的 symbol_orders_map（如果存在）
		// 有可能只是没有订单那默认的空map
		let symbol_orders_map = event_symbol_orders_map.get(event_id).cloned().unwrap_or_default();

		init_add_event(event_msg.clone(), symbol_orders_map).await?;

		let total_orders: usize = event_symbol_orders_map.get(event_id).map(|m| m.values().map(|v| v.len()).sum()).unwrap_or(0);

		if total_orders > 0 {
			info!("Restored event {} with {} orders", event_id, total_orders);
		} else {
			info!("Restored event {} with no orders", event_id);
		}
	}

	info!("Load completed: {} events restored", valid_events.len());
	Ok(())
}

/// 初始化添加市场（用于从快照加载）
/// symbol_orders_map: key 是单个市场所有的 symbol（字符串），value 是对应的订单列表
pub async fn init_add_event(snapshot_event: common::store_types::SnapshotEvent, symbol_orders_map: HashMap<String, Vec<Order>>) -> anyhow::Result<()> {
	let event_id = snapshot_event.event_id;

	// 检查市场是否已存在
	{
		let manager = get_manager().read().await;
		let event_managers = manager.event_managers.read().await;
		if event_managers.contains_key(&event_id) {
			info!("Event {} already exists, skipping", event_id);
			return Ok(());
		}
	}

	let config = get_config();
	let max_order_count = config.engine.engine_max_order_count;

	// 创建退出信号的broadcast channel
	let (exit_sender, _) = broadcast::channel(1);

	// 创建JoinSet来管理所有MatchEngine任务
	let mut join_set = JoinSet::new();
	let mut order_senders = HashMap::new();

	// 为每个market创建一个MatchEngine（处理token_0和token_1两个结果）
	for (market_id_str, snapshot_market) in &snapshot_event.markets {
		// 解析 market_id 从 String 到 i16
		let market_id: i16 = market_id_str.parse().map_err(|e| anyhow::anyhow!("Invalid market_id '{}': {}", market_id_str, e))?;

		// 创建exit_receiver
		let exit_receiver = exit_sender.subscribe();

		// 从market中获取token_ids
		let token_ids = if snapshot_market.token_ids.len() >= 2 {
			(snapshot_market.token_ids[0].clone(), snapshot_market.token_ids[1].clone())
		} else {
			return Err(anyhow::anyhow!("Option {} must have at least 2 token_ids", market_id));
		};

		// 从 map 中获取token_0和token_1的订单列表
		let token_0_symbol = PredictionSymbol::new(event_id, market_id, &token_ids.0);
		let token_1_symbol = PredictionSymbol::new(event_id, market_id, &token_ids.1);
		let token_0_symbol_key = token_0_symbol.to_string();
		let token_1_symbol_key = token_1_symbol.to_string();

		// 构建symbol_orders_map用于new_with_orders
		let mut market_orders_map = HashMap::new();
		if let Some(token_0_orders) = symbol_orders_map.get(&token_0_symbol_key) {
			market_orders_map.insert(token_0_symbol_key.clone(), token_0_orders.clone());
		}
		if let Some(token_1_orders) = symbol_orders_map.get(&token_1_symbol_key) {
			market_orders_map.insert(token_1_symbol_key.clone(), token_1_orders.clone());
		}

		let initial_update_id = snapshot_market.update_id;

		// 创建 MatchEngine
		let (mut engine, order_sender) = if market_orders_map.is_empty() {
			MatchEngine::new(event_id, market_id, token_ids, max_order_count, exit_receiver)
		} else {
			MatchEngine::new_with_orders(event_id, market_id, token_ids, market_orders_map, max_order_count, exit_receiver)?
		};

		// 设置 Engine 的 update_id 为快照值 + 1
		engine.update_id = initial_update_id;

		// 启动MatchEngine的run任务并加入JoinSet
		join_set.spawn(async move {
			engine.run().await;
		});

		// 使用event_id和market_id的组合作为key（不区分token_0/token_1，因为一个MatchEngine处理两个结果）
		let market_key = format!("{}{}{}", event_id, common::consts::SYMBOL_SEPARATOR, market_id);
		order_senders.insert(market_key, order_sender);
	}

	// 创建监控任务，等待所有MatchEngine任务完成
	let event_id_for_monitor = event_id;
	tokio::spawn(async move {
		// 等待所有任务完成
		while let Some(result) = join_set.join_next().await {
			match result {
				Ok(_) => {
					// 任务正常完成
				}
				Err(e) => {
					error!("MatchEngine task panicked for event {}: {}", event_id_for_monitor, e);
				}
			}
		}

		info!("All MatchEngine tasks for event {} have completed", event_id_for_monitor);
	});

	// 至此各个撮合引擎以及监控task已经运行

	// 创建EventManager
	let event_manager = EventManager { event_id, order_senders, exit_signal: exit_sender, end_timestamp: snapshot_event.end_date };

	// 将EventManager存储到全局Manager中
	let manager = get_manager().write().await;
	let mut event_managers = manager.event_managers.write().await;
	event_managers.insert(event_id, event_manager);

	// 注意：从快照加载时不需要通知 store，因为 store 已经有这些数据了

	info!("Event {} initialized from snapshot with {} markets", event_id, snapshot_event.markets.len());
	Ok(())
}
