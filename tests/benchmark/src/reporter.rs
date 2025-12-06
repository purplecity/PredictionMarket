//! 压测结果报告
//!
//! 支持输出格式：text, json, html

use {
	colored::Colorize,
	serde::{Deserialize, Serialize},
	std::collections::HashMap,
	tabled::{Table, Tabled},
};

/// 单个场景的测试结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScenarioResult {
	pub total_orders: u64,
	pub elapsed_ms: u64,
	pub tps: f64,
	pub p50_ns: u64,
	pub p95_ns: u64,
	pub p99_ns: u64,
	pub p999_ns: u64,
	pub max_ns: u64,
	pub min_ns: u64,
	pub mean_ns: u64,
}

/// 用于表格显示的行
#[derive(Tabled)]
struct ResultRow {
	#[tabled(rename = "场景")]
	scenario: String,
	#[tabled(rename = "订单数")]
	orders: String,
	#[tabled(rename = "TPS")]
	tps: String,
	#[tabled(rename = "P50 (μs)")]
	p50: String,
	#[tabled(rename = "P95 (μs)")]
	p95: String,
	#[tabled(rename = "P99 (μs)")]
	p99: String,
	#[tabled(rename = "Max (μs)")]
	max: String,
}

/// 压测报告
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkReporter {
	pub results: HashMap<String, ScenarioResult>,
	pub timestamp: String,
}

impl BenchmarkReporter {
	pub fn new() -> Self {
		let timestamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
		Self { results: HashMap::new(), timestamp }
	}

	pub fn add_result(&mut self, scenario: &str, result: ScenarioResult) {
		self.results.insert(scenario.to_string(), result);
	}

	/// 打印文本格式的总结报告
	pub fn print_summary(&self) {
		println!();
		println!("{}", "═══════════════════════════════════════════════════════════".cyan());
		println!("{}", "                     压测结果汇总                          ".cyan().bold());
		println!("{}", "═══════════════════════════════════════════════════════════".cyan());
		println!();

		let rows: Vec<ResultRow> = self
			.results
			.iter()
			.map(|(scenario, result)| {
				ResultRow {
					scenario: scenario.clone(),
					orders: format!("{}", result.total_orders),
					tps: format!("{:.0}", result.tps),
					p50: format!("{:.2}", result.p50_ns as f64 / 1000.0),
					p95: format!("{:.2}", result.p95_ns as f64 / 1000.0),
					p99: format!("{:.2}", result.p99_ns as f64 / 1000.0),
					max: format!("{:.2}", result.max_ns as f64 / 1000.0),
				}
			})
			.collect();

		let table = Table::new(rows).to_string();
		println!("{}", table);

		println!();
		println!("{} {}", "测试时间:".dimmed(), self.timestamp);
		println!();
	}

	/// 输出 JSON 格式
	pub fn to_json(&self) -> anyhow::Result<String> {
		Ok(serde_json::to_string_pretty(self)?)
	}

	/// 输出 HTML 格式
	pub fn to_html(&self) -> String {
		let mut html = String::new();

		html.push_str(
			r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>Match Engine Benchmark Report</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, Oxygen, Ubuntu, Cantarell, sans-serif;
            max-width: 1200px;
            margin: 0 auto;
            padding: 20px;
            background-color: #f5f5f5;
        }
        h1 {
            color: #333;
            text-align: center;
        }
        .timestamp {
            text-align: center;
            color: #666;
            margin-bottom: 30px;
        }
        table {
            width: 100%;
            border-collapse: collapse;
            background-color: white;
            box-shadow: 0 1px 3px rgba(0,0,0,0.12);
            border-radius: 8px;
            overflow: hidden;
        }
        th, td {
            padding: 12px 15px;
            text-align: right;
            border-bottom: 1px solid #eee;
        }
        th {
            background-color: #4a90d9;
            color: white;
            font-weight: 600;
        }
        th:first-child, td:first-child {
            text-align: left;
        }
        tr:hover {
            background-color: #f8f9fa;
        }
        .tps {
            font-weight: bold;
            color: #28a745;
        }
        .scenario-name {
            font-weight: 500;
        }
    </style>
</head>
<body>
    <h1>Match Engine Benchmark Report</h1>
"#,
		);

		html.push_str(&format!(r#"    <p class="timestamp">测试时间: {}</p>"#, self.timestamp));

		html.push_str(
			r#"
    <table>
        <thead>
            <tr>
                <th>场景</th>
                <th>订单数</th>
                <th>TPS</th>
                <th>P50 (μs)</th>
                <th>P95 (μs)</th>
                <th>P99 (μs)</th>
                <th>P99.9 (μs)</th>
                <th>Max (μs)</th>
            </tr>
        </thead>
        <tbody>
"#,
		);

		for (scenario, result) in &self.results {
			html.push_str(&format!(
				r#"            <tr>
                <td class="scenario-name">{}</td>
                <td>{}</td>
                <td class="tps">{:.0}</td>
                <td>{:.2}</td>
                <td>{:.2}</td>
                <td>{:.2}</td>
                <td>{:.2}</td>
                <td>{:.2}</td>
            </tr>
"#,
				scenario,
				result.total_orders,
				result.tps,
				result.p50_ns as f64 / 1000.0,
				result.p95_ns as f64 / 1000.0,
				result.p99_ns as f64 / 1000.0,
				result.p999_ns as f64 / 1000.0,
				result.max_ns as f64 / 1000.0,
			));
		}

		html.push_str(
			r#"        </tbody>
    </table>
</body>
</html>"#,
		);

		html
	}
}

impl Default for BenchmarkReporter {
	fn default() -> Self {
		Self::new()
	}
}
