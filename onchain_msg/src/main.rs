use tracing::info;

pub mod cache;
pub mod config;
pub mod consts;
pub mod db;
pub mod event_consumer;
pub mod graceful;
pub mod init;
pub mod onchain_action_consumer;
pub mod rpc_client;
pub mod trade_response_consumer;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	init::init_all().await?;

	info!("🚀 OnchainMsg service starting");

	// 启动所有消费者（不等待它们完成）
	if let Err(e) = trade_response_consumer::start_consumers().await {
		tracing::error!("Failed to start trade response consumers: {}", e);
		return Err(e);
	}

	if let Err(e) = onchain_action_consumer::start_consumers().await {
		tracing::error!("Failed to start onchain action consumers: {}", e);
		return Err(e);
	}

	if let Err(e) = event_consumer::start_consumer().await {
		tracing::error!("Failed to start event consumer: {}", e);
		return Err(e);
	}

	// 等待 shutdown 信号并执行优雅停机
	graceful::shutdown_signal().await;

	Ok(())
}
