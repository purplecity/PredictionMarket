use {
	common::engine_types::{OrderSide, OrderStatus, OrderType},
	rust_decimal::Decimal,
	serde::{Deserialize, Serialize},
	sqlx::FromRow,
	std::str::FromStr,
};

/// API 统一响应格式
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiResponse<T> {
	#[serde(rename = "code")]
	pub code: i32,
	#[serde(rename = "msg")]
	pub msg: String,
	#[serde(rename = "data")]
	pub data: Option<T>,
}

impl<T> ApiResponse<T> {
	/// 创建成功响应
	pub fn success(data: T) -> Self {
		Self { code: 0, msg: "success".to_string(), data: Some(data) }
	}

	/// 创建失败响应
	pub fn error(error_code: crate::api_error::ApiErrorCode) -> Self {
		Self { code: error_code.as_i32(), msg: error_code.message().to_string(), data: None }
	}

	/// 创建自定义错误响应
	pub fn customer_error(msg: String) -> Self {
		Self { code: crate::api_error::ApiErrorCode::CustomerError.as_i32(), msg, data: None }
	}
}

/// 用户图片更新请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserImageRequest {
	#[serde(rename = "image_url")]
	pub image_url: String,
	#[serde(rename = "image_type")]
	pub image_type: String,
}

impl UserImageRequest {
	/// 校验参数值
	/// - image_url 必须包含 "https://storage.googleapis.com" 字符串
	/// - image_type 必须是允许的字符串列表之一（当前只有 "header"）
	pub fn validate(&self) -> Result<(), String> {
		// 校验 image_url
		if !self.image_url.contains("https://storage.googleapis.com") {
			return Err("image_url must contain 'https://storage.googleapis.com'".to_string());
		}

		// 校验 image_type
		let valid_image_types = ["header"];
		if !valid_image_types.contains(&self.image_type.as_str()) {
			return Err(format!("image_type must be one of: {:?}", valid_image_types));
		}

		Ok(())
	}
}

/// 用户资料更新请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfileRequest {
	pub name: String,
	pub bio: String,
}

impl UserProfileRequest {
	/// 校验参数值
	/// - name 不为空且不超过 32 个字符
	/// - bio 不为空且不超过 128 个字符
	pub fn validate(&self) -> Result<(), String> {
		// 校验 name
		if self.name.is_empty() {
			return Err("name cannot be empty".to_string());
		}
		if self.name.len() > 32 {
			return Err("name cannot exceed 32 characters".to_string());
		}

		// 校验 bio
		if self.bio.is_empty() {
			return Err("bio cannot be empty".to_string());
		}
		if self.bio.len() > 128 {
			return Err("bio cannot exceed 128 characters".to_string());
		}

		Ok(())
	}
}

/// 下单请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaceOrderRequest {
	// SignatureOrderMsg fields
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

	// Additional fields
	pub event_id: i64,
	pub market_id: i16,
	pub price: String,
	pub order_type: String,
}

impl PlaceOrderRequest {
	/*
	校验请求参数,当前的约定是
	makerAmout永远最多2位小数.
	price区间是[0.001,0.999]最多4位小数
	然后takerAmount如果是买的话 = makerAmount/price 然后直接截断2位
	如果是卖的话takerAmount=price*makerAmount 不用截断
	*/
	pub fn validate(&self) -> Result<(), String> {
		// 1. 校验 side
		if self.side != common::consts::ORDER_SIDE_BUY && self.side != common::consts::ORDER_SIDE_SELL {
			return Err("side must be buy or sell".to_string());
		}

		// 2. 校验 order_type
		if self.order_type != common::consts::ORDER_TYPE_LIMIT && self.order_type != common::consts::ORDER_TYPE_MARKET {
			return Err("order_type must be limit or market".to_string());
		}

		// 3. 校验 price
		let price = Decimal::from_str(&self.price).map_err(|e| format!("Invalid price: {}", e))?;
		let min_price = Decimal::new(1, 3); // 0.001
		let max_price = Decimal::new(999, 3); // 0.999
		if price < min_price || price > max_price {
			return Err(format!("price must be between 0.001 and 0.999, got {}", price));
		}
		// 最多4位小数
		if price.scale() > 4 {
			return Err(format!("price can have at most 4 decimal places, got {}", price));
		}

		// 4. 校验 makerAmount 和 takerAmount
		let maker_amount = Decimal::from_str(&self.maker_amount).map_err(|e| format!("Invalid makerAmount: {}", e))?;
		let taker_amount = Decimal::from_str(&self.taker_amount).map_err(|e| format!("Invalid takerAmount: {}", e))?;

		// 校验都不能为 0
		if maker_amount.is_zero() {
			return Err("makerAmount must not be zero".to_string());
		}
		if taker_amount.is_zero() {
			return Err("takerAmount must not be zero".to_string());
		}

		// 校验币的数量 必须能被 10^16 整除
		let divisor_16 = Decimal::from(10u64.pow(16));
		if self.side == common::consts::ORDER_SIDE_SELL && maker_amount % divisor_16 != Decimal::ZERO {
			return Err(format!("token amount must be divisible by 10^16, got {}", maker_amount));
		} else if self.side == common::consts::ORDER_SIDE_BUY && taker_amount % divisor_16 != Decimal::ZERO {
			return Err(format!("token amount must be divisible by 10^16, got {}", taker_amount));
		}

		// 5. 根据 side 校验 takerAmount
		// 转换 makerAmount 为实际值（除以 10^18）
		let divisor_18 = Decimal::from(10u64.pow(18));
		let maker_amount_real = maker_amount.checked_div(divisor_18).ok_or("makerAmount division overflow".to_string())?;

		// 根据订单方向计算期望的 takerAmount
		let expected_taker_amount_real = if self.side == common::consts::ORDER_SIDE_BUY {
			// BUY: takerAmount = makerAmount / price，截断保留 2 位小数
			let calculated = maker_amount_real.checked_div(price).ok_or("Division overflow when calculating takerAmount".to_string())?;

			if self.order_type == common::consts::ORDER_TYPE_MARKET { calculated.trunc_with_scale(2).normalize() } else { calculated.normalize() }
		} else {
			// SELL: takerAmount = makerAmount * price，截断保留 2 位小数
			let calculated = maker_amount_real.checked_mul(price).ok_or("Multiplication overflow when calculating takerAmount".to_string())?;
			//calculated.trunc_with_scale(2).normalize()
			//卖的时候takerAmount=price*makerAmount 不用截断
			if calculated.scale() > 6 {
				return Err(format!("calculated  takerAmountcan have at most 6 decimal places, got {}", calculated));
			}
			calculated.normalize()
		};

		// 将期望的 takerAmount 转换回原始值（乘以 10^18）
		let expected_taker_amount = expected_taker_amount_real.checked_mul(divisor_18).ok_or("takerAmount multiplication overflow".to_string())?;

		// 校验 takerAmount 必须等于计算值
		if taker_amount != expected_taker_amount {
			return Err(format!(
				"takerAmount mismatch: expected {}, got {}. When side is {}, takerAmount must equal makerAmount {} price (market buy order rounded to 2 decimal places)",
				expected_taker_amount,
				taker_amount,
				self.side,
				if self.side == common::consts::ORDER_SIDE_BUY { "/" } else { "*" }
			));
		}

		Ok(())
	}
}

/// 下单响应
pub type PlaceOrderResponse = ApiResponse<String>;

/// 取消订单请求（无请求体，从路径或查询参数获取 order_id）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CancelOrderRequest {
	pub order_id: String,
}

/// 取消订单响应
pub type CancelOrderResponse = ApiResponse<()>;

/// 余额查询请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceRequest {
	pub uid: i64,
}

/// 余额查询响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BalanceResponse {
	#[serde(with = "rust_decimal::serde::str")]
	pub balance: Decimal,
}

/// 取消所有订单响应
pub type CancelAllOrdersResponse = ApiResponse<()>;

// ============ Query Types (from api_query) ============

/// 用户数据请求（可选邀请码参数）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDataRequest {
	pub invite_code: Option<String>,
}

/// 用户数据响应（只返回需要的字段）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserDataResponse {
	pub id: i64,
	#[serde(rename = "privy_id")]
	pub privy_id: String,
	#[serde(rename = "privy_evm_address")]
	pub privy_evm_address: String,
	#[serde(rename = "privy_email")]
	pub privy_email: String,
	#[serde(rename = "privy_x")]
	pub privy_x: String,
	#[serde(rename = "privy_x_image")]
	pub privy_x_image: String,
	pub name: String,
	pub bio: String,
	#[serde(rename = "profile_image")]
	pub profile_image: String,
	#[serde(rename = "invite_code")]
	pub invite_code: String,
}

impl From<common::model::Users> for UserDataResponse {
	fn from(user: common::model::Users) -> Self {
		Self {
			id: user.id,
			privy_id: user.privy_id.clone(),
			privy_evm_address: user.privy_evm_address.clone(),
			privy_email: user.privy_email,
			privy_x: user.privy_x,
			privy_x_image: user.privy_x_image,
			name: if user.name.is_empty() { user.privy_evm_address } else { user.name },
			bio: user.bio,
			profile_image: user.profile_image,
			invite_code: user.privy_id, // 邀请码即 privy_id
		}
	}
}

/// 邀请统计响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteStatsResponse {
	/// 邀请获得积分比例 (除以100)，如5表示5%
	pub invite_points_ratio: i32,
	/// 被邀请人boost倍数，如"1.50"表示1.5倍
	pub invitee_boost: String,
	/// 本赛季邀请获得积分
	pub invite_earned_points: i64,
	/// 被邀请人总交易量
	pub invitees_total_volume: String,
	/// 邀请人数
	pub invite_count: i64,
}

/// 积分排行榜响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardPointsResponse {
	pub leaderboard: Vec<LeaderboardPointsItem>,
	/// 当前用户排名，">200" 表示超过200名
	pub my_rank: Option<String>,
	/// 当前用户总积分
	pub my_total_points: Option<i64>,
}

/// 积分排行榜项目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardPointsItem {
	pub rank: i32,
	pub user_id: i64,
	pub name: String,
	pub profile_image: String,
	pub total_points: i64,
}

/// 交易量排行榜响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardVolumeResponse {
	pub leaderboard: Vec<LeaderboardVolumeItem>,
	/// 当前用户排名，">200" 表示超过200名
	pub my_rank: Option<String>,
	/// 当前用户累积交易量
	pub my_accumulated_volume: Option<String>,
}

/// 交易量排行榜项目
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardVolumeItem {
	pub rank: i32,
	pub user_id: i64,
	pub name: String,
	pub profile_image: String,
	pub accumulated_volume: String,
}

/// 排行榜请求参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaderboardRequest {
	/// 可选的赛季ID，不传则查询当前赛季
	pub season_id: Option<i32>,
}

/// 当前赛季信息响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentSeasonResponse {
	pub season_id: i32,
	pub name: String,
	/// 开始时间戳（秒）
	pub start_time: i64,
	/// 结束时间戳（秒）
	pub end_time: i64,
}

/// 赛季列表项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeasonItem {
	pub season_id: i32,
	/// 开始时间戳（秒）
	pub start_time: i64,
	/// 结束时间戳（秒）
	pub end_time: i64,
}

/// 所有赛季列表响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SeasonsListResponse {
	pub seasons: Vec<SeasonItem>,
}

/// 单条邀请记录
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteRecordItem {
	pub user_id: i64,
	pub name: String,
	pub profile_image: String,
	pub created_at: chrono::DateTime<chrono::Utc>,
}

/// 邀请记录列表响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InviteRecordsResponse {
	pub records: Vec<InviteRecordItem>,
}

/// 获取图片签名URL请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GetImageSignRequest {
	pub bucket_type: String,
	pub image_type: String,
}

impl GetImageSignRequest {
	pub fn validate(&self) -> Result<(), String> {
		let valid_image_types = ["jpg", "jpeg", "png", "gif", "webp"];
		let image_type_lower = self.image_type.to_lowercase();
		if !valid_image_types.contains(&image_type_lower.as_str()) {
			return Err(format!("image_type must be one of: {:?}", valid_image_types));
		}

		let valid_bucket_types = ["header"];
		if !valid_bucket_types.contains(&self.bucket_type.as_str()) {
			return Err(format!("bucket_type must be one of: {:?}", valid_bucket_types));
		}

		Ok(())
	}
}

// 市场列表请求和响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsRequest {
	pub ending_soon: Option<bool>,
	pub newest: Option<bool>,
	pub topic: Option<String>,
	pub title: Option<String>,
	pub volume: Option<bool>,
	pub page: i16,
	pub page_size: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventsResponse {
	pub events: Vec<SingleEventResponse>,
	pub total: i16,
	pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleEventResponse {
	pub event_id: i64,
	pub slug: String,
	pub image: String,
	pub title: String,
	#[serde(with = "rust_decimal::serde::str")]
	pub volume: Decimal,
	pub topic: String,
	pub markets: Vec<EventMarketResponse>,
	pub has_streamer: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMarketResponse {
	pub market_id: i16,
	pub title: String,
	pub question: String,
	pub outcome_0_name: String,
	pub outcome_1_name: String,
	pub outcome_0_token_id: String,
	pub outcome_1_token_id: String,
	pub outcome_0_chance: String,
	pub outcome_1_chance: String,
	pub outcome_0_best_bid: String,
	pub outcome_0_best_ask: String,
	pub outcome_1_best_bid: String,
	pub outcome_1_best_ask: String,
}

// 市场详情请求和响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDetailRequest {
	pub event_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventDetailResponse {
	pub event_id: i64,
	pub slug: String,
	pub image: String,
	pub title: String,
	pub rules: String,
	#[serde(with = "rust_decimal::serde::str")]
	pub volume: Decimal,
	pub endtime: i64,
	pub starttime: i64,
	pub closed: bool,
	pub resolved: bool,
	pub markets: Vec<EventMarketDetail>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventMarketDetail {
	pub title: String,
	pub question: String,
	pub image: String,
	pub market_id: i16,
	#[serde(with = "rust_decimal::serde::str")]
	pub volume: Decimal,
	pub condition_id: String,
	pub parent_collection_id: String,
	pub outcome_0_name: String,
	pub outcome_1_name: String,
	pub outcome_0_token_id: String,
	pub outcome_1_token_id: String,
	#[serde(with = "rust_decimal::serde::str")]
	pub outcome_0_chance: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub outcome_1_chance: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub outcome_0_best_bid: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub outcome_0_best_ask: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub outcome_1_best_bid: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub outcome_1_best_ask: Decimal,
	pub closed: bool,
	pub winner_outcome_name: String,
	pub winner_outcome_token_id: String,
}

// Event Streamers 接口请求和响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventStreamersRequest {
	pub event_id: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventStreamersResponse {
	pub streamers: Vec<StreamerWithViewerCount>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamerWithViewerCount {
	pub streamer_uid: String,
	pub viewer_count: i32,
	pub live_room: LiveRoomRow,
	pub user: Option<StreamerUserInfo>,
}

/// Streamer 用户信息（简化版，只包含必要字段）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamerUserInfo {
	pub privy_id: String,
	pub name: String,
	pub profile_image: String,
}

impl StreamerUserInfo {
	/// 从 Users 模型转换为 StreamerUserInfo
	pub fn from_user(user: &common::model::Users) -> Self {
		Self { privy_id: user.privy_id.clone(), name: user.name.clone(), profile_image: user.profile_image.clone() }
	}
}

// Streamer Viewer Count 接口请求和响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamerViewerCountRequest {
	pub streamer_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamerViewerCountResponse {
	pub viewer_count: i32,
}

/// 用户信息（来自 Users 表）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
	pub id: i64,
	pub privy_id: String,
	pub privy_evm_address: String,
	pub privy_email: String,
	pub privy_x: String,
	pub privy_x_image: String,
	pub name: String,
	pub bio: String,
	pub profile_image: String,
	pub last_login_ip: String,
	pub last_login_region: String,
	pub last_login_at: i64,
	pub registered_ip: String,
	pub registered_region: String,
	pub created_at: i64,
}

impl UserInfo {
	/// 从 Users 模型转换为 UserInfo
	pub fn from_user(user: &common::model::Users) -> Self {
		Self {
			id: user.id,
			privy_id: user.privy_id.clone(),
			privy_evm_address: user.privy_evm_address.clone(),
			privy_email: user.privy_email.clone(),
			privy_x: user.privy_x.clone(),
			privy_x_image: user.privy_x_image.clone(),
			name: user.name.clone(),
			bio: user.bio.clone(),
			profile_image: user.profile_image.clone(),
			last_login_ip: user.last_login_ip.clone(),
			last_login_region: user.last_login_region.clone(),
			last_login_at: user.last_login_at.timestamp(),
			registered_ip: user.registered_ip.clone(),
			registered_region: user.registered_region.clone(),
			created_at: user.created_at.timestamp(),
		}
	}
}

// Lives 接口请求和响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LivesRequest {
	pub ending_soon: Option<bool>,
	pub newest: Option<bool>,
	pub topic: Option<String>,
	pub volume: Option<bool>,
	pub page: i16,
	pub page_size: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LivesResponse {
	pub streamers: Vec<StreamerEventResponse>,
	pub total: i16,
	pub has_more: bool,
}

/// Streamer 和对应的 Event 信息
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamerEventResponse {
	pub streamer_uid: String,
	pub live_room: LiveRoomRow,
	pub user: Option<StreamerUserInfo>,
	pub event: SingleEventResponse,
}

/// PostgreSQL live_rooms 表的行结构
#[derive(Debug, Clone, Serialize, Deserialize, FromRow)]
pub struct LiveRoomRow {
	pub id: String,
	pub room_id: Option<String>,
	pub room_name: Option<String>,
	pub room_desc: Option<String>,
	pub status: Option<String>,
	pub event_id: Option<String>,
	#[serde(with = "chrono::serde::ts_milliseconds_option")]
	pub create_date: Option<chrono::DateTime<chrono::Utc>>,
	pub picture_url: Option<String>,
	#[serde(with = "chrono::serde::ts_milliseconds_option")]
	pub created_at: Option<chrono::DateTime<chrono::Utc>>,
	#[serde(with = "chrono::serde::ts_milliseconds_option")]
	pub updated_at: Option<chrono::DateTime<chrono::Utc>>,
}

// Streamer Detail 接口请求和响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamerDetailRequest {
	pub streamer_uid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamerDetailResponse {
	pub streamer_uid: String,
	pub live_room: Option<LiveRoomRow>,
	pub user: Option<StreamerUserInfo>,
	pub event: Option<SingleEventResponse>,
}

/// 投资组合价值请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioValueRequest {
	pub uid: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioValueResponse {
	#[serde(with = "rust_decimal::serde::str")]
	pub value: Decimal,
}

/// 交易量请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradedVolumeRequest {
	pub uid: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TradedVolumeResponse {
	#[serde(with = "rust_decimal::serde::str")]
	pub volume: Decimal,
}

/*
又是这样的问题 动态计算+分页同时存在.
这在数据小的时候还可以一次性拿到后端计算然后给到前端
在数据量大的时候就直接不返回数据给前端 告诉前端必须指定条件了 告诉前端不支持动态分页这种
如果不是动态的分页 而且页数也比较大那么就采取按照限制值分页 简单来说就是用where取代offset避免全表扫描
不管怎样条数都搞大点直接500-1000条
*/

// 持仓请求和响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionRequest {
	pub uid: i64,
	pub event_id: Option<i64>,
	pub market_id: Option<i16>,
	pub question: Option<String>,
	pub value: Option<bool>,
	pub quantity: Option<bool>,
	pub avg_price: Option<bool>,
	pub profit_value: Option<bool>,
	pub profit_percentage: Option<bool>,
	pub page: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PositionsResponse {
	pub positions: Vec<SinglePositionResponse>,
	pub total: i16,
	pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SinglePositionResponse {
	pub event_id: i64,
	pub market_id: i16,
	pub event_title: String,
	pub market_title: String,
	pub market_question: String,
	pub event_image: String,
	pub market_image: String,
	pub outcome_name: String,
	pub condition_id: String,
	pub token_id: String,
	#[serde(with = "rust_decimal::serde::str")]
	pub avg_price: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub quantity: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub value: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub profit_value: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub profit_percentage: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub current_price: Decimal,
}

// 已平仓请求和响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedPositionsRequest {
	pub uid: i64,
	pub event_id: Option<i64>,
	pub market_id: Option<i16>,
	pub question: Option<String>,
	pub avg_price: Option<bool>,
	pub profit_value: Option<bool>,
	pub profit_percentage: Option<bool>,
	pub redeem_timestamp: Option<bool>,
	pub page: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClosedPositionsResponse {
	pub positions: Vec<SingleClosedPositionResponse>,
	pub total: i16,
	pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleClosedPositionResponse {
	pub event_id: i64,
	pub market_id: i16,
	pub event_title: String,
	pub market_title: String,
	pub market_question: String,
	pub event_image: String,
	pub market_image: String,
	pub outcome_name: String,
	pub token_id: String,
	#[serde(with = "rust_decimal::serde::str")]
	pub avg_price: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub quantity: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub value: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub payout: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub profit_value: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub profit_percentage: Decimal,
	pub redeem_timestamp: i64,
	pub win: bool,
}

//没有过滤条件 每一条都有交易hash
// 活动请求和响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityRequest {
	pub uid: i64,
	pub event_id: Option<i64>,
	pub market_id: Option<i16>,
	pub page: i16,
	pub page_size: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityResponse {
	pub activities: Vec<SingleActivityResponse>,
	pub total: i16,
	pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleActivityResponse {
	pub event_id: i64,
	pub market_id: i16,
	pub event_title: String,
	pub market_title: String,
	pub market_question: String,
	pub event_image: String,
	pub market_image: String,
	pub types: String,
	pub timestamp: i64,
	pub outcome_name: Option<String>,
	#[serde(with = "rust_decimal::serde::str_option")]
	pub price: Option<Decimal>,
	#[serde(with = "rust_decimal::serde::str")]
	pub quantity: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub value: Decimal,
	pub tx_hash: String,
}

// 没有分页条件
// 未成交订单请求和响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenOrdersRequest {
	pub uid: i64,
	pub event_id: Option<i64>,
	pub market_id: Option<i16>,
	pub page: i16,
	pub page_size: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenOrdersResponse {
	pub orders: Vec<SingleOpenOrderResponse>,
	pub total: i16,
	pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleOpenOrderResponse {
	pub event_id: i64,
	pub market_id: i16,
	pub event_title: String,
	pub market_title: String,
	pub market_question: String,
	pub event_image: String,
	pub market_image: String,
	pub order_id: String,
	pub side: String,
	pub outcome_name: String,
	#[serde(with = "rust_decimal::serde::str")]
	pub price: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub quantity: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub filled_quantity: Decimal,
	#[serde(with = "rust_decimal::serde::str")]
	pub volume: Decimal,
	pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthRequest {
	pub event_id: i64,
	pub market_id: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthResponse {
	pub update_id: u64,
	pub timestamp: i64,
	pub depths: std::collections::HashMap<String, DepthBook>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DepthBook {
	pub latest_trade_price: String,
	pub bids: Vec<common::websocket_types::PriceLevelInfo>,
	pub asks: Vec<common::websocket_types::PriceLevelInfo>,
}

/// 事件余额请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBalanceRequest {
	pub event_id: i64,
	pub market_id: Option<i16>,
}

/// 事件余额响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBalanceResponse {
	pub token_available: std::collections::HashMap<String, String>,
	#[serde(with = "rust_decimal::serde::str")]
	pub cash_available: Decimal,
}

//订单委托历史
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderHistoryRequest {
	pub event_id: Option<i64>,
	pub market_id: Option<i16>,
	pub page: i16,
	pub page_size: i16,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OrderHistoryResponse {
	pub order_history: Vec<SingleOrderHistoryResponse>,
	pub total: i16,
	pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SingleOrderHistoryResponse {
	pub order_id: String,        //订单id
	pub event_title: String,     //事件标题
	pub event_image: String,     //事件图片
	pub market_title: String,    //市场标题
	pub market_question: String, //市场问题
	pub market_image: String,    //市场图片
	pub token_id: String,        //token id
	pub outcome: String,         //outcome名字
	pub order_side: OrderSide,   //买入或卖出
	pub order_type: OrderType,   //限价单或市价单
	#[serde(with = "rust_decimal::serde::str")]
	pub price: Decimal, //就算是市价单也有价格
	#[serde(with = "rust_decimal::serde::str")]
	pub quantity: Decimal, //提交的数量 永远是token的数量 而不是usdc
	#[serde(with = "rust_decimal::serde::str")]
	pub volume: Decimal, //用以冻结usdc 要自己算下等于price*quantity 前端不会传volume 但是不管是限价单还是市价单一定会有price 市价单没吃完会直接取消
	#[serde(with = "rust_decimal::serde::str")]
	pub filled_quantity: Decimal, //已成交的数量 永远是token的数量 而不是usdc
	#[serde(with = "rust_decimal::serde::str")]
	pub cancelled_quantity: Decimal, //已取消的数量 永远是token的数量 而不是usdc
	pub status: OrderStatus,     //订单状态
	pub created_at: i64,         //创建时间
	pub updated_at: i64,         //更新时间
}

/// 用户积分查询请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointsRequest {
	pub uid: i64,
}

/// 用户积分查询响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PointsResponse {
	pub boost_points: i64,
	pub invite_points: i64,
	pub total_points: i64,
	/// 排名，前200名显示具体名次，否则为 ">200"
	pub rank: String,
}

/// 用户简单信息查询请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleInfoRequest {
	pub uid: i64,
}

/// 用户简单信息查询响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimpleInfoResponse {
	pub name: String,
	pub profile_image: String,
}

/// 用户赛季积分交易量查询请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPointsVolumesRequest {
	pub season_id: Option<i32>,
}

/// 用户赛季积分交易量查询响应
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPointsVolumesResponse {
	pub total_points: i64,
	#[serde(with = "rust_decimal::serde::str")]
	pub accumulated_volume: Decimal,
}
