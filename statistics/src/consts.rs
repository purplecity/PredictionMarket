// 积分系统通用常量 - 从 common::consts 导出
pub use common::consts::{DEFAULT_INVITE_POINTS_RATIO, DEFAULT_INVITEE_BOOST, DEFAULT_TRADING_POINTS_RATIO, INVITE_RATIO_DIVISOR, TRADING_RATIO_DIVISOR};

/// 限价单最小挂单时长（秒）- 10分钟
pub const MIN_ORDER_DURATION_SECS: i64 = 600;

/// 限价单最小挂单USDC数量
pub const MIN_ORDER_USDC_AMOUNT: &str = "10";

/// 单个价格档位最高积分
pub const MAX_POINTS_PER_PRICE_LEVEL: i64 = 10;

/// 交易量最低阈值 (200 USDC)
pub const MIN_TRADING_VOLUME_USDC: &str = "200";

/// Graceful shutdown 等待消费者完成的时间（秒）
pub const GRACEFUL_CONSUMER_WAIT_SECS: u64 = 5;

/// 优雅停机时等待任务完成的时间（秒）
pub const GRACEFUL_TASKS_WAIT_SECS: u64 = 5;

// ============================================================================
// 空投配置
// ============================================================================

/// Disperse.app 合约地址
pub const DISPERSE_CONTRACT_ADDRESS: &str = "0x34d7bde1b0d376f5d5fee556db834269fbbbd0bd";

/// USDC ERC20 合约地址
pub const USDC_CONTRACT_ADDRESS: &str = "0x462AFce0eDAf8f63F9D029A3168a22a95A49803C";

/// 每个用户空投的原生代币数量 (18位小数, 0.0001 = 10^14)
pub const AIRDROP_NATIVE_AMOUNT_PER_USER: u128 = 100_000_000_000_000;

/// 原生代币余额阈值 (低于此值不执行空投, 0.002 = 20 * 0.0001)
pub const AIRDROP_NATIVE_THRESHOLD: u128 = 2_000_000_000_000_000;

/// 每个用户空投的 USDC 数量 (18位小数, 500 USDC = 500 * 10^18)
pub const AIRDROP_AMOUNT_PER_USER: u128 = 500_000_000_000_000_000_000;

/// USDC 余额阈值 (低于此值不执行空投, 10_000 USDC)
pub const AIRDROP_USDC_THRESHOLD: u128 = 10_000_000_000_000_000_000_000;

/// 每批最多处理的空投消息数
pub const AIRDROP_BATCH_SIZE: usize = 20;

/// 每轮处理完后休息的时间（秒）
pub const AIRDROP_WAIT_MORE_SECS: u64 = 3;

/// 阻塞等待消息的超时时间（毫秒）
pub const AIRDROP_BLOCK_TIMEOUT_MS: usize = 5000;

/// 空投交易发送+确认的超时时间（秒）
pub const AIRDROP_TX_TIMEOUT_SECS: u64 = 30;
