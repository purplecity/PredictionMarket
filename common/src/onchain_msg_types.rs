use {
	crate::model::SignatureOrderMsg,
	serde::{Deserialize, Serialize},
};
//推送新的用户注册消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewUser {
	pub user_id: i64,
	pub privy_id: String,
	pub privy_evm_address: String,
}

//充值
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Deposit {
	pub user_id: i64,
	pub privy_id: String,
	pub user_address: String,
	pub token_id: String,
	pub amount: String, //乘以了精度的 我自己转
	pub tx_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Withdraw {
	pub user_id: i64,
	pub privy_id: String,
	pub user_address: String,
	pub token_id: String,
	pub amount: String, //乘以了精度的 我自己转
	pub tx_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Split {
	pub user_id: i64,
	pub privy_id: String,
	pub user_address: String,
	pub amount: String, //乘以了精度的 我自己转
	pub condition_id: String,
	pub tx_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Merge {
	pub user_id: i64,
	pub privy_id: String,
	pub user_address: String,
	pub amount: String, //乘以了精度的 我自己转
	pub condition_id: String,
	pub tx_hash: String,
}

//直接就是把用户token全部赎回为usdc 结果代币不管相不相等 有多少等
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Redeem {
	pub user_id: i64,
	pub privy_id: String,
	pub user_address: String,
	pub payout: String, //乘以了精度的 我自己转
	pub condition_id: String,
	pub tx_hash: String,
}

//processor发出去的消息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeOnchainSendRequest {
	pub match_info: MatchOrderInfo,
	pub trade_id: String,
	pub event_id: i64,
	pub market_id: i32,
	pub taker_trade_info: TakerTradeInfo,
	pub maker_trade_infos: Vec<MakerTradeInfo>,
}

//交易发送的回应 相比于request去掉了match_info 加上了用于判断成功或者失败的tx_hash和success字段
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradeOnchainSendResponse {
	pub trade_id: String,
	pub event_id: i64,
	pub market_id: i32,
	pub taker_trade_info: TakerTradeInfo,
	pub maker_trade_infos: Vec<MakerTradeInfo>,
	pub tx_hash: String,
	pub success: bool,
}

//用于合约交互 后续还要交手续费
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MatchOrderInfo {
	pub taker_order: SignatureOrderMsg,
	pub maker_order: Vec<SignatureOrderMsg>,
	pub taker_fill_amount: String,      //乘以了精度的
	pub taker_receive_amount: String,   //乘以了精度的
	pub maker_fill_amount: Vec<String>, //乘以了精度的
}

//taker没有价格 taker是吃多个maker的可能
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakerTradeInfo {
	pub taker_side: String,
	pub taker_user_id: i64,
	pub taker_usdc_amount: String,  //除以了10的精度次方的 从decimal转化而来 因为这是内部使用
	pub taker_token_amount: String, //除以了10的精度次方的 从decimal转化而来 因为这是内部使用
	pub taker_token_id: String,
	pub taker_order_id: String,
	pub taker_unfreeze_amount: String,
	pub real_taker_usdc_amount: String,  //真实的变化 除以了10的精度次方的 从decimal转化而来 因为这是内部使用
	pub real_taker_token_amount: String, //真实的变化 除以了10的精度次方的 从decimal转化而来 因为这是内部使用
	pub taker_privy_user_id: String,     // 用于websocket推送
	pub taker_outcome_name: String,      // 用于websocket推送
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MakerTradeInfo {
	pub maker_side: String,
	pub maker_user_id: i64,
	pub maker_usdc_amount: String,  //除以了10的精度次方的 从decimal转化而来 因为这是内部使用
	pub maker_token_amount: String, //除以了10的精度次方的 从decimal转化而来 因为这是内部使用
	pub maker_token_id: String,
	pub maker_order_id: String,
	pub maker_price: String,
	pub real_maker_usdc_amount: String,  //真实的变化 除以了10的精度次方的 从decimal转化而来 因为这是内部使用
	pub real_maker_token_amount: String, //真实的变化 除以了10的精度次方的 从decimal转化而来 因为这是内部使用
	pub maker_privy_user_id: String,     // 用于websocket推送
	pub maker_outcome_name: String,      // 用于websocket推送
}
