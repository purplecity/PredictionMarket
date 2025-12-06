use {
	crate::engine_types::{OrderSide, OrderStatus, OrderType},
	chrono::Utc,
	rust_decimal::Decimal,
	serde::{Deserialize, Serialize},
	sqlx::prelude::FromRow,
	std::collections::HashMap,
};

/// https://docs.rs/sqlx/latest/sqlx/postgres/types/index.html 注意类型转换

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Users {
	pub id: i64,
	//privy信息
	pub privy_id: String,
	pub privy_evm_address: String,
	pub privy_email: String,
	pub privy_x: String,
	pub privy_x_image: String,
	//用户可编辑信息
	pub name: String,
	pub bio: String,
	pub profile_image: String,
	//登陆信息
	pub last_login_ip: String,
	pub last_login_region: String,
	pub last_login_at: chrono::DateTime<Utc>,
	//注册信息
	pub registered_ip: String,
	pub registered_region: String,
	pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Events {
	pub id: i64,                                                  //市场id
	pub event_identifier: String,                                 //市场唯一标识符
	pub slug: String,                                             //标识符 title小写替换空格用于url
	pub title: String,                                            //标题
	pub description: String,                                      //描述
	pub image: String,                                            //图片地址
	pub end_date: Option<chrono::DateTime<Utc>>,                  //市场不一定有准确的结束时间
	pub closed: bool,                                             //合约层面市场提交结束 结束了不一定resolved
	pub closed_at: Option<chrono::DateTime<Utc>>,                 //收到关闭市场消息的时间
	pub resolved: bool,                                           //程序在closed收到之后处理resolved
	pub resolved_at: Option<chrono::DateTime<Utc>>,               //处理完成时间
	pub topic: String,                                            //主题类别 crypto sport等等
	pub volume: Decimal,                                          //用于首页排序
	pub markets: sqlx::types::Json<HashMap<String, EventMarket>>, //选项信息
	pub recommended: bool,                                        //是否推荐
	pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMarket {
	pub id: i16, //选项id
	pub condition_id: String,
	pub parent_collection_id: String,
	pub market_identifier: String,    //选项唯一标识符
	pub question: String,             //问题
	pub slug: String,                 //标识符 title小写替换空格替换左斜线
	pub title: String,                //标题
	pub image: String,                //图片地址
	pub outcomes: Vec<String>,        //如果是yes/no yes在前no在后 否则就按照字典序存储
	pub token_ids: Vec<String>,       //token id
	pub win_outcome_name: String,     //赢的结果名称
	pub win_outcome_token_id: String, //赢的结果token id
}
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct EventTopics {
	pub topic: String,
	pub active: bool,
}

//用户订单
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Orders {
	pub id: uuid::Uuid,   //订单id
	pub user_id: i64,     //用户id
	pub event_id: i64,    //市场id
	pub market_id: i16,   //选项id
	pub token_id: String, //token id
	pub outcome: String,  //outcome名字

	pub order_side: OrderSide, //买入或卖出
	pub order_type: OrderType, //限价单或市价单

	pub price: Decimal,              //就算是市价单也有价格
	pub quantity: Decimal,           //提交的数量 永远是token的数量 而不是usdc
	pub volume: Decimal,             //用以冻结usdc 要自己算下等于price*quantity 前端不会传volume 但是不管是限价单还是市价单一定会有price 市价单没吃完会直接取消
	pub filled_quantity: Decimal,    //已成交的数量 永远是token的数量 而不是usdc
	pub cancelled_quantity: Decimal, //已取消的数量 永远是token的数量 而不是usdc
	pub status: OrderStatus,         //订单状态

	pub signature_order_msg: sqlx::types::Json<SignatureOrderMsg>, //发送交易需要 保持前端格式不动
	pub update_id: i64,                                            //用于websocket消息顺序保证，每次更新自增
	pub created_at: chrono::DateTime<Utc>,
	pub updated_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignatureOrderMsg {
	pub expiration: String,
	#[serde(rename = "feeRateBps")]
	pub fee_rate_bps: String,
	pub maker: String,
	#[serde(rename = "makerAmount")]
	pub maker_amount: String,
	pub nonce: String,
	pub salt: i64,
	pub side: String,
	pub signature: String,
	#[serde(rename = "signatureType")]
	pub signature_type: i16,
	pub signer: String,
	pub taker: String,
	#[serde(rename = "takerAmount")]
	pub taker_amount: String,
	#[serde(rename = "tokenId")]
	pub token_id: String,
}

//撮合成交信息
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Trades {
	pub batch_id: uuid::Uuid,                   //发送到链上的批次id
	pub match_timestamp: chrono::DateTime<Utc>, //单笔trade的撮合时间
	pub order_id: uuid::Uuid,                   //订单id

	pub taker: bool, //是否是taker用于统计
	//batch trade的交易量
	//等于taker的usdc_amount+(maker中相同方向的usdc_amount之和)
	//或者说等于(maker中相反方向的usdc_amount之和)+(相同方向的token_amount之和)
	pub trade_volume: Decimal,

	pub user_id: i64,     //用户id
	pub event_id: i64,    //市场id
	pub market_id: i16,   //选项id
	pub token_id: String, //token id

	pub side: OrderSide,       //买入或卖出
	pub avg_price: Decimal,    //成交价格
	pub usdc_amount: Decimal,  //usdc数量
	pub token_amount: Decimal, //token数量

	pub fee: Option<Decimal>, //手续费 手续费针对的永远是receiver
	pub real_amount: Decimal, //实际receive了多少

	pub onchain_send_handled: bool, //是否已经处理了链上发送
	pub tx_hash: Option<String>,    //交易hash
}

/// 活动类型
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "activity_type", rename_all = "snake_case")]
pub enum ActivityType {
	/// 买
	Buy,
	/// 卖
	Sell,
	/// 赎回
	Redeem,
	/// split
	Split,
	/// merge
	Merge,
}

/// 用户1155 token资产
/// 用户redeem会把redeemed变为true 在链上会把用户对应的2个token的余额全部burn掉
/// balance和fronzen_balance正常变化也正常显示不会莫名变为0 但是如果有redeemed了那么就等于不能操作这个资产了
/// redeemed之后 最终frozen_balance一定会变为0 因为不会允许token得操作了 链上操作也会失败
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct Positions {
	pub user_id: i64,
	pub event_id: Option<i64>, //usdc没有event_id 其他都有
	pub market_id: Option<i16>,
	pub token_id: String,
	pub balance: Decimal,
	pub frozen_balance: Decimal,
	pub usdc_cost: Option<Decimal>, //split/merge/交易 都有usdc的变化
	pub avg_price: Option<Decimal>, //avg_price = usdc_cost/(balance+frozen_balance)
	pub redeemed: Option<bool>,     //true就变成了closed_position 否则就是position持仓中
	pub redeemed_timestamp: Option<chrono::DateTime<Utc>>,
	pub payout: Option<Decimal>,      //赎回的金额
	pub privy_id: Option<String>,     // 用于websocket推送
	pub outcome_name: Option<String>, // 用于websocket推送
	pub update_id: i64,               // 用于websocket消息顺序保证，每次更新自增
	pub updated_at: chrono::DateTime<Utc>,
	pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct AssetHistory {
	pub id: i64,
	pub user_id: i64,
	pub history_type: AssetHistoryType,
	// USDC 相关字段
	pub usdc_amount: Option<Decimal>, // USDC 变化量，正数表示增加，负数表示减少，如果只有 token 变化则为 None
	pub usdc_balance_before: Option<Decimal>,
	pub usdc_balance_after: Option<Decimal>,
	pub usdc_frozen_balance_before: Option<Decimal>,
	pub usdc_frozen_balance_after: Option<Decimal>,
	// Token 相关字段
	pub token_id: Option<String>,      // Token ID，如果只有 USDC 变化则为 None
	pub token_amount: Option<Decimal>, // Token 变化量，正数表示增加，负数表示减少，如果只有 USDC 变化则为 None
	pub token_balance_before: Option<Decimal>,
	pub token_balance_after: Option<Decimal>,
	pub token_frozen_balance_before: Option<Decimal>,
	pub token_frozen_balance_after: Option<Decimal>,
	pub tx_hash: Option<String>,
	pub trade_id: Option<String>, // 交易 ID
	pub order_id: Option<String>, // 订单 ID
	pub created_at: chrono::DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[sqlx(type_name = "asset_history_type", rename_all = "snake_case")]
pub enum AssetHistoryType {
	/// 创建订单
	CreateOrder,
	///订单拒绝
	OrderRejected,
	/// 取消订单
	CancelOrder,
	/// 充值成功
	Deposit,
	/// 提现成功
	Withdraw,
	/// 买
	OnChainBuySuccess,
	/// 卖
	OnChainSellSuccess,
	/// 买失败
	OnChainBuyFailed,
	/// 卖失败
	OnChainSellFailed,
	/// 赎回
	Redeem,
	/// split
	Split,
	/// merge
	Merge,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct OperationHistory {
	pub id: i64,
	pub event_id: i64,
	pub market_id: i16,
	pub user_id: i64,
	pub history_type: AssetHistoryType,
	pub outcome_name: Option<String>,
	pub token_id: Option<String>,
	pub quantity: Option<Decimal>,
	pub price: Option<Decimal>,
	pub value: Option<Decimal>,
	pub tx_hash: String,
	pub created_at: chrono::DateTime<Utc>,
}
