mod asset_service;
mod config;
mod consts;
mod db;
mod graceful;
mod handlers;
mod init;
mod user_event;

use {
	crate::config::get_config,
	std::net::SocketAddr,
	tonic::transport::Server,
	tracing::{error, info},
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
	init::init_all().await?;

	let config = get_config();
	let addr: SocketAddr = config.grpc.get_addr().parse()?;
	info!("Asset gRPC server starting on {}", addr);

	let service = asset_service::create_asset_service();

	// 创建 shutdown signal
	let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();

	// 启动 gRPC 服务器
	let server_handle = tokio::spawn(async move {
		Server::builder()
			.add_service(service)
			.serve_with_shutdown(addr, async {
				shutdown_rx.await.ok();
			})
			.await
			.map_err(|e| anyhow::anyhow!("Server error: {}", e))
	});

	// 开始优雅停机流程
	graceful::graceful_shutdown(shutdown_tx).await?;

	// 等待 gRPC 服务器停止
	if let Err(e) = server_handle.await {
		error!("Server handle error: {}", e);
	}

	info!("Asset service stopped");
	Ok(())
}

// for sidekick
