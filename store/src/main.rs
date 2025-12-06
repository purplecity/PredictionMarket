pub mod config;
pub mod consumer;
pub mod graceful;
pub mod init;
pub mod storage;

pub mod consts;
use {anyhow::Result, std::sync::Arc, tracing::info};

#[tokio::main]
async fn main() -> Result<()> {
	init::init_all().await?;

	let storage = Arc::new(storage::OrderStorage::new());

	// 首先加载快照
	info!("Loading snapshot...");
	let last_message_id = storage.load_snapshot().await?;

	// 启动定期保存任务
	storage.start_periodic_save();

	info!("Store service started");

	// 启动消费者任务
	tokio::spawn({
		let storage = storage.clone();
		async move {
			consumer::consumer_task(storage, last_message_id).await;
		}
	});

	// 等待系统信号并执行优雅停机
	graceful::shutdown_signal().await;

	Ok(())
}
