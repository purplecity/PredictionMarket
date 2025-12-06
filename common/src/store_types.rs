use {
	crate::{
		engine_types::{Order, PredictionSymbol},
		event_types::EngineMQEventCreate,
	},
	chrono::Utc,
	serde::{Deserialize, Serialize},
	std::collections::HashMap,
};

/// 订单变化事件类型（用于推送到 store）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum OrderChangeEvent {
	/// 订单创建（新订单加入订单簿）
	OrderCreated(Order),
	/// 订单更新（部分成交）
	OrderUpdated(Order),
	/// 订单完全成交
	OrderFilled { order_id: String, symbol: PredictionSymbol },
	/// 订单取消
	OrderCancelled { order_id: String, symbol: PredictionSymbol },
	/// 市场添加（市场创建时发送，包含市场ID、选项列表和结束时间戳）
	EventAdded(EngineMQEventCreate),
	/// 市场移除（市场到期或被删除时发送）
	EventRemoved(i64),
	/// 订单或撮合操作导致的 update_id 更新（用于同步每个 market 的最新 update_id）
	MarketUpdateId { event_id: i64, market_id: i16, update_id: u64 },
}

/// 订单快照（按 symbol 组织）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderSnapshot {
	pub symbol: PredictionSymbol,
	pub orders: Vec<Order>,
	pub timestamp: i64, //快照时间戳
}

/// 快照专用的 Market 信息（带 update_id）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotMarket {
	pub market_id: i16,         //选项id
	pub outcomes: Vec<String>,  //结果列表
	pub token_ids: Vec<String>, //token id列表
	pub update_id: u64,         //这个 market 的最新 update_id
}

/// 快照专用的 Event 信息（带每个 market 的 update_id）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotEvent {
	pub event_id: i64,
	pub markets: HashMap<String, SnapshotMarket>, //选项信息，key 是 market_id 的字符串形式
	pub end_date: Option<chrono::DateTime<Utc>>,  //市场不一定有准确的结束时间
}

/// 所有订单的快照（按 symbol 组织）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllOrdersSnapshot {
	pub snapshots: Vec<OrderSnapshot>,
	pub events: HashMap<i64, SnapshotEvent>, // 使用新的 SnapshotEvent 类型
	pub timestamp: i64,                      //快照时间戳
	#[serde(default)]
	pub last_message_id: Option<String>, // 已处理的最新消息 ID
}
