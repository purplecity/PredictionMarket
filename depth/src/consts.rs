/// 定时推送深度快照的间隔（秒）
pub const DEPTH_PUSH_INTERVAL_SECS: u64 = 5;

/// 批量推送到 WebSocket Stream 的批次大小
pub const DEPTH_PUSH_BATCH_SIZE: usize = 50;

/// 优雅停机时等待任务完成的超时时间（秒）
pub const PROCESSING_TIMEOUT_SECS: u64 = 10;

/// 优雅停机时等待任务完成的时间（秒）
pub const GRACEFUL_TASKS_WAIT_SECS: u64 = 10;
