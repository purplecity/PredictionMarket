//! 压测场景实现
//!
//! 包含5个测试场景：
//! 1. pure_insert: 纯挂单
//! 2. full_match: 完全撮合
//! 3. partial_match: 部分撮合
//! 4. cross_match: 交叉撮合
//! 5. deep_orderbook: 深度压力测试

use {
	crate::reporter::ScenarioResult,
	colored::Colorize,
	common::engine_types::{Order, OrderSide, OrderType, PredictionSymbol},
	hdrhistogram::Histogram,
	indicatif::{ProgressBar, ProgressStyle},
	match_engine::engine::MatchEngine,
	std::time::{Duration, Instant},
	tokio::sync::broadcast,
};

/// 生成测试订单
fn generate_order(order_num: u64, user_id: i64, symbol: &PredictionSymbol, side: OrderSide, price: i32, quantity: u64) -> Order {
	Order {
		order_id: format!("bench_{}", order_num),
		symbol: symbol.clone(),
		side,
		order_type: OrderType::Limit,
		price,
		opposite_result_price: 10000 - price,
		quantity,
		remaining_quantity: quantity,
		filled_quantity: 0,
		status: common::engine_types::OrderStatus::New,
		user_id,
		privy_id: format!("privy_{}", user_id),
		outcome_name: "Yes".to_string(),
		order_num,
		timestamp: 0,
	}
}

/// 创建进度条
fn create_progress_bar(total: u64, msg: &str) -> ProgressBar {
	let pb = ProgressBar::new(total);
	pb.set_style(
		ProgressStyle::default_bar().template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len} ({per_sec}) {msg}").expect("valid progress template").progress_chars("#>-"),
	);
	pb.set_message(msg.to_string());
	pb
}

/// 计算统计数据
fn calculate_stats(histogram: &Histogram<u64>, total_orders: u64, elapsed: std::time::Duration) -> ScenarioResult {
	let tps = total_orders as f64 / elapsed.as_secs_f64();
	ScenarioResult {
		total_orders,
		elapsed_ms: elapsed.as_millis() as u64,
		tps,
		p50_ns: histogram.value_at_quantile(0.50),
		p95_ns: histogram.value_at_quantile(0.95),
		p99_ns: histogram.value_at_quantile(0.99),
		p999_ns: histogram.value_at_quantile(0.999),
		max_ns: histogram.max(),
		min_ns: histogram.min(),
		mean_ns: histogram.mean() as u64,
	}
}

/// 场景1: 纯挂单
/// 所有买单价格低于所有卖单价格，不会触发撮合
pub async fn run_pure_insert(order_count: u64, warmup_count: u64) -> anyhow::Result<ScenarioResult> {
	let event_id = 1i64;
	let market_id = 1i16;
	let token_0_id = "token_0".to_string();
	let token_1_id = "token_1".to_string();

	let (exit_tx, exit_rx) = broadcast::channel(1);
	let (mut engine, _sender) = MatchEngine::new(event_id, market_id, (token_0_id.clone(), token_1_id.clone()), 1000000, exit_rx);

	let symbol = PredictionSymbol::new(event_id, market_id, &token_0_id);

	// 预热
	println!("{}", format!("  预热中 ({} 订单)...", warmup_count).dimmed());
	for i in 0..warmup_count {
		let side = if i % 2 == 0 { OrderSide::Buy } else { OrderSide::Sell };
		// 买单价格 1000-2000，卖单价格 8000-9000，不会撮合
		let price = if side == OrderSide::Buy { 1000 + (i % 1000) as i32 } else { 8000 + (i % 1000) as i32 };
		let mut order = generate_order(i, (i % 1000 + 1) as i64, &symbol, side, price, 100);
		let _ = engine.submit_order(&mut order);
	}

	// 正式测试
	let pb = create_progress_bar(order_count, "纯挂单测试");
	let mut histogram = Histogram::<u64>::new(3).expect("create histogram");

	let start = Instant::now();
	for i in 0..order_count {
		let order_num = warmup_count + i;
		let side = if i % 2 == 0 { OrderSide::Buy } else { OrderSide::Sell };
		// 买单价格 1000-2000，卖单价格 8000-9000
		let price = if side == OrderSide::Buy { 1000 + (i % 1000) as i32 } else { 8000 + (i % 1000) as i32 };
		let mut order = generate_order(order_num, (i % 1000 + 1) as i64, &symbol, side, price, 100);

		let op_start = Instant::now();
		let _ = engine.submit_order(&mut order);
		let op_elapsed = op_start.elapsed();

		let _ = histogram.record(op_elapsed.as_nanos() as u64);
		pb.inc(1);
	}
	let elapsed = start.elapsed();
	pb.finish_with_message("完成");

	let _ = exit_tx.send(());

	let result = calculate_stats(&histogram, order_count, elapsed);
	print_result(&result);
	Ok(result)
}

/// 场景2: 完全撮合
/// 每个 taker 订单都能完全成交
pub async fn run_full_match(order_count: u64, warmup_count: u64) -> anyhow::Result<ScenarioResult> {
	let event_id = 1i64;
	let market_id = 1i16;
	let token_0_id = "token_0".to_string();
	let token_1_id = "token_1".to_string();

	let (exit_tx, exit_rx) = broadcast::channel(1);
	let (mut engine, _sender) = MatchEngine::new(event_id, market_id, (token_0_id.clone(), token_1_id.clone()), 1000000, exit_rx);

	let symbol = PredictionSymbol::new(event_id, market_id, &token_0_id);

	// 预热：填充大量卖单作为 maker
	println!("{}", format!("  预热中 ({} 卖单作为 maker)...", warmup_count).dimmed());
	for i in 0..warmup_count {
		// 卖单价格 5000，大量挂单等待被吃
		let mut order = generate_order(i, (i % 1000 + 1) as i64, &symbol, OrderSide::Sell, 5000, 100);
		let _ = engine.submit_order(&mut order);
	}

	// 正式测试：用买单去吃
	let pb = create_progress_bar(order_count, "完全撮合测试");
	let mut histogram = Histogram::<u64>::new(3).expect("create histogram");

	let start = Instant::now();
	for i in 0..order_count {
		let order_num = warmup_count + i;
		// 买单价格 6000，高于卖单价格 5000，会完全成交
		// 使用不同的 user_id 避免自成交
		let mut order = generate_order(order_num, (i % 1000 + 1001) as i64, &symbol, OrderSide::Buy, 6000, 100);

		let op_start = Instant::now();
		let _ = engine.submit_order(&mut order);
		let op_elapsed = op_start.elapsed();

		let _ = histogram.record(op_elapsed.as_nanos() as u64);
		pb.inc(1);
	}
	let elapsed = start.elapsed();
	pb.finish_with_message("完成");

	let _ = exit_tx.send(());

	let result = calculate_stats(&histogram, order_count, elapsed);
	print_result(&result);
	Ok(result)
}

/// 场景3: 部分撮合
/// 50% 订单能撮合，50% 挂单
pub async fn run_partial_match(order_count: u64, warmup_count: u64) -> anyhow::Result<ScenarioResult> {
	let event_id = 1i64;
	let market_id = 1i16;
	let token_0_id = "token_0".to_string();
	let token_1_id = "token_1".to_string();

	let (exit_tx, exit_rx) = broadcast::channel(1);
	let (mut engine, _sender) = MatchEngine::new(event_id, market_id, (token_0_id.clone(), token_1_id.clone()), 1000000, exit_rx);

	let symbol = PredictionSymbol::new(event_id, market_id, &token_0_id);

	// 预热：填充一些卖单
	println!("{}", format!("  预热中 ({} 订单)...", warmup_count).dimmed());
	for i in 0..warmup_count {
		// 卖单价格 5000
		let mut order = generate_order(i, (i % 1000 + 1) as i64, &symbol, OrderSide::Sell, 5000, 100);
		let _ = engine.submit_order(&mut order);
	}

	// 正式测试：交替发送可撮合和不可撮合的订单
	let pb = create_progress_bar(order_count, "部分撮合测试");
	let mut histogram = Histogram::<u64>::new(3).expect("create histogram");

	let start = Instant::now();
	for i in 0..order_count {
		let order_num = warmup_count + i;
		let (side, price) = if i % 2 == 0 {
			// 可以撮合的买单
			(OrderSide::Buy, 6000)
		} else {
			// 不能撮合的买单（价格太低）
			(OrderSide::Buy, 3000)
		};
		let mut order = generate_order(order_num, (i % 1000 + 1001) as i64, &symbol, side, price, 100);

		let op_start = Instant::now();
		let _ = engine.submit_order(&mut order);
		let op_elapsed = op_start.elapsed();

		let _ = histogram.record(op_elapsed.as_nanos() as u64);
		pb.inc(1);
	}
	let elapsed = start.elapsed();
	pb.finish_with_message("完成");

	let _ = exit_tx.send(());

	let result = calculate_stats(&histogram, order_count, elapsed);
	print_result(&result);
	Ok(result)
}

/// 场景4: 交叉撮合
/// 买 Yes (token_0) 匹配卖 Yes + 买 No (token_1)
pub async fn run_cross_match(order_count: u64, warmup_count: u64) -> anyhow::Result<ScenarioResult> {
	let event_id = 1i64;
	let market_id = 1i16;
	let token_0_id = "token_0".to_string();
	let token_1_id = "token_1".to_string();

	let (exit_tx, exit_rx) = broadcast::channel(1);
	let (mut engine, _sender) = MatchEngine::new(event_id, market_id, (token_0_id.clone(), token_1_id.clone()), 1000000, exit_rx);

	let symbol_0 = PredictionSymbol::new(event_id, market_id, &token_0_id);
	let symbol_1 = PredictionSymbol::new(event_id, market_id, &token_1_id);

	// 预热：在两个 token 上都填充订单
	println!("{}", format!("  预热中 ({} 订单)...", warmup_count).dimmed());
	for i in 0..warmup_count / 2 {
		// token_0 卖单，价格 5000
		let mut order = generate_order(i, (i % 1000 + 1) as i64, &symbol_0, OrderSide::Sell, 5000, 100);
		let _ = engine.submit_order(&mut order);

		// token_1 买单，价格 5000 (opposite_result_price = 5000，即对应 token_0 价格 5000)
		let mut order = generate_order(warmup_count / 2 + i, (i % 1000 + 1) as i64, &symbol_1, OrderSide::Buy, 5000, 100);
		let _ = engine.submit_order(&mut order);
	}

	// 正式测试：用 token_0 买单去吃（会触发交叉撮合）
	let pb = create_progress_bar(order_count, "交叉撮合测试");
	let mut histogram = Histogram::<u64>::new(3).expect("create histogram");

	let start = Instant::now();
	for i in 0..order_count {
		let order_num = warmup_count + i;
		// token_0 买单，价格 6000
		let mut order = generate_order(order_num, (i % 1000 + 1001) as i64, &symbol_0, OrderSide::Buy, 6000, 100);

		let op_start = Instant::now();
		let _ = engine.submit_order(&mut order);
		let op_elapsed = op_start.elapsed();

		let _ = histogram.record(op_elapsed.as_nanos() as u64);
		pb.inc(1);
	}
	let elapsed = start.elapsed();
	pb.finish_with_message("完成");

	let _ = exit_tx.send(());

	let result = calculate_stats(&histogram, order_count, elapsed);
	print_result(&result);
	Ok(result)
}

/// 场景5: 深度压力测试
/// 先填充大量深度，然后持续运行指定时间
pub async fn run_deep_orderbook(duration_secs: u64) -> anyhow::Result<ScenarioResult> {
	let event_id = 1i64;
	let market_id = 1i16;
	let token_0_id = "token_0".to_string();
	let token_1_id = "token_1".to_string();

	let (exit_tx, exit_rx) = broadcast::channel(1);
	// 容量设置：根据系统内存调整，10 分钟测试约需 500 万订单容量
	let (mut engine, _sender) = MatchEngine::new(event_id, market_id, (token_0_id.clone(), token_1_id.clone()), 5_000_000, exit_rx);

	let symbol = PredictionSymbol::new(event_id, market_id, &token_0_id);

	// 预热：填充深度
	// 创建 10000 个价格档位，每个档位 100 个订单
	let depth_levels = 10000u64;
	let orders_per_level = 100u64;
	let total_depth_orders = depth_levels * orders_per_level;
	println!("{}", format!("  填充深度中 ({} 档位, {} 订单/档位, 共 {} 订单)...", depth_levels, orders_per_level, total_depth_orders).dimmed());

	let depth_pb = create_progress_bar(total_depth_orders, "填充深度");
	let mut order_num = 0u64;
	for level in 0..depth_levels {
		let price = (100 + level) as i32; // 价格从 100 到 10099
		for _j in 0..orders_per_level {
			// 每个订单使用唯一的 user_id
			let mut order = generate_order(order_num, order_num as i64 + 1, &symbol, OrderSide::Sell, price, 1000);
			let _ = engine.submit_order(&mut order);
			order_num += 1;
			depth_pb.inc(1);
		}
	}
	depth_pb.finish_with_message("深度填充完成");

	println!("{}", format!("  开始压测 (持续 {} 秒)...", duration_secs).dimmed());

	// 正式测试：持续运行指定时间
	let pb = ProgressBar::new(duration_secs);
	pb.set_style(ProgressStyle::default_bar().template("{spinner:.green} [{elapsed_precise}] [{bar:40.cyan/blue}] {pos}/{len}s {msg}").expect("valid progress template").progress_chars("#>-"));
	pb.set_message("深度压力测试");

	let mut histogram = Histogram::<u64>::new(3).expect("create histogram");
	let duration = Duration::from_secs(duration_secs);
	let start = Instant::now();
	let mut total_orders = 0u64;
	let mut last_second = 0u64;

	while start.elapsed() < duration {
		// 每个订单使用唯一的 user_id
		let user_id = order_num + total_orders + 1;
		// 交替买卖，价格重叠确保能撮合
		let side = if total_orders.is_multiple_of(2) { OrderSide::Buy } else { OrderSide::Sell };
		let price = if side == OrderSide::Buy { 5500 + (total_orders % 500) as i32 } else { 4500 + (total_orders % 500) as i32 };
		let mut order = generate_order(order_num + total_orders, user_id as i64, &symbol, side, price, 1000);

		let op_start = Instant::now();
		let _ = engine.submit_order(&mut order);
		let op_elapsed = op_start.elapsed();

		let _ = histogram.record(op_elapsed.as_nanos() as u64);
		total_orders += 1;

		// 更新进度条（每秒更新一次）
		let current_second = start.elapsed().as_secs();
		if current_second > last_second {
			pb.set_position(current_second);
			last_second = current_second;
		}
	}
	let elapsed = start.elapsed();
	pb.finish_with_message("完成");

	// 打印 orderbook 状态，确认订单数据在内存中
	let token_0_orders = engine.token_0_orders.len();
	let token_1_orders = engine.token_1_orders.len();
	println!();
	println!("  {}: token_0 订单数={}, token_1 订单数={}", "Orderbook 状态".cyan(), token_0_orders, token_1_orders);
	println!("  {}: {}", "总订单数".cyan(), token_0_orders + token_1_orders);

	let _ = exit_tx.send(());

	let result = calculate_stats(&histogram, total_orders, elapsed);
	print_result(&result);
	Ok(result)
}

/// 打印单个场景的结果
fn print_result(result: &ScenarioResult) {
	println!();
	println!("  {}: {:.2} ops/s", "TPS".green().bold(), result.tps);
	println!("  {}: {:?}", "总耗时".green(), std::time::Duration::from_millis(result.elapsed_ms));
	println!(
		"  {}: P50={:.2}μs P95={:.2}μs P99={:.2}μs Max={:.2}μs",
		"延迟".green(),
		result.p50_ns as f64 / 1000.0,
		result.p95_ns as f64 / 1000.0,
		result.p99_ns as f64 / 1000.0,
		result.max_ns as f64 / 1000.0
	);
}

/// 场景6: 多市场并发测试
/// 测试多个市场同时运行时的总 TPS
/// 使用 std::thread 真正并行执行，每个市场运行在独立线程
/// 持续运行指定时间
pub async fn run_multi_market(duration_secs: u64, warmup_count: u64, market_count: usize) -> anyhow::Result<Vec<ScenarioResult>> {
	use std::{
		sync::{
			Arc,
			atomic::{AtomicU64, Ordering},
		},
		thread,
	};

	// 获取可用的 CPU 核心
	let core_ids = core_affinity::get_core_ids().unwrap_or_default();
	let num_cores = core_ids.len();
	println!("{}", format!("  可用 CPU 核心: {} 个", num_cores).dimmed());
	println!("{}", format!("  启动 {} 个市场并发测试 (std::thread + CPU 绑定, 持续 {} 秒)...", market_count, duration_secs).dimmed());

	let total_orders = Arc::new(AtomicU64::new(0));
	let mut handles = Vec::new();

	let start = Instant::now();

	for market_idx in 0..market_count {
		let total_orders = Arc::clone(&total_orders);
		// 绑定到不同的 CPU 核心（循环使用）
		let core_id = if !core_ids.is_empty() { Some(core_ids[market_idx % num_cores]) } else { None };

		// 使用 std::thread 实现真正的并行
		let handle = thread::spawn(move || {
			// 绑定 CPU 核心
			if let Some(core) = core_id {
				core_affinity::set_for_current(core);
			}

			let event_id = 1i64;
			let market_id = market_idx as i16;
			let token_0_id = format!("token_0_{}", market_idx);
			let token_1_id = format!("token_1_{}", market_idx);

			let (exit_tx, exit_rx) = broadcast::channel(1);
			// 容量设置：根据系统内存调整，每个市场 500 万订单容量
			let (mut engine, _sender) = MatchEngine::new(event_id, market_id, (token_0_id.clone(), token_1_id.clone()), 5_000_000, exit_rx);

			let symbol = PredictionSymbol::new(event_id, market_id, &token_0_id);

			// 预热：填充买卖双方订单，每个订单使用唯一 user_id
			for i in 0..warmup_count {
				let side = if i % 2 == 0 { OrderSide::Buy } else { OrderSide::Sell };
				// 买单 4000-5000，卖单 5000-6000，中间价格可以撮合
				let price = if side == OrderSide::Buy { 4000 + (i % 1000) as i32 } else { 5000 + (i % 1000) as i32 };
				let mut order = generate_order(i, i as i64 + 1, &symbol, side, price, 1000);
				let _ = engine.submit_order(&mut order);
			}

			// 正式测试：持续运行指定时间，混合买卖订单
			let mut histogram = Histogram::<u64>::new(3).expect("create histogram");
			let duration = Duration::from_secs(duration_secs);
			let market_start = Instant::now();
			let mut order_count = 0u64;

			while market_start.elapsed() < duration {
				let order_num = warmup_count + order_count;
				// 每个订单使用唯一的 user_id（基于 market_idx 偏移避免跨市场冲突）
				let user_id = (market_idx as u64 * 1_000_000_000) + order_num + 1;
				// 交替买卖，价格重叠确保能撮合
				let side = if order_count.is_multiple_of(2) { OrderSide::Buy } else { OrderSide::Sell };
				let price = if side == OrderSide::Buy { 5500 + (order_count % 500) as i32 } else { 4500 + (order_count % 500) as i32 };
				let mut order = generate_order(order_num, user_id as i64, &symbol, side, price, 1000);

				let op_start = Instant::now();
				let _ = engine.submit_order(&mut order);
				let op_elapsed = op_start.elapsed();

				let _ = histogram.record(op_elapsed.as_nanos() as u64);
				order_count += 1;
			}

			let elapsed = market_start.elapsed();
			total_orders.fetch_add(order_count, Ordering::Relaxed);

			// 统计 orderbook 中剩余订单数
			let remaining_orders = engine.token_0_orders.len() + engine.token_1_orders.len();

			let _ = exit_tx.send(());

			(calculate_stats(&histogram, order_count, elapsed), remaining_orders)
		});

		handles.push(handle);
	}

	// 等待所有线程完成
	let mut results = Vec::new();
	let mut total_remaining_orders = 0usize;
	for handle in handles {
		let (result, remaining) = handle.join().expect("Thread panicked");
		results.push(result);
		total_remaining_orders += remaining;
	}

	let total_elapsed = start.elapsed();
	let total = total_orders.load(Ordering::Relaxed);
	let total_tps = total as f64 / total_elapsed.as_secs_f64();

	println!();
	println!("  {}: {} 个市场", "市场数".green().bold(), market_count);
	println!("  {}: {} 订单", "总订单数".green().bold(), total);
	println!("  {}: {:?}", "总耗时".green(), total_elapsed);
	println!("  {}: {:.2} ops/s", "总 TPS".green().bold(), total_tps);
	println!("  {}: {} (确认订单数据在内存中)", "剩余订单数".green().bold(), total_remaining_orders);
	println!();

	// 打印各市场统计
	let avg_tps: f64 = results.iter().map(|r| r.tps).sum::<f64>() / results.len() as f64;
	let avg_p50: f64 = results.iter().map(|r| r.p50_ns as f64).sum::<f64>() / results.len() as f64;
	let avg_p99: f64 = results.iter().map(|r| r.p99_ns as f64).sum::<f64>() / results.len() as f64;
	let max_p99: u64 = results.iter().map(|r| r.p99_ns).max().unwrap_or(0);

	println!("  {}: {:.2} ops/s", "平均单市场 TPS".cyan(), avg_tps);
	println!("  {}: P50={:.2}μs P99={:.2}μs MaxP99={:.2}μs", "平均延迟".cyan(), avg_p50 / 1000.0, avg_p99 / 1000.0, max_p99 as f64 / 1000.0);

	Ok(results)
}
