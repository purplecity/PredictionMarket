use {
	crate::{asset_service::set_stop_receiving_rpc, consts::PROCESSING_TIMEOUT_SECS},
	std::time::Duration,
	tokio::sync::oneshot,
	tracing::info,
};

/// 优雅停机
pub async fn graceful_shutdown(shutdown_tx: oneshot::Sender<()>) -> anyhow::Result<()> {
	// 等待系统信号
	#[cfg(unix)]
	{
		use {
			tokio::signal::{
				self,
				unix::{SignalKind, signal},
			},
			tracing::info,
		};
		let mut sigterm = signal(SignalKind::terminate())?;
		tokio::select! {
			_ = signal::ctrl_c() => {
				info!("Received SIGINT, starting graceful shutdown...");
			}
			_ = sigterm.recv() => {
				info!("Received SIGTERM, starting graceful shutdown...");
			}
		}
	}
	#[cfg(not(unix))]
	{
		signal::ctrl_c().await?;
		info!("Received SIGINT, starting graceful shutdown...");
	}

	info!("Step 1: Stopping RPC service from accepting new requests...");
	//  停止 gRPC 服务器（不再接受新连接）
	set_stop_receiving_rpc(true);
	let _ = shutdown_tx.send(());

	info!("Step 2: Waiting for pending RPC operations to complete (max {}s)...", PROCESSING_TIMEOUT_SECS);
	// 等待正在处理的 RPC 请求完成（给一个超时时间）
	tokio::time::sleep(Duration::from_secs(PROCESSING_TIMEOUT_SECS)).await;

	info!("Step 3: Closing database connection pool...");
	// 关闭数据库连接池（等待所有借出的连接归还）
	crate::db::close_db_pool().await;

	info!("Graceful shutdown completed");
	Ok(())
}
