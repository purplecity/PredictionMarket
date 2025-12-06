mod config;
mod consts;
mod consumer;
mod graceful;
mod init;
mod pusher;
mod storage;

use {std::sync::Arc, storage::DepthStorage, tokio::sync::RwLock, tracing::info};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	// 初始化所有配置和连接
	init::init_all().await?;

	// 创建深度存储
	let storage = Arc::new(RwLock::new(DepthStorage::new()));

	// 启动消费任务
	let storage_clone = storage.clone();
	tokio::spawn(async move {
		consumer::start_consumer_task(storage_clone).await;
	});

	// 启动推送任务
	let storage_clone = storage.clone();
	tokio::spawn(async move {
		pusher::start_pusher_task(storage_clone).await;
	});

	info!("Depth service started");

	// 等待系统信号并执行优雅停机
	graceful::shutdown_signal().await;

	Ok(())
}

//for update 1
