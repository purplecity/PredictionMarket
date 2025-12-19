use {
	chrono::Utc,
	rust_decimal::Decimal,
	serde::{Deserialize, Serialize},
	std::collections::HashMap,
};

// 用于/api/api_query的市场创建
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMQEventCreate {
	pub event_id: i64,                              //市场id
	pub event_identifier: String,                   //市场唯一标识符
	pub slug: String,                               //标识符 title小写替换空格用于url
	pub title: String,                              //标题
	pub description: String,                        //描述
	pub image: String,                              //图片地址
	pub end_date: Option<chrono::DateTime<Utc>>,    //市场不一定有准确的结束时间
	pub topic: String,                              //主题类别 crypto sport等等
	pub markets: HashMap<String, ApiMQEventMarket>, //选项信息
	pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMQEventMarket {
	pub market_id: i16,                        //选项id
	pub parent_collection_id: String,          //父集合id
	pub condition_id: String,                  //条件id
	pub market_identifier: String,             //选项唯一标识符
	pub question: String,                      //问题
	pub slug: String,                          //标识符 title小写替换空格替换左斜线
	pub title: String,                         //标题
	pub image: String,                         //图片地址
	pub outcome_info: HashMap<String, String>, //代币地址-结果名称
	pub outcome_names: Vec<String>,            //如果是yes/no yes在前no在后 否则就按照字典序存储
	pub outcome_token_ids: Vec<String>,        //token id
}
// 用于match_engin/store的市场创建
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineMQEventCreate {
	pub event_id: i64,
	pub markets: HashMap<String, EngineMQEventMarket>, //选项信息
	pub end_date: Option<chrono::DateTime<Utc>>,       //市场不一定有准确的结束时间
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EngineMQEventMarket {
	pub market_id: i16,         //选项id
	pub outcomes: Vec<String>,  //结果列表
	pub token_ids: Vec<String>, //token id列表
}

//用于api/api_query/engine/store的结束
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MQEventClose {
	pub event_id: i64,
}

//用于onchain_msg的市场创建/结束
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnchainMQEventCreate {
	pub event_id: i64,
	pub markets: HashMap<String, OnchainMQEventMarket>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnchainMQEventMarket {
	pub market_id: i16,
	pub condition_id: String,
	pub token_ids: Vec<String>,
	#[serde(default)]
	pub outcomes: Vec<String>,
}

/// 单个 Market 添加消息 (用于 onchain_event_stream)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnchainMQMarketAdd {
	pub event_id: i64,
	pub market: OnchainMQEventMarket,
}

/// 单个 Market 关闭消息 (用于 onchain_event_stream)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OnchainMQMarketClose {
	pub event_id: i64,
	pub market_id: i16,
	pub win_outcome_token_id: String,
	pub win_outcome_name: String,
}

/// 统一的事件消息枚举 (用于 onchain_event_stream)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "types")]
pub enum OnchainEventMessage {
	Create(OnchainMQEventCreate),
	Close(MQEventClose),
	MarketAdd(OnchainMQMarketAdd),
	MarketClose(OnchainMQMarketClose),
}

/// API 单个 Market 添加消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMQMarketAdd {
	pub event_id: i64,
	pub market_id: i16,
	pub market: ApiMQEventMarket,
}

/// API 单个 Market 关闭消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiMQMarketClose {
	pub event_id: i64,
	pub market_id: i16,
}

/// API 事件消息枚举 (用于 api_mq_stream)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "types")]
pub enum ApiEventMqMessage {
	#[serde(rename = "EventCreate")]
	EventCreate(Box<ApiMQEventCreate>),
	#[serde(rename = "EventClose")]
	EventClose(MQEventClose),
	#[serde(rename = "MarketAdd")]
	MarketAdd(Box<ApiMQMarketAdd>),
	#[serde(rename = "MarketClose")]
	MarketClose(ApiMQMarketClose),
}

/// Event 交易量统计 (用于 Redis 缓存)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheEventVolume {
	pub event_id: i64,
	pub total_volume: Decimal,
	pub market_volumes: Vec<CacheMarketVolume>,
}

/// Market 交易量统计
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMarketVolume {
	pub market_id: i16,
	pub volume: Decimal,
}
