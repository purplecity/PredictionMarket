pub const COMMON_ENV_PATH: &str = "./deploy/common.env";
pub const MATCH_ENGINE_CONFIG_PATH: &str = "./deploy/match_engine";
pub const STORE_CONFIG_PATH: &str = "./deploy/store";
pub const ASSET_CONFIG_PATH: &str = "./deploy/asset";
pub const EVENT_CONFIG_PATH: &str = "./deploy/event";
pub const API_CONFIG_PATH: &str = "./deploy/api";
pub const API_QUERY_CONFIG_PATH: &str = "./deploy/api_query";
pub const PROCESSOR_CONFIG_PATH: &str = "./deploy/processor";
pub const ONCHAIN_MSG_CONFIG_PATH: &str = "./deploy/onchain_msg";
pub const DEPTH_CONFIG_PATH: &str = "./deploy/depth";
pub const WEBSOCKET_USER_CONFIG_PATH: &str = "./deploy/websocket_user";
pub const WEBSOCKET_DEPTH_CONFIG_PATH: &str = "./deploy/websocket_depth";

///jason 推送的市场创建关闭消息的stream
pub const EVENT_MQ_STREAM: &str = "event-stream";
pub const EVENT_MQ_MSG_KEY: &str = "event-stream";

///推送给alen 新增用户的stream
pub const NEW_USER_STREAM: &str = "deepsense:asset:service:new_user";
pub const NEW_USER_MSG_KEY: &str = "new_user";

///推送给alen 发送交易请求的stream
pub const TRADE_SEND_STREAM: &str = "deepsense:onchain:service:send_request";
pub const TRADE_SEND_MSG_KEY: &str = "send_request";

///alen推送交易响应的stream
pub const TRADE_RESPONSE_STREAM: &str = "deepsense:onchain:service:send_response";
pub const TRADE_RESPONSE_MSG_KEY: &str = "send_response";

///alen推送链上事件的stream
pub const ONCHAIN_ACTION_STREAM: &str = "deepsense:asset:service:event";
pub const ONCHAIN_ACTION_MSG_KEY: &str = "event";

///撮合引擎接收订单输入的stream
pub const ORDER_INPUT_STREAM: &str = "order_input_stream";
pub const ORDER_INPUT_MSG_KEY: &str = "order_input_key";

///撮合引擎接收事件创建结束的stream
pub const EVENT_INPUT_STREAM: &str = "event_input_stream";
pub const EVENT_INPUT_MSG_KEY: &str = "event_input_key";

///撮合引擎推送订单变化到store的stream 用于更新store内存
pub const STORE_STREAM: &str = "store_stream";
pub const STORE_MSG_KEY: &str = "store_key";

///撮合引擎推送订单变化到processor的stream 用于处理撮合产出信息
pub const PROCESSOR_STREAM: &str = "processor_stream";
pub const PROCESSOR_MSG_KEY: &str = "processor_key";

///撮合引擎推送深度快照到depth的stream
pub const DEPTH_STREAM: &str = "depth_stream";
pub const DEPTH_MSG_KEY: &str = "depth_key";

///撮合引擎推送价格档位变化到websocket_depth的stream
pub const WEBSOCKET_STREAM: &str = "websocket_stream";
pub const WEBSOCKET_MSG_KEY: &str = "websocket_key";

/// api知道event的创建和关闭
pub const API_MQ_STREAM: &str = "api_mq_stream";
pub const API_MQ_MSG_KEY: &str = "api_mq_key";

///onchain_msg知道event的创建和关闭
pub const ONCHAIN_EVENT_STREAM: &str = "onchain_event_stream";
pub const ONCHAIN_EVENT_MSG_KEY: &str = "onchain_event_key";

/// 用户WebSocket事件流 - 用于推送订单和仓位变化给用户
pub const USER_EVENT_STREAM: &str = "user_event_stream";
pub const USER_EVENT_MSG_KEY: &str = "user_event_key";

/// Redis Stream 最大长度限制（使用 MAXLEN ~ 近似裁剪以提高性能）
/// 所有 stream 统一使用 10000 作为最大长度
pub const API_MQ_STREAM_MAXLEN: usize = 10000;
pub const ONCHAIN_EVENT_STREAM_MAXLEN: usize = 10000;
pub const WEBSOCKET_STREAM_MAXLEN: usize = 10000;
pub const USER_EVENT_STREAM_MAXLEN: usize = 10000;

/// Store服务快照文件存储目录
pub const STORE_DATA_DIR: &str = "./data/store";

/// Match Engine快照文件存储目录
/// 只要不删除数据库那么从STORE_DATA_DIR读也没问题 如果删除数据库又从STORE_DATA_DIR中读 就会因为trades表中order_id外键关联而失败 因为order_id不存在了
pub const MATCH_ENGINE_DATA_DIR: &str = "./data/match_engine";

/// 订单快照文件名
pub const ORDERS_SNAPSHOT_FILE: &str = "orders_snapshot.json";

/// Store服务快照保存间隔时间（秒）
pub const SNAPSHOT_INTERVAL_SECONDS: u64 = 5;

/// symbol切割符
pub const SYMBOL_SEPARATOR: &str = "-*******-";

/// 价格范围是[0.01,0.99] 刻度是0.0001 最多4位小数
pub const PRICE_MULTIPLIER: i32 = 10000;
/// 数量最多2位小数
pub const QUANTITY_MULTIPLIER: i32 = 100;

/// USDC Token ID (小写)
pub const USDC_TOKEN_ID: &str = "usdc";

/// Protobuf 字符串常量 - OrderSide
pub const ORDER_SIDE_BUY: &str = "buy";
pub const ORDER_SIDE_SELL: &str = "sell";
pub const ORDER_TYPE_LIMIT: &str = "limit";
pub const ORDER_TYPE_MARKET: &str = "market";

/// 运行模式常量
pub const RUN_MODE_DEV: &str = "dev";
pub const RUN_MODE_PROD: &str = "prod";

/// Redis DB 常量 - 用于统一管理各个 Redis 连接池使用的数据库编号
pub const REDIS_DB_COMMON_MQ: u32 = 0;
pub const REDIS_DB_WEBSOCKET_MQ: u32 = 3;
pub const REDIS_DB_CACHE: u32 = 4;
pub const REDIS_DB_LOCK: u32 = 5;
pub const REDIS_DB_ENGINE_INPUT_MQ: u32 = 6;
pub const REDIS_DB_ENGINE_OUTPUT_MQ: u32 = 7;

pub const REDIS_POOL_MAX_SIZE: usize = 32;

/// PostgreSQL 连接池配置常量
pub const POSTGRES_MAX_CONNECTIONS: u32 = 32;
pub const POSTGRES_MIN_CONNECTIONS: u32 = 1;
pub const POSTGRES_IDLE_TIMEOUT_SECS: u64 = 600;
pub const POSTGRES_MAX_LIFETIME_SECS: u64 = 1800;
pub const POSTGRES_ACQUIRE_TIMEOUT_SECS: u64 = 30;
pub const POSTGRES_TEST_BEFORE_ACQUIRE: bool = false;

/// 验证运行模式是否有效
pub fn validate_run_mode(run_mode: &str) -> anyhow::Result<()> {
	match run_mode {
		RUN_MODE_DEV | RUN_MODE_PROD => Ok(()),
		_ => Err(anyhow::anyhow!("Invalid RUN_MODE: {}. Must be either '{}' or '{}'", run_mode, RUN_MODE_DEV, RUN_MODE_PROD)),
	}
}
