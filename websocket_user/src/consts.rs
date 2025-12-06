/// 心跳超时时间（秒）- 如果客户端在此时间内没有发送心跳，则断开连接
pub const HEARTBEAT_TIMEOUT_SECS: u64 = 30;

/// 心跳检查间隔（秒）- 服务端检查心跳超时的频率
pub const HEARTBEAT_CHECK_INTERVAL_SECS: u64 = 5;

/// 优雅停机时等待消费者完成的时间（秒）
pub const GRACEFUL_CONSUMER_WAIT_SECS: u64 = 10;
/// 优雅停机时等待任务完成的时间（秒）
pub const GRACEFUL_TASKS_WAIT_SECS: u64 = 10;

/// 每次从 Redis Stream 读取的消息批次大小
pub const USER_EVENT_BATCH_SIZE: usize = 100;
