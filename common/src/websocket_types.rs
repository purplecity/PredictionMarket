use {
	serde::{Deserialize, Serialize},
	std::collections::HashMap,
};

/// 价格档位行情变化
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevelInfo {
	pub price: String,          //除以了精度的 用于前端显示和websocket服务构造订单簿
	pub total_quantity: String, //除以了精度的 用于前端显示和websocket服务构造订单簿
	pub total_size: String,     //除以了精度的 就是price*total_quantity的和
}

/// 单个token的价格档位变化
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleTokenPriceInfo {
	pub latest_trade_price: String,
	pub bids: Vec<PriceLevelInfo>,
	pub asks: Vec<PriceLevelInfo>,
}

/// WebSocket 价格档位变化消息 - 包含同一market下两个token的变化
/// 两个token共享同一个update_id，因为订单簿是交叉处理的
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketPriceChanges {
	pub event_id: i64,
	pub market_id: i16,
	pub update_id: u64,
	pub timestamp: i64,
	pub changes: HashMap<String, SingleTokenPriceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSocketDepth {
	pub event_id: i64,
	pub market_id: i16,
	pub update_id: u64,
	pub timestamp: i64,
	pub depths: HashMap<String, SingleTokenPriceInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "types", rename_all = "snake_case")]
pub enum OpenOrderChangeEvent {
	OpenOrderCreated(UserOpenOrders),
	OpenOrderUpdated(UserOpenOrders),
	OpenOrderCancelled { event_id: i64, market_id: i16, order_id: String, privy_id: String, update_id: i64 },
	OpenOrderFilled { event_id: i64, market_id: i16, order_id: String, privy_id: String, update_id: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserOpenOrders {
	pub privy_id: String,
	pub event_id: i64,
	pub market_id: i16,
	pub order_id: String,
	pub side: String,
	pub outcome_name: String,
	pub price: String,
	pub quantity: String,
	pub volume: String,
	pub filled_quantity: String,
	pub update_id: i64,
	pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "types", rename_all = "snake_case")]
pub enum PositionChangeEvent {
	PositionCreated(UserPositions),
	PositionUpdated(UserPositions),
	PositionRemoved { event_id: i64, market_id: i16, token_id: String, privy_id: String, update_id: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPositions {
	pub privy_id: String,
	pub event_id: i64,
	pub market_id: i16,
	pub outcome_name: String,
	pub token_id: String,
	pub avg_price: String,
	pub quantity: String,
	pub update_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerMsg {
	pub event_id: i64,
	//对于深度就是book和price_change
	//对于个人信息就是open_order_change和position_change
	pub event_type: String,
	pub data: serde_json::Value,
}

/// 用户事件包装类型 - 用于在单个Redis Stream中传输不同类型的用户事件
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event_type", rename_all = "snake_case")]
pub enum UserEvent {
	OpenOrderChange(OpenOrderChangeEvent),
	PositionChange(PositionChangeEvent),
}

/// API Key 变更事件 - 用于通知 websocket_user 添加/删除 API Key
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum ApiKeyEvent {
	/// 添加 API Key
	Add { api_key: String, privy_id: String },
	/// 删除 API Key
	Remove { api_key: String },
}

impl UserEvent {
	/// 获取事件对应的用户ID
	pub fn get_privy_user_id(&self) -> &str {
		match self {
			UserEvent::OpenOrderChange(event) => {
				match event {
					OpenOrderChangeEvent::OpenOrderCreated(order) => &order.privy_id,
					OpenOrderChangeEvent::OpenOrderUpdated(order) => &order.privy_id,
					OpenOrderChangeEvent::OpenOrderCancelled { privy_id, .. } => privy_id,
					OpenOrderChangeEvent::OpenOrderFilled { privy_id, .. } => privy_id,
				}
			}
			UserEvent::PositionChange(event) => {
				match event {
					PositionChangeEvent::PositionCreated(pos) => &pos.privy_id,
					PositionChangeEvent::PositionUpdated(pos) => &pos.privy_id,
					PositionChangeEvent::PositionRemoved { privy_id, .. } => privy_id,
				}
			}
		}
	}

	/// 获取事件对应的event_id
	pub fn get_event_id(&self) -> i64 {
		match self {
			UserEvent::OpenOrderChange(event) => {
				match event {
					OpenOrderChangeEvent::OpenOrderCreated(order) => order.event_id,
					OpenOrderChangeEvent::OpenOrderUpdated(order) => order.event_id,
					OpenOrderChangeEvent::OpenOrderCancelled { event_id, .. } => *event_id,
					OpenOrderChangeEvent::OpenOrderFilled { event_id, .. } => *event_id,
				}
			}
			UserEvent::PositionChange(event) => {
				match event {
					PositionChangeEvent::PositionCreated(pos) => pos.event_id,
					PositionChangeEvent::PositionUpdated(pos) => pos.event_id,
					PositionChangeEvent::PositionRemoved { event_id, .. } => *event_id,
				}
			}
		}
	}

	/// 获取事件类型字符串
	pub fn get_event_type_str(&self) -> &'static str {
		match self {
			UserEvent::OpenOrderChange(_) => "open_order_change",
			UserEvent::PositionChange(_) => "position_change",
		}
	}

	/// 获取 update_id
	pub fn get_update_id(&self) -> i64 {
		match self {
			UserEvent::OpenOrderChange(event) => {
				match event {
					OpenOrderChangeEvent::OpenOrderCreated(order) => order.update_id,
					OpenOrderChangeEvent::OpenOrderUpdated(order) => order.update_id,
					OpenOrderChangeEvent::OpenOrderCancelled { update_id, .. } => *update_id,
					OpenOrderChangeEvent::OpenOrderFilled { update_id, .. } => *update_id,
				}
			}
			UserEvent::PositionChange(event) => {
				match event {
					PositionChangeEvent::PositionCreated(pos) => pos.update_id,
					PositionChangeEvent::PositionUpdated(pos) => pos.update_id,
					PositionChangeEvent::PositionRemoved { update_id, .. } => *update_id,
				}
			}
		}
	}

	/// 获取 tracking key (用于 websocket 消息顺序控制)
	/// 对于 order: order_id
	/// 对于 position: token_id
	pub fn get_tracking_key(&self) -> String {
		match self {
			UserEvent::OpenOrderChange(event) => {
				match event {
					OpenOrderChangeEvent::OpenOrderCreated(order) => order.order_id.clone(),
					OpenOrderChangeEvent::OpenOrderUpdated(order) => order.order_id.clone(),
					OpenOrderChangeEvent::OpenOrderCancelled { order_id, .. } => order_id.clone(),
					OpenOrderChangeEvent::OpenOrderFilled { order_id, .. } => order_id.clone(),
				}
			}
			UserEvent::PositionChange(event) => {
				match event {
					PositionChangeEvent::PositionCreated(pos) => pos.token_id.clone(),
					PositionChangeEvent::PositionUpdated(pos) => pos.token_id.clone(),
					PositionChangeEvent::PositionRemoved { token_id, .. } => token_id.clone(),
				}
			}
		}
	}
}
