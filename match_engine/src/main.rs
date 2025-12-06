pub mod config;
pub mod consts;
pub mod engine;
pub mod graceful;
pub mod helper;
pub mod init;
pub mod input;
pub mod load;
pub mod orderbook;
pub mod output;
pub mod types;

use {anyhow::Result, tracing::info};

#[tokio::main]
async fn main() -> Result<()> {
	init::init_all().await?;

	// 从快照加载数据并恢复市场
	info!("Loading orders and events from snapshot...");
	load::load().await?;

	crate::input::start_input_consumer().await?;

	info!("Match engine started");

	// 等待系统信号并执行优雅停机
	graceful::wait_for_shutdown().await?;

	Ok(())
}

//for update
