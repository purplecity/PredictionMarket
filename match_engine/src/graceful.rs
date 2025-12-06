use {
	crate::{
		consts::ORDER_PROCESSING_TIMEOUT_SECS,
		engine::{get_manager, get_shutdown},
		input::{remove_event, stop_all_events},
	},
	std::time::Duration,
	tokio::signal,
	tracing::{error, info},
};
/// 等待系统信号并执行优雅停机
pub async fn wait_for_shutdown() -> anyhow::Result<()> {
	// 等待系统信号
	#[cfg(unix)]
	{
		use tokio::signal::unix::{SignalKind, signal};
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

	// 开始优雅停机流程
	graceful_shutdown().await?;

	Ok(())
}

/// 如果撮合停那么api也停 从而停止往event和order的input发消息 然后等待全部处理完在退出input的task
/// 优雅停机
pub async fn graceful_shutdown() -> anyhow::Result<()> {
	info!("Step 1: Stopping new orders...");
	// 1. 停止接受新订单
	stop_all_events(true).await;

	info!("Step 2: Waiting for pending orders to complete (max {}s)...", ORDER_PROCESSING_TIMEOUT_SECS);
	// 2. 等待正在处理的订单完成（给一个超时时间）
	tokio::time::sleep(Duration::from_secs(ORDER_PROCESSING_TIMEOUT_SECS)).await;

	info!("Step 3: Sending shutdown signal to all consumers...");
	// 3. 发送 shutdown signal 给所有 consumer
	let _ = get_shutdown().send(());

	info!("Step 4: Removing all events...");
	// 4. 移除所有市场（会发送 exit signal 给所有 MatchEngine）
	let event_ids: Vec<i64> = {
		let manager = get_manager().read().await;
		let event_managers = manager.event_managers.read().await;
		event_managers.keys().cloned().collect()
	};

	for event_id in event_ids {
		if let Err(e) = remove_event(event_id).await {
			error!("Failed to remove event {} during shutdown: {}", event_id, e);
		}
	}

	info!("Step 5: Waiting for all tasks to complete (max 10s)...");
	// 5. 等待所有任务完成
	tokio::time::sleep(Duration::from_secs(10)).await;

	info!("Graceful shutdown completed");
	Ok(())
}
