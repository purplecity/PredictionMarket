mod config;
mod consts;
mod consumer;
mod db;
mod graceful;
mod init;
mod statistic;
mod types;

use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	init::init_all().await?;

	// 启动消费者任务
	tokio::spawn(async {
		consumer::consumer_task().await;
	});

	// 启动统计任务
	tokio::spawn(async {
		statistic::start_statistic_task().await;
	});

	info!("Event service started");

	// 等待系统信号并执行优雅停机
	graceful::shutdown_signal().await;

	Ok(())
}

// for sidekick
