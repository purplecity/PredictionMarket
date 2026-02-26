use tracing::info;

pub mod config;
pub mod consts;
pub mod consumer;
pub mod graceful;
pub mod init;
pub mod rpc_client;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	init::init_all().await?;

	info!("🚀 Processor service starting");

	// 启动消费者（不等待它们完成）
	if let Err(e) = consumer::start_consumers().await {
		tracing::error!("Failed to start consumers: {}", e);
		return Err(e);
	}

	// 等待 shutdown 信号并执行优雅停机
	graceful::shutdown_signal().await;

	Ok(())
}

//for update 1
// for sidekick
