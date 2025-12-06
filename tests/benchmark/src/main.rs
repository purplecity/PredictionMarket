//! Match Engine Benchmark
//!
//! 压测 match_engine 的纯撮合性能

use {
	benchmark::{reporter::BenchmarkReporter, scenarios::*},
	clap::{Parser, ValueEnum},
	colored::Colorize,
};

// 使用 mimalloc 替代系统分配器，提升多线程性能
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[derive(Debug, Clone, ValueEnum)]
enum Scenario {
	/// 纯挂单：订单不撮合，测试 orderbook 插入性能
	PureInsert,
	/// 完全撮合：每个订单都能完全成交
	FullMatch,
	/// 部分撮合：混合场景，部分订单成交
	PartialMatch,
	/// 交叉撮合：Yes/No 双向撮合
	CrossMatch,
	/// 深度压力：orderbook 深度达到 10000+ 档位
	DeepOrderbook,
	/// 多市场并发：测试多核并行性能
	MultiMarket,
	/// 运行所有场景
	All,
}

#[derive(Parser, Debug)]
#[command(name = "bench")]
#[command(about = "Match Engine Benchmark Tool", long_about = None)]
struct Args {
	/// 测试场景
	#[arg(short, long, value_enum, default_value = "all")]
	scenario: Scenario,

	/// 订单数量（用于非持续时间测试的场景）
	#[arg(short, long, default_value = "100000")]
	orders: u64,

	/// 预热订单数量
	#[arg(short, long, default_value = "10000")]
	warmup: u64,

	/// 输出报告格式 (text, json, html)
	#[arg(short, long, default_value = "text")]
	report: String,

	/// 结果输出目录
	#[arg(long, default_value = "../results")]
	output_dir: String,

	/// 多市场测试时的市场数量
	#[arg(short, long, default_value = "4")]
	markets: usize,

	/// 持续时间（秒），用于 deep_orderbook 和 multi_market 场景
	#[arg(short, long, default_value = "600")]
	duration: u64,
}

fn print_banner() {
	println!();
	println!("{}", "╔═══════════════════════════════════════════════════════════╗".cyan());
	println!("{}", "║           Match Engine Benchmark Suite                    ║".cyan());
	println!("{}", "║           测试纯撮合性能上限                                ║".cyan());
	println!("{}", "╚═══════════════════════════════════════════════════════════╝".cyan());
	println!();
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
	let args = Args::parse();

	print_banner();

	println!("{} 订单数量: {}", "配置:".green().bold(), args.orders);
	println!("{} 预热数量: {}", "     ".green(), args.warmup);
	println!("{} 持续时间: {} 秒 (用于 deep_orderbook/multi_market)", "     ".green(), args.duration);
	println!("{} 报告格式: {}", "     ".green(), args.report);
	println!();

	let mut reporter = BenchmarkReporter::new();

	match args.scenario {
		Scenario::PureInsert => {
			println!("{}", "▶ 运行场景: 纯挂单 (pure_insert)".yellow().bold());
			let result = run_pure_insert(args.orders, args.warmup).await?;
			reporter.add_result("pure_insert", result);
		}
		Scenario::FullMatch => {
			println!("{}", "▶ 运行场景: 完全撮合 (full_match)".yellow().bold());
			let result = run_full_match(args.orders, args.warmup).await?;
			reporter.add_result("full_match", result);
		}
		Scenario::PartialMatch => {
			println!("{}", "▶ 运行场景: 部分撮合 (partial_match)".yellow().bold());
			let result = run_partial_match(args.orders, args.warmup).await?;
			reporter.add_result("partial_match", result);
		}
		Scenario::CrossMatch => {
			println!("{}", "▶ 运行场景: 交叉撮合 (cross_match)".yellow().bold());
			let result = run_cross_match(args.orders, args.warmup).await?;
			reporter.add_result("cross_match", result);
		}
		Scenario::DeepOrderbook => {
			println!("{}", format!("▶ 运行场景: 深度压力 (deep_orderbook, 持续 {} 秒)", args.duration).yellow().bold());
			let result = run_deep_orderbook(args.duration).await?;
			reporter.add_result("deep_orderbook", result);
		}
		Scenario::MultiMarket => {
			println!("{}", format!("▶ 运行场景: 多市场并发 ({} 个市场, 持续 {} 秒)", args.markets, args.duration).yellow().bold());
			let results = run_multi_market(args.duration, args.warmup, args.markets).await?;
			for (i, result) in results.iter().enumerate() {
				reporter.add_result(&format!("multi_market_{}", i), result.clone());
			}
		}
		Scenario::All => {
			println!("{}", "▶ 运行所有场景".yellow().bold());
			println!();

			// 场景1: 纯挂单
			println!("{}", "━━━ 1/5 纯挂单 (pure_insert) ━━━".cyan());
			let result = run_pure_insert(args.orders, args.warmup).await?;
			reporter.add_result("pure_insert", result);
			println!();

			// 场景2: 完全撮合
			println!("{}", "━━━ 2/5 完全撮合 (full_match) ━━━".cyan());
			let result = run_full_match(args.orders, args.warmup).await?;
			reporter.add_result("full_match", result);
			println!();

			// 场景3: 部分撮合
			println!("{}", "━━━ 3/5 部分撮合 (partial_match) ━━━".cyan());
			let result = run_partial_match(args.orders, args.warmup).await?;
			reporter.add_result("partial_match", result);
			println!();

			// 场景4: 交叉撮合
			println!("{}", "━━━ 4/5 交叉撮合 (cross_match) ━━━".cyan());
			let result = run_cross_match(args.orders, args.warmup).await?;
			reporter.add_result("cross_match", result);
			println!();

			// 场景5: 深度压力（注意：All 场景下使用订单数量而非持续时间，避免太长）
			println!("{}", "━━━ 5/5 深度压力 (deep_orderbook) ━━━".cyan());
			println!("{}", "  注意: All 场景使用 60 秒测试，如需 10 分钟请单独运行 -s deep-orderbook".dimmed());
			let result = run_deep_orderbook(60).await?;
			reporter.add_result("deep_orderbook", result);
			println!();
		}
	}

	// 输出报告
	match args.report.as_str() {
		"json" => {
			let json = reporter.to_json()?;
			let path = format!("{}/benchmark_result.json", args.output_dir);
			std::fs::create_dir_all(&args.output_dir)?;
			std::fs::write(&path, json)?;
			println!("{} {}", "JSON 报告已保存到:".green(), path);
		}
		"html" => {
			let html = reporter.to_html();
			let path = format!("{}/benchmark_result.html", args.output_dir);
			std::fs::create_dir_all(&args.output_dir)?;
			std::fs::write(&path, html)?;
			println!("{} {}", "HTML 报告已保存到:".green(), path);
		}
		_ => {
			reporter.print_summary();
		}
	}

	println!();
	println!("{}", "✓ 压测完成".green().bold());

	Ok(())
}
