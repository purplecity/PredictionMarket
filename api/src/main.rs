use {std::net::SocketAddr, tracing::info};

pub mod api_error;
pub mod api_types;
pub mod cache;
pub mod config;
pub mod consts;
pub mod consumer;
pub mod db;
pub mod graceful;
pub mod handlers;
pub mod init;
pub mod internal_service;
pub mod load;
pub mod rpc_chat_service;
pub mod rpc_client;
pub mod server;
pub mod singleflight;
pub mod user_handlers;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	init::init_all().await?;

	// 启动 consumer task 消费市场事件
	tokio::spawn(consumer::consumer_task());
	info!("Event consumer task started");

	let config = config::get_config();
	let addr = config.server.get_addr();
	let listener = tokio::net::TcpListener::bind(&addr).await?;
	info!("🚀 API Server is running at {}", listener.local_addr()?);

	let app = server::app()?;

	// 使用 axum 的 with_graceful_shutdown 实现优雅停机
	axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>()).with_graceful_shutdown(graceful::shutdown_signal()).await?;

	info!("API service stopped");
	Ok(())
}

// for sidekick
