mod config;
mod consts;
mod consumer;
mod graceful;
mod handler;
mod init;
mod storage;

use {
	axum::{Router, routing::get},
	handler::AppState,
	std::sync::Arc,
	storage::UserStorage,
	tokio::sync::RwLock,
	tracing::info,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	// 初始化所有配置和连接
	init::init_all().await?;

	// 创建用户连接存储
	let storage = Arc::new(RwLock::new(UserStorage::new()));

	// 启动消费任务
	let storage_clone = storage.clone();
	tokio::spawn(async move {
		consumer::start_consumer_task(storage_clone).await;
	});

	// 创建应用状态
	let app_state = AppState { storage };

	// 创建路由
	let app = Router::new().route("/user", get(handler::ws_handler)).with_state(app_state);

	// 获取服务器地址
	let addr = config::get_config().server.get_addr();
	info!("WebSocket User service starting on {}", addr);

	// 启动HTTP服务器
	let listener = tokio::net::TcpListener::bind(&addr).await?;
	let server = axum::serve(listener, app);

	// 在后台运行服务器
	tokio::spawn(async move {
		if let Err(e) = server.await {
			tracing::error!("Server error: {}", e);
		}
	});

	info!("WebSocket User service started");

	// 等待系统信号并执行优雅停机
	graceful::shutdown_signal().await;

	Ok(())
}

//for update
