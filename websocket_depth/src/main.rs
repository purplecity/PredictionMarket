mod config;
mod consts;
mod consumer;
mod errors;
mod graceful;
mod handler;
mod init;
mod storage;

use {
	axum::{Router, routing::get},
	handler::AppState,
	std::sync::Arc,
	storage::DepthStorage,
	tracing::info,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	// 初始化所有配置和连接
	init::init_all().await?;

	// 创建深度存储（DashMap内部已是并发安全的，无需RwLock）
	let storage = Arc::new(DepthStorage::new());

	// 从Redis缓存加载初始深度数据
	if let Err(e) = consumer::load_initial_depths(storage.clone()).await {
		tracing::warn!("Failed to load initial depths: {}", e);
	}
	// 加载完成后释放 cache_redis 连接池（后续不再使用）
	common::redis_pool::close_cache_redis_pool();
	info!("Cache Redis pool closed after initial depth loading");

	// 创建startup完成信号通道
	let (startup_tx, startup_rx) = tokio::sync::oneshot::channel();

	// 启动消费任务
	let storage_clone = storage.clone();
	tokio::spawn(async move {
		consumer::start_consumer_task(storage_clone, startup_tx).await;
	});

	// 等待20s启动窗口完成
	info!("Waiting for startup window to complete...");
	let _ = startup_rx.await;
	info!("Startup window complete");

	// 创建应用状态
	let app_state = AppState { storage };

	// 创建路由
	let app = Router::new().route("/depth", get(handler::ws_handler)).with_state(app_state);

	// 获取服务器地址
	let addr = config::get_config().server.get_addr();
	info!("WebSocket Depth service starting on {}", addr);

	// 启动HTTP服务器
	let listener = tokio::net::TcpListener::bind(&addr).await?;
	let server = axum::serve(listener, app);

	// 在后台运行服务器
	tokio::spawn(async move {
		if let Err(e) = server.await {
			tracing::error!("Server error: {}", e);
		}
	});

	info!("WebSocket Depth service started");

	// 等待系统信号并执行优雅停机
	graceful::shutdown_signal().await;

	Ok(())
}
