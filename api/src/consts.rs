/// Graceful shutdown wait times (in seconds)
pub const GRACEFUL_CONSUMER_WAIT_SECS: u64 = 10;
pub const GRACEFUL_TASKS_WAIT_SECS: u64 = 10;

/// Event 创建后多少秒才显示给前端 (留给其他服务处理时间)
pub const EVENT_DISPLAY_DELAY_SECS: i64 = 5;

/// Special log identifiers
pub const LOG_ORDER_NOT_PUSHED: &str = "[ORDER_NOT_PUSHED_TO_MATCHING]";
