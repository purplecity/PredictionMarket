use serde::{Deserialize, Serialize};

/// 市场创建
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventCreate {
	pub event_identifier: String,  //event的唯一标识
	pub slug: String,              //标识符 title小写替换空格替换左斜线
	pub title: String,             //标题
	pub description: String,       //描述
	pub image: String,             //图片地址
	pub end_date: Option<i64>,     //时间戳 秒
	pub topic: String,             //主题类别 crypto sport等等
	pub markets: Vec<EventMarket>, //数组
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMarket {
	pub parent_collection_id: String,
	pub condition_id: String,
	pub market_identifier: String, //市场唯一标识符 不必全局唯一 用来区别在事件内区分不同市场
	pub question: String,          //问题
	pub slug: String,              //标识符 title小写替换空格替换左斜线
	pub title: String,             //标题
	pub image: Option<String>,     //图片地址
	#[serde(rename = "outcomeNames")]
	pub outcome_names: Vec<String>, //结果名称列表
	#[serde(rename = "tokenIds")]
	pub token_ids: Vec<String>, //结果token id列表
}

impl EventMarket {
	/// 验证市场数据
	pub fn validate(&self) -> Result<(), String> {
		// 验证 outcome_names 长度必须为 2
		if self.outcome_names.len() != 2 {
			return Err(format!("outcome_names must have exactly 2 elements, got {}", self.outcome_names.len()));
		}

		// 验证 token_ids 长度必须为 2
		if self.token_ids.len() != 2 {
			return Err(format!("token_ids must have exactly 2 elements, got {}", self.token_ids.len()));
		}

		// 验证 outcome_names 中两个元素不能相等
		if self.outcome_names[0] == self.outcome_names[1] {
			return Err(format!("outcome_names elements must be different, got duplicate: {}", self.outcome_names[0]));
		}

		// 验证 token_ids 中两个元素不能相等
		if self.token_ids[0] == self.token_ids[1] {
			return Err(format!("token_ids elements must be different, got duplicate: {}", self.token_ids[0]));
		}

		Ok(())
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventClose {
	pub event_identifier: String,          //市场的唯一标识
	pub markets_result: Vec<MarketResult>, //市场选项结果列表
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketResult {
	pub market_identifier: String,    //选项的唯一标识
	pub win_outcome_token_id: String, //赢的结果代币地址
	pub win_outcome_name: String,     //赢的结果名称
}

/// 单个市场添加（在已有Event下添加Market）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketAdd {
	pub event_identifier: String, //event的唯一标识
	pub market: EventMarket,      //要添加的市场
}

/// 单个市场关闭
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarketClose {
	pub event_identifier: String,    //event的唯一标识
	pub market_result: MarketResult, //市场结果
}
