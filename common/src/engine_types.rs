use {
	crate::{
		consts::SYMBOL_SEPARATOR,
		event_types::{EngineMQEventCreate, MQEventClose},
	},
	core::fmt,
	serde::{Deserialize, Serialize},
	std::{fmt::Display, str::FromStr},
};

/// 订单类型（预测市场只支持限价单和市价单）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "order_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum OrderType {
	/// 限价单
	Limit,
	/// 市价单
	Market,
}

impl Display for OrderType {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"{}",
			match self {
				OrderType::Limit => "limit",
				OrderType::Market => "market",
			}
		)
	}
}

/// 订单方向
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Hash, sqlx::Type)]
#[sqlx(type_name = "order_side", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum OrderSide {
	/// 买入
	Buy,
	/// 卖出
	Sell,
}

impl Display for OrderSide {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"{}",
			match self {
				OrderSide::Buy => "buy",
				OrderSide::Sell => "sell",
			}
		)
	}
}

/// 订单状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "order_status", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum OrderStatus {
	/// 新订单
	New,
	/// 部分成交
	PartiallyFilled,
	/// 完全成交
	Filled,
	/// 已取消
	Cancelled,
	/// 拒绝
	Rejected,
}

impl Display for OrderStatus {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"{}",
			match self {
				OrderStatus::New => "New",
				OrderStatus::PartiallyFilled => "PartiallyFilled",
				OrderStatus::Filled => "Filled",
				OrderStatus::Cancelled => "Cancelled",
				OrderStatus::Rejected => "Rejected",
			}
		)
	}
}

/// 预测市场交易标识符（市场id + 选项id + 结果token代币）
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct PredictionSymbol {
	pub event_id: i64,
	pub market_id: i16,
	pub token_id: String,
}

impl PredictionSymbol {
	pub fn new(event_id: i64, market_id: i16, token_id: &str) -> Self {
		Self { event_id, market_id, token_id: token_id.to_string() }
	}
}

impl Display for PredictionSymbol {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(f, "{}{}{}{}{}", self.event_id, SYMBOL_SEPARATOR, self.market_id, SYMBOL_SEPARATOR, self.token_id)
	}
}

impl FromStr for PredictionSymbol {
	type Err = String;

	fn from_str(s: &str) -> Result<Self, Self::Err> {
		let parts: Vec<&str> = s.split(SYMBOL_SEPARATOR).collect();
		if parts.len() != 3 {
			return Err(format!("Invalid symbol format: expected event_id{}market_name{}outcome, got {}", SYMBOL_SEPARATOR, SYMBOL_SEPARATOR, s));
		}

		let event_id = parts[0].parse::<i64>().map_err(|e| e.to_string())?;
		let market_id = parts[1].parse::<i16>().map_err(|e| e.to_string())?;
		let token_id = parts[2].to_string();
		Ok(Self { event_id, market_id, token_id })
	}
}

/// 撮合内存订单存储
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Order {
	pub order_id: String, //订单id
	pub symbol: PredictionSymbol,
	pub side: OrderSide,
	pub order_type: OrderType,
	pub quantity: u64,              // 预测市场交易的数量 其实是股份 最多2位小数乘以了100 变成了整数
	pub price: i32,                 // 原始价格乘以10000变成整数 [0.0001,0.9999]->[1,9999]
	pub opposite_result_price: i32, // 相反结果的价格 等于10000-price
	pub status: OrderStatus,
	pub filled_quantity: u64,
	pub remaining_quantity: u64,
	pub timestamp: i64, //撮合接受时间戳
	pub user_id: i64,
	pub order_num: u64,       //订单编号 用于撮合在价格相同的情况下先来先撮合
	pub privy_id: String,     // 用于websocket推送
	pub outcome_name: String, // 用于websocket推送
}

impl Order {
	#[allow(clippy::too_many_arguments)]
	pub fn new(
		order_id: String,
		symbol: PredictionSymbol,
		side: OrderSide,
		order_type: OrderType,
		quantity: u64,
		price: i32,
		user_id: i64,
		privy_id: String,
		outcome_name: String,
	) -> anyhow::Result<Self> {
		use chrono::Utc;
		let datetime = Utc::now();
		let timestamp = datetime.timestamp_millis();
		if !(100..=9900).contains(&price) {
			return Err(anyhow::anyhow!("Invalid price: {}", price));
		}

		Ok(Self {
			order_id,
			symbol,
			side,
			order_type,
			quantity,
			price,
			opposite_result_price: 10000 - price,
			status: OrderStatus::New,
			filled_quantity: 0,
			remaining_quantity: quantity,
			timestamp,
			user_id,
			order_num: 0,
			privy_id,
			outcome_name,
		})
	}

	/// 检查订单是否可以跟maker匹配
	pub fn can_match(&self, maker: &Order) -> bool {
		// 必须是同一个symbol（同一市场、同一选项、同一结果）
		if self.symbol != maker.symbol {
			return false;
		}

		//预测市场不同 市价单也是有价格的 也要看价格说话
		//市价单总是可以匹配
		// if self.order_type == OrderType::Market {
		// 	return true;
		// }

		//相同方向也可以匹配的 比如不同结果的买和卖是可以匹配的 因为是交叉撮合规则
		// 检查价格匹配
		match (self.side, maker.side) {
			(OrderSide::Buy, OrderSide::Sell) => {
				// 必须同一个结果
				if self.symbol.token_id != maker.symbol.token_id {
					return false;
				}
				// 买单价格 >= 卖单价格
				self.price >= maker.price
			}
			(OrderSide::Sell, OrderSide::Buy) => {
				// 必须同一个结果
				if self.symbol.token_id != maker.symbol.token_id {
					return false;
				}
				// 卖单价格 <= 买单价格
				self.price <= maker.price
			}
			(OrderSide::Buy, OrderSide::Buy) => {
				// 不能是同一个结果
				if self.symbol.token_id == maker.symbol.token_id {
					return false;
				}
				// 相反结果的相反方向的价格  比如maker 挂了 3000price买no 那么等价于挂了一个7000price的yes结果的卖单
				self.price >= maker.opposite_result_price
			}
			(OrderSide::Sell, OrderSide::Sell) => {
				// 不能是同一个结果
				if self.symbol.token_id == maker.symbol.token_id {
					return false;
				}
				// 相反结果的相反方向的价格  比如maker 挂了 3000price卖no 那么等价于挂了一个7000price的yes结果的买单
				self.price <= maker.opposite_result_price
			}
		}
	}

	/// 计算匹配价格（价格优先原则）始终是maker价格 能成为maker肯定是有价格
	pub fn match_price(&self, maker: &Order) -> i32 {
		maker.price
	}
}

impl Display for Order {
	fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
		write!(
			f,
			"Order {{ order_id: {}, symbol: {}, side: {}, order_type: {}, quantity: {}, price: {}, opposite_result_price: {}, status: {}, filled_quantity: {}, remaining_quantity: {}, timestamp: {}, user_id: {}, order_num: {}, privy_id: {}, outcome_name: {} }}",
			self.order_id,
			self.symbol,
			self.side,
			self.order_type,
			self.quantity,
			self.price,
			self.opposite_result_price,
			self.status,
			self.filled_quantity,
			self.remaining_quantity,
			self.timestamp,
			self.user_id,
			self.order_num,
			self.privy_id,
			self.outcome_name
		)
	}
}

//input消息
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "types")]
pub enum OrderInputMessage {
	SubmitOrder(SubmitOrderMessage),
	CancelOrder(CancelOrderMessage),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubmitOrderMessage {
	pub order_id: String,
	pub symbol: PredictionSymbol,
	pub side: OrderSide,
	pub order_type: OrderType,
	pub quantity: u64, //乘以了100的 原始数字应该是最多保留2位小数 下单时候校验
	pub price: i32,    //乘以了10000的 原始数字应该是最多保留4位小数 下单时候校验
	pub user_id: i64,
	pub privy_id: String,     // 用于websocket推送
	pub outcome_name: String, // 用于websocket推送
}

// 取消所有订单的话就是一个个取消
#[derive(Debug, Serialize, Deserialize)]
pub struct CancelOrderMessage {
	pub symbol: PredictionSymbol,
	pub order_id: String,
}

/// 市场一旦开启 单个市场就没有pause和resume
/// 但是全局给了下
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "types")]
pub enum EventInputMessage {
	AddOneEvent(EngineMQEventCreate),
	RemoveOneEvent(MQEventClose),            //到期remove
	StopAllEvents(StopAllEventsMessage),     //维护停机 所有市场不再接单
	ResumeAllEvents(ResumeAllEventsMessage), //维护恢复 中断后的市场可以重新接单
}

#[derive(Debug, Serialize, Deserialize)]
pub struct StopAllEventsMessage {
	pub stop: bool, // false 无操作  true 所有市场不再接单 但是用户可以提交订单 撮合引擎会拒绝
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ResumeAllEventsMessage {
	pub resume: bool, // false 无操作  true 所有市场可以重新接单
}
