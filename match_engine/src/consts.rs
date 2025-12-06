/// 订单输入流的消费者组名称
pub const ORDER_INPUT_CONSUMER_GROUP: &str = "order_input_consumer_group";

/// 订单输入流的消费者名称前缀
pub const ORDER_INPUT_CONSUMER_NAME_PREFIX: &str = "order_input_consumer";

/// 市场过期检查定时任务间隔（秒）
pub const EVENT_EXPIRY_CHECK_INTERVAL_SECS: u64 = 300; // 5 minutes

/// 市场过期延迟时间（秒），超过 end_timestamp + 这个时间后才会移除市场
pub const EVENT_EXPIRY_DELAY_SECS: i64 = 1800; // 30 minutes

/// 订单处理时间（秒），优雅停机时等待正在处理的订单完成的最大时间
pub const ORDER_PROCESSING_TIMEOUT_SECS: u64 = 120;

/// 退出时取消订单的批次大小
pub const CANCEL_ORDERS_BATCH_SIZE: usize = 100;
