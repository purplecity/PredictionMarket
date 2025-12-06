use crate::engine_types::{OrderSide, OrderType, PredictionSymbol};
// 重新导出公共类型以便兼容现有代码
use serde::{Deserialize, Serialize};

/// Processor 消息类型
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "types")]
pub enum ProcessorMessage {
	OrderRejected(OrderRejected),
	OrderSubmitted(OrderSubmitted),
	OrderTraded(OrderTraded),
	OrderCancelled(OrderCancelled),
}

/// 订单被拒绝 参数跟input的参数一样 再加了一个reason
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderRejected {
	pub order_id: String,
	pub symbol: PredictionSymbol,
	pub user_id: i64,
	pub privy_id: String, // 用于websocket推送
	pub reason: String,
}

/// 订单提交成功 但是未成交 参数跟input的参数一样 成为了maker
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderSubmitted {
	pub order_id: String,
	pub symbol: PredictionSymbol,
	pub side: OrderSide,
	pub order_type: OrderType,
	pub quantity: String,        //除以了精度的
	pub filled_quantity: String, //除以了精度的
	pub price: String,           //除以了精度的
	pub user_id: i64,
	pub privy_id: String,     // 用于websocket推送
	pub outcome_name: String, // 用于websocket推送
}

/// 订单取消 可能成交了一部分
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderCancelled {
	pub order_id: String,
	pub symbol: PredictionSymbol,
	pub user_id: i64,
	pub privy_id: String, // 用于websocket推送
	pub cancelled_quantity: String,
	pub cancelled_volume: String,
}

/// 部分成交和完全成交一样 在processor中不能直接去替换数据库订单(因为多个消费者可能同时修改相同的单子 比如一个trade对应部分成交 一个trade对应完全成交 但是这两个trade处理时间顺序是随机的) 只能根据分布式锁慢慢减去单子直到等于0就是完全成交
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderTraded {
	pub taker_symbol: PredictionSymbol, //taker的symbol
	pub taker_id: i64,
	pub taker_privy_user_id: String, // 用于websocket推送
	pub taker_outcome_name: String,  // 用于websocket推送
	pub taker_order_id: String,
	pub taker_order_side: OrderSide,
	pub taker_token_id: String,
	pub trades: Vec<Trade>,
}

/// 交易 撮合交易是交叉撮合的 所以taker和maker可能有自己的单独的token_id和价格
/// 但是放到链上去 taker是聚合的
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trade {
	pub timestamp: i64, //撮合成交时间戳
	pub event_id: i64,
	pub market_id: i16,
	pub quantity: String, //不管token id相同不相同撮合成交的数量是相同的 除以了精度的

	pub taker_usdc_amount: String, //除以了精度的
	pub taker_price: String,       //除以了精度的

	pub maker_id: i64,
	pub maker_privy_user_id: String, // 用于websocket推送
	pub maker_outcome_name: String,  // 用于websocket推送
	pub maker_order_id: String,
	pub maker_order_side: OrderSide,
	pub maker_token_id: String,
	pub maker_usdc_amount: String, //除以了精度的
	pub maker_price: String,       //除以了精度的

	pub maker_filled_quantity: String, //除以了精度的 为了推送websocket使用
	pub maker_quantity: String,        //除以了精度的 为了推送websocket使用 maker的原始订单数量
}
