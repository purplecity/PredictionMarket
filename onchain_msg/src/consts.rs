/// Graceful shutdown 等待消费者完成的时间（秒）
pub const GRACEFUL_CONSUMER_WAIT_SECS: u64 = 10;

/// 优雅停机时等待任务完成的时间（秒）
pub const GRACEFUL_TASKS_WAIT_SECS: u64 = 10;

/// Trade response 消费者组名称
pub const TRADE_RESPONSE_CONSUMER_GROUP: &str = "trade_response_group";

/// Trade response 消费者名称前缀
pub const TRADE_RESPONSE_CONSUMER_PREFIX: &str = "trade_response_consumer";

/// Onchain action 消费者组名称
pub const ONCHAIN_ACTION_CONSUMER_GROUP: &str = "onchain_action_group";

/// Onchain action 消费者名称前缀
pub const ONCHAIN_ACTION_CONSUMER_PREFIX: &str = "onchain_action_consumer";
