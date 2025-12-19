use {
	common::engine_types::{PredictionSymbol, SubmitOrderMessage},
	serde::{Deserialize, Serialize},
};

/// 订单簿处理任务的控制消息
#[derive(Debug)]
pub enum OrderBookControl {
	/// 提交订单进行撮合
	SubmitOrder(SubmitOrderMessage),
	/// 取消订单
	CancelOrder(String),
}

/// 订单簿统计信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookStats {
	pub symbol: PredictionSymbol,
	pub bid_levels: usize,
	pub ask_levels: usize,
	pub total_bid_orders: usize,
	pub total_ask_orders: usize,
	pub total_bid_quantity: u64,
	pub total_ask_quantity: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceLevel {
	pub price: String,           //格式化了之后
	pub price_i32: i32,          //保持乘了10000,用于排序
	pub total_quantity: String,  //格式化了之后的
	pub total_quantity_u64: u64, //乘以100的 用于对比上一秒的深度快照生成价格档位变化
	pub order_count: usize,      //这个价格档位的订单数量
}

/// 订单簿深度
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderBookDepth {
	pub symbol: PredictionSymbol,
	pub bids: Vec<PriceLevel>, // 买盘，价格从高到低
	pub asks: Vec<PriceLevel>, // 卖盘，价格从低到高
	pub timestamp: i64,        //快照时间戳
	pub update_id: u64,
}
