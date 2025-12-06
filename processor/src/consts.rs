/// 消费者组名称
pub const PROCESSOR_CONSUMER_GROUP: &str = "processor_group";

/// 消费者名称前缀
pub const PROCESSOR_CONSUMER_NAME_PREFIX: &str = "processor_consumer";

/// 单次批处理的交易数量
pub const TRADE_BATCH_SIZE: usize = 4;

/// Graceful shutdown 等待消费者完成的时间（秒）
pub const GRACEFUL_CONSUMER_WAIT_SECS: u64 = 10;

/// 优雅停机时等待任务完成的时间（秒）
pub const GRACEFUL_TASKS_WAIT_SECS: u64 = 10;

/// WebSocket 事件 pipeline 批量推送大小
pub const WEBSOCKET_EVENT_BATCH_SIZE: usize = 50;
