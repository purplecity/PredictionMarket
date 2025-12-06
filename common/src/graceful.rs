use {tokio::signal, tracing::info};

/// 等待系统信号并执行优雅停机（适用于所有服务）
///
/// # 参数
/// - `shutdown_callback`: 发送停机信号的回调函数
/// - `consumer_wait_secs`: 等待消费者完成的时间（秒）
/// - `tasks_wait_secs`: 等待任务完成的时间（秒）
///
/// # 示例
/// ```rust
/// common::graceful::shutdown_signal_with_callback(
///     crate::consumer::send_shutdown,
///     GRACEFUL_CONSUMER_WAIT_SECS,
///     GRACEFUL_TASKS_WAIT_SECS,
/// ).await;
/// ```
pub async fn shutdown_signal_with_callback<F>(shutdown_callback: F, consumer_wait_secs: u64, tasks_wait_secs: u64)
where
	F: FnOnce(),
{
	// 等待系统信号
	#[cfg(unix)]
	{
		use tokio::signal::unix::{SignalKind, signal};
		let mut sigterm = match signal(SignalKind::terminate()) {
			Ok(sigterm) => sigterm,
			Err(e) => {
				info!("Failed to create SIGTERM signal handler: {}", e);
				let _ = signal::ctrl_c().await;
				graceful_shutdown_simple(shutdown_callback, consumer_wait_secs, tasks_wait_secs).await;
				return;
			}
		};
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
		let _ = signal::ctrl_c().await;
		info!("Received SIGINT, starting graceful shutdown...");
	}

	// 开始优雅停机流程
	graceful_shutdown_simple(shutdown_callback, consumer_wait_secs, tasks_wait_secs).await;
}

/// 简单的优雅停机流程
async fn graceful_shutdown_simple<F>(shutdown_callback: F, consumer_wait_secs: u64, tasks_wait_secs: u64)
where
	F: FnOnce(),
{
	info!("Step 1: Sending shutdown signal to consumer...");
	shutdown_callback();

	info!("Step 2: Waiting for pending consumer operations to complete (max {}s)...", consumer_wait_secs);
	tokio::time::sleep(tokio::time::Duration::from_secs(consumer_wait_secs)).await;

	info!("Step 3: Waiting for all tasks to complete (max {}s)...", tasks_wait_secs);
	tokio::time::sleep(tokio::time::Duration::from_secs(tasks_wait_secs)).await;

	info!("Graceful shutdown completed");
}
