use tracing::info;

pub mod airdrop;
pub mod calculator;
pub mod config;
pub mod consts;
pub mod db;
pub mod graceful;
pub mod init;
pub mod scheduler;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	init::init_all().await?;

	info!("Statistics service starting");

	// 启动每日定时调度器（不等待完成）
	tokio::spawn(async {
		if let Err(e) = scheduler::start_scheduler().await {
			tracing::error!("Daily scheduler error: {}", e);
		}
	});

	// 启动赛季结束任务（不等待完成）
	tokio::spawn(async {
		if let Err(e) = scheduler::start_season_end_task().await {
			tracing::error!("Season end task error: {}", e);
		}
	});

	// 启动空投消费任务（从 Redis stream 消费新用户注册消息，批量空投 USDC）
	tokio::spawn(async {
		if let Err(e) = airdrop::start_airdrop_task().await {
			tracing::error!("Airdrop task error: {}", e);
		}
	});

	// 等待 shutdown 信号并执行优雅停机
	graceful::shutdown_signal().await;

	Ok(())
}

// for sidekick
