// Re-export using common graceful module (simple form without Result)
pub async fn shutdown_signal() {
	common::graceful::shutdown_signal_with_callback(crate::init::send_shutdown_signal, crate::consts::GRACEFUL_CONSUMER_WAIT_SECS, crate::consts::GRACEFUL_TASKS_WAIT_SECS).await;
}
