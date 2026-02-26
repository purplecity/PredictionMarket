use {
	crate::{
		api_error::{ApiErrorCode, InternalError},
		api_types::{
			ApiResponse, BalanceRequest, BalanceResponse, CancelAllOrdersResponse, CancelOrderResponse, CurrentSeasonResponse, EventDetailRequest, EventDetailResponse, EventMarketDetail,
			EventMarketResponse, EventStreamersRequest, EventStreamersResponse, EventsRequest, EventsResponse, GetImageSignRequest, InviteRecordItem, InviteRecordsResponse, InviteStatsResponse,
			LeaderboardPointsItem, LeaderboardPointsResponse, LeaderboardRequest, LeaderboardVolumeItem, LeaderboardVolumeResponse, LivesRequest, LivesResponse, PlaceOrderRequest, PlaceOrderResponse,
			PointsRequest, PointsResponse, SeasonItem, SeasonsListResponse, SimpleInfoRequest, SimpleInfoResponse, SingleEventResponse, StreamerDetailRequest, StreamerDetailResponse,
			StreamerEventResponse, StreamerUserInfo, StreamerViewerCountRequest, StreamerViewerCountResponse, StreamerWithViewerCount, UserDataRequest, UserDataResponse, UserImageRequest,
			UserPointsVolumesRequest, UserPointsVolumesResponse, UserProfileRequest,
		},
		cache::{self, CacheError},
		consts::{DEFAULT_INVITE_POINTS_RATIO, DEFAULT_INVITEE_BOOST, LEADERBOARD_MAX_DISPLAY},
		db,
		internal_service::{
			ReqEip712OrderVerify, ReqGetGoogleImageSign, ReqGetIpInfo, ReqGetPrivyUserInfo, ResGetGoogleImageSign, SignOrder as InternalSignOrder, get_google_image_sign, get_ip_info,
			get_privy_user_info, verify_eip712_order,
		},
		rpc_chat_service, rpc_client,
		server::ClientInfo,
		singleflight::{
			self, get_event_detail_group, get_event_streamers_group, get_events_group, get_lives_group, get_market_validation_group, get_points_group, get_simple_info_group,
			get_streamer_detail_group, get_streamer_viewer_count_group, get_topics_group, get_user_points_volumes_group,
		},
	},
	axum::{
		extract::{Extension, Query},
		response::Json,
	},
	chrono::Utc,
	common::{
		consts::{AIRDROP_MSG_KEY, AIRDROP_STREAM, NEW_USER_MSG_KEY, NEW_USER_STREAM, ORDER_INPUT_MSG_KEY, ORDER_INPUT_STREAM},
		depth_types::CacheMarketPriceInfo,
		engine_types::{CancelOrderMessage, OrderInputMessage, OrderSide as EngineOrderSide, OrderStatus, OrderType, PredictionSymbol, SubmitOrderMessage},
		key::{PRICE_CACHE_KEY, market_field},
		model::{Events, SignatureOrderMsg, Users},
		redis_pool,
	},
	redis::AsyncCommands,
	rust_decimal::{Decimal, prelude::ToPrimitive},
	std::{str::FromStr, sync::Arc},
	tracing::info,
};

/// 认证结果
pub struct AuthResult {
	pub user_id: i64,
	pub privy_id: String,
	pub is_api_key_user: bool,
}

/// 认证错误类型
pub enum AuthError {
	AuthFailed,
	UserNotFound,
	Internal,
}

/// 从 ClientInfo 中获取认证信息
/// 优先使用 privy_id，如果没有则尝试使用 api_key
/// 返回 user_id, privy_id 和是否为 api_key 用户
pub async fn get_user_id_from_auth(client_info: &ClientInfo, request_id: &str) -> Result<AuthResult, AuthError> {
	// 1. 优先尝试 privy_id
	if let Some(privy_id) = &client_info.privy_id {
		match cache::get_user_id_by_privy_id(privy_id).await {
			Ok(user_id) => {
				return Ok(AuthResult { user_id, privy_id: privy_id.clone(), is_api_key_user: false });
			}
			Err(CacheError::UserNotFound(_)) => {
				return Err(AuthError::UserNotFound);
			}
			Err(e) => {
				tracing::error!("request_id={}, privy_id={} - Failed to get user id from cache: {}", request_id, privy_id, e);
				return Err(AuthError::Internal);
			}
		}
	}

	// 2. 尝试 api_key
	if !client_info.api_key.is_empty() {
		match cache::get_user_id_and_privy_id_by_api_key(&client_info.api_key).await {
			Ok((user_id, privy_id)) => {
				return Ok(AuthResult { user_id, privy_id, is_api_key_user: true });
			}
			Err(CacheError::UserNotFound(_)) => {
				return Err(AuthError::AuthFailed);
			}
			Err(e) => {
				tracing::error!("request_id={}, api_key={} - Failed to get user id from cache: {}", request_id, client_info.api_key, e);
				return Err(AuthError::Internal);
			}
		}
	}

	// 3. 都没有，返回认证失败
	Err(AuthError::AuthFailed)
}

/// 将 AuthError 转换为 ApiErrorCode
impl AuthError {
	pub fn to_api_error_code(&self) -> ApiErrorCode {
		match self {
			AuthError::AuthFailed => ApiErrorCode::AuthFailed,
			AuthError::UserNotFound => ApiErrorCode::UserNotFound,
			AuthError::Internal => ApiErrorCode::InternalError,
		}
	}
}

/// 更新用户头像
pub async fn handle_user_image(Extension(client_info): Extension<ClientInfo>, Json(params): Json<UserImageRequest>) -> Result<Json<ApiResponse<()>>, InternalError> {
	let auth_result = match get_user_id_from_auth(&client_info, &client_info.request_id).await {
		Ok(result) => result,
		Err(e) => return Ok(Json(ApiResponse::error(e.to_api_error_code()))),
	};
	let user_id = auth_result.user_id;

	if let Err(err_msg) = params.validate() {
		return Ok(Json(ApiResponse::customer_error(err_msg)));
	}

	let write_pool = db::get_db_write_pool().map_err(|e| {
		tracing::error!("Failed to get db write pool: {}", e);
		InternalError
	})?;

	sqlx::query("UPDATE users SET profile_image = $1 WHERE id = $2").bind(&params.image_url).bind(user_id).execute(&write_pool).await.map_err(|e| {
		tracing::error!("Failed to update user profile image: {}", e);
		InternalError
	})?;

	Ok(Json(ApiResponse::success(())))
}

/// 更新用户资料
pub async fn handle_user_profile(Extension(client_info): Extension<ClientInfo>, Json(params): Json<UserProfileRequest>) -> Result<Json<ApiResponse<()>>, InternalError> {
	let auth_result = match get_user_id_from_auth(&client_info, &client_info.request_id).await {
		Ok(result) => result,
		Err(e) => return Ok(Json(ApiResponse::error(e.to_api_error_code()))),
	};
	let user_id = auth_result.user_id;

	if let Err(err_msg) = params.validate() {
		return Ok(Json(ApiResponse::customer_error(err_msg)));
	}

	let write_pool = db::get_db_write_pool().map_err(|e| {
		tracing::error!("Failed to get db write pool: {}", e);
		InternalError
	})?;

	sqlx::query("UPDATE users SET name = $1, bio = $2 WHERE id = $3").bind(&params.name).bind(&params.bio).bind(user_id).execute(&write_pool).await.map_err(|e| {
		tracing::error!("Failed to update user profile: {}", e);
		InternalError
	})?;

	Ok(Json(ApiResponse::success(())))
}

/// 解析订单金额参数（已通过 validate 校验）
fn parse_amounts(params: &PlaceOrderRequest) -> Result<(Decimal, Decimal, Decimal), String> {
	let divisor = Decimal::from(10u64.pow(18));

	let price = Decimal::from_str(&params.price).map_err(|_| "Invalid price format".to_string())?;
	let maker_amount = Decimal::from_str(&params.maker_amount).map_err(|_| "Invalid maker_amount format".to_string())?.checked_div(divisor).ok_or("maker_amount division overflow".to_string())?;
	let taker_amount = Decimal::from_str(&params.taker_amount).map_err(|_| "Invalid taker_amount format".to_string())?.checked_div(divisor).ok_or("taker_amount division overflow".to_string())?;

	Ok((price, maker_amount, taker_amount))
}

/// 计算订单的 token_amount 和 volume
fn calculate_token_amount_and_volume(side: &str, price: Decimal, maker_amount: Decimal, taker_amount: Decimal) -> Result<(Decimal, Decimal), String> {
	if side == common::consts::ORDER_SIDE_BUY {
		let volume = price.checked_mul(taker_amount).ok_or("Volume calculation overflow".to_string())?;
		Ok((taker_amount, volume))
	} else {
		let volume = price.checked_mul(maker_amount).ok_or("Volume calculation overflow".to_string())?;
		Ok((maker_amount, volume))
	}
}

/// 转换金额为引擎所需的整数格式
fn convert_to_engine_format(token_amount: Decimal, price: Decimal, _request_id: &str, _privy_id: &str, _order_id: &str) -> Result<(u64, i32), String> {
	// 转换 quantity（保留 2 位小数，乘以 100）
	let quantity_u64 = token_amount.checked_mul(Decimal::ONE_HUNDRED).ok_or("Token amount checked_mul overflow".to_string())?.to_u64().ok_or("Token amount conversion to u64 failed".to_string())?;

	// 转换 price（保留 4 位小数，乘以 10000）
	let price_i32 = price.checked_mul(Decimal::from(common::consts::PRICE_MULTIPLIER)).ok_or("Price checked_mul overflow".to_string())?.to_i32().ok_or("Price conversion to i32 failed".to_string())?;

	Ok((quantity_u64, price_i32))
}

/// 下单
pub async fn handle_place_order(Extension(client_info): Extension<ClientInfo>, Json(params): Json<PlaceOrderRequest>) -> Result<Json<PlaceOrderResponse>, InternalError> {
	let auth_result = match get_user_id_from_auth(&client_info, &client_info.request_id).await {
		Ok(result) => result,
		Err(e) => return Ok(Json(ApiResponse::error(e.to_api_error_code()))),
	};
	let user_id = auth_result.user_id;
	let privy_id = auth_result.privy_id;

	if let Err(err_msg) = params.validate() {
		return Ok(Json(ApiResponse::customer_error(err_msg)));
	}

	// 验证订单签名
	let side_int = if params.side == common::consts::ORDER_SIDE_BUY { 0 } else { 1 };
	let verify_req = ReqEip712OrderVerify {
		signature: params.signature.clone(),
		chain_id: common::consts::get_chain_id(),
		order: InternalSignOrder {
			salt: params.salt.to_string(),
			maker: params.maker.clone(),
			signer: params.signer.clone(),
			taker: params.taker.clone(),
			token_id: params.token_id.clone(),
			maker_amount: params.maker_amount.clone(),
			taker_amount: params.taker_amount.clone(),
			expiration: params.expiration.clone(),
			nonce: params.nonce.clone(),
			fee_rate_bps: params.fee_rate_bps.clone(),
			side: side_int,
			signature_type: params.signature_type as i32,
		},
	};

	match verify_eip712_order(verify_req).await {
		Ok(res) => {
			if !res.valid {
				return Ok(Json(ApiResponse::error(ApiErrorCode::SignatureInvalid)));
			}
		}
		Err(e) => {
			tracing::error!("request_id={}, privy_id={} - Failed to verify eip712 order signature: {}", client_info.request_id, privy_id, e);
			return Err(InternalError);
		}
	}

	// 打印请求参数日志
	info!("[PlaceOrder] request_id={}, privy_id={}, params={:?}", client_info.request_id, privy_id, params);

	// 2. 获取市场信息并检查是否关闭（使用 singleflight 避免重复查询）
	let singleflight_key = format!("{}|{}", params.event_id, params.market_id);
	let market_result = get_market_validation_group()
		.await
		.work(&singleflight_key, async {
			// 这里会检查缓存、数据库以及market的closed状态
			cache::get_market_and_check_closed(params.event_id, params.market_id).await
		})
		.await;

	let market = match market_result {
		Ok(m) => m,
		Err(Some(e)) => {
			// Leader failed - 直接匹配 CacheError 枚举
			match e {
				CacheError::EventNotFoundOrClosed(_) => {
					return Ok(Json(ApiResponse::error(ApiErrorCode::EventNotFoundOrClosed)));
				}
				CacheError::EventExpired(_) => {
					return Ok(Json(ApiResponse::error(ApiErrorCode::EventExpired)));
				}
				CacheError::MarketNotFoundOrClosed(..) => {
					return Ok(Json(ApiResponse::error(ApiErrorCode::MarketNotFoundOrClosed)));
				}
				_ => {
					tracing::error!("request_id={}, privy_id={} - Failed to get market from cache: {}", client_info.request_id, privy_id, e);
					return Err(InternalError);
				}
			}
		}
		Err(None) => {
			// Leader dropped
			tracing::error!("request_id={}, privy_id={} - Singleflight leader dropped", client_info.request_id, privy_id);
			return Err(InternalError);
		}
	};

	// 3. 验证 token_id 并获取 outcome_name
	let token_index = match market.outcome_token_ids.iter().position(|id| id == &params.token_id) {
		Some(idx) => idx,
		None => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::TokenIdNotFound)));
		}
	};

	let outcome_name = match market.outcome_names.get(token_index) {
		Some(name) => name,
		None => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::TokenIdNotFound)));
		}
	};

	// 4. 解析金额参数（已通过 validate 校验）
	let (price, maker_amount, taker_amount) = match parse_amounts(&params) {
		Ok(amounts) => amounts,
		Err(err_msg) => {
			tracing::error!("request_id={}, privy_id={} - Failed to parse amounts: {}", client_info.request_id, privy_id, err_msg);
			return Err(InternalError);
		}
	};

	// 5. 计算 token_amount 和 volume
	let (token_amount, volume) = match calculate_token_amount_and_volume(&params.side, price, maker_amount, taker_amount) {
		Ok(result) => result,
		Err(err_msg) => {
			return Ok(Json(ApiResponse::customer_error(err_msg)));
		}
	};

	let symbol = PredictionSymbol { event_id: params.event_id, market_id: params.market_id, token_id: params.token_id.clone() };
	let (proto_side, engine_side) =
		if params.side == common::consts::ORDER_SIDE_BUY { (common::consts::ORDER_SIDE_BUY, EngineOrderSide::Buy) } else { (common::consts::ORDER_SIDE_SELL, EngineOrderSide::Sell) };
	let (proto_order_type, engine_order_type) =
		if params.order_type == common::consts::ORDER_TYPE_LIMIT { (common::consts::ORDER_TYPE_LIMIT, OrderType::Limit) } else { (common::consts::ORDER_TYPE_MARKET, OrderType::Market) };

	let signature_order_msg = SignatureOrderMsg {
		expiration: params.expiration.clone(),
		fee_rate_bps: params.fee_rate_bps.clone(),
		maker: params.maker.clone(),
		maker_amount: params.maker_amount.clone(),
		nonce: params.nonce.clone(),
		salt: params.salt,
		side: proto_side.to_string(),
		signature: params.signature.clone(),
		signature_type: params.signature_type,
		signer: params.signer.clone(),
		taker: params.taker.clone(),
		taker_amount: params.taker_amount.clone(),
		token_id: params.token_id.clone(),
	};

	// 根据订单类型决定 gRPC 请求的 quantity 和 volume
	let (rpc_quantity, rpc_volume) = match engine_order_type {
		OrderType::Limit => {
			// 限价单：正常传递
			match engine_side {
				EngineOrderSide::Buy => (token_amount.to_string(), volume.to_string()),
				EngineOrderSide::Sell => (token_amount.to_string(), "0".to_string()),
			}
		}
		OrderType::Market => {
			// 市价单：根据买卖方向设置
			match engine_side {
				EngineOrderSide::Buy => {
					// 市价买单：quantity=0, volume=maker_amount
					("0".to_string(), maker_amount.to_string())
				}
				EngineOrderSide::Sell => {
					// 市价卖单：quantity=maker_amount, volume=0
					(maker_amount.to_string(), "0".to_string())
				}
			}
		}
	};

	let create_order_req = proto::CreateOrderRequest {
		user_id,
		event_id: params.event_id,
		market_id: params.market_id as i32,
		outcome: outcome_name.clone(),
		token_id: params.token_id.clone(),
		order_side: proto_side.to_string(),
		order_type: proto_order_type.to_string(),
		price: params.price.clone(),
		quantity: rpc_quantity,
		volume: rpc_volume,
		signature_order_msg: Some(proto::SignatureOrderMsg {
			expiration: signature_order_msg.expiration.clone(),
			fee_rate_bps: signature_order_msg.fee_rate_bps.clone(),
			maker: signature_order_msg.maker.clone(),
			maker_amount: signature_order_msg.maker_amount.clone(),
			nonce: signature_order_msg.nonce.clone(),
			salt: signature_order_msg.salt,
			side: signature_order_msg.side.clone(),
			signature: signature_order_msg.signature.clone(),
			signature_type: signature_order_msg.signature_type as i32,
			signer: signature_order_msg.signer.clone(),
			taker: signature_order_msg.taker.clone(),
			taker_amount: signature_order_msg.taker_amount.clone(),
			token_id: signature_order_msg.token_id.clone(),
		}),
	};

	let create_order_resp = rpc_client::create_order(create_order_req).await.map_err(|e| {
		tracing::error!("request_id={}, privy_id={} - Asset RPC CreateOrder failed: {}", client_info.request_id, privy_id, e);
		InternalError
	})?;

	if !create_order_resp.success {
		return Ok(Json(ApiResponse::customer_error(create_order_resp.reason)));
	}

	let order_id = create_order_resp.order_id;
	info!("request_id={}, privy_id={}, order_id={} - Order created successfully", client_info.request_id, privy_id, order_id);

	// 6. 转换为引擎格式
	let (quantity_u64, price_i32) = match convert_to_engine_format(token_amount, price, &client_info.request_id, &privy_id, &order_id) {
		Ok(result) => result,
		Err(err_msg) => {
			tracing::error!("{} request_id={}, privy_id={}, order_id={} - {}", crate::consts::LOG_ORDER_NOT_PUSHED, client_info.request_id, privy_id, order_id, err_msg);
			return Ok(Json(ApiResponse::error(ApiErrorCode::InvalidParameter)));
		}
	};

	// 7. 根据订单类型计算 volume_decimal 和 quantity_u64
	// volume 不乘以精度，就是实际的 USDC 值
	let (final_quantity_u64, volume_decimal) = if engine_order_type == OrderType::Market {
		// 市价单
		match engine_side {
			EngineOrderSide::Buy => {
				// 市价买单：quantity=0, volume=maker_amount（不乘以精度）
				(0u64, maker_amount)
			}
			EngineOrderSide::Sell => {
				// 市价卖单：quantity=已算好的quantity_u64, volume=0
				(quantity_u64, Decimal::ZERO)
			}
		}
	} else {
		match engine_side {
			EngineOrderSide::Buy => {
				// 限价单：保持当前逻辑，volume 不乘以精度
				(quantity_u64, volume)
			}
			EngineOrderSide::Sell => (quantity_u64, Decimal::ZERO),
		}
	};

	let submit_order_msg = SubmitOrderMessage {
		order_id: order_id.clone(),
		symbol,
		side: engine_side,
		order_type: engine_order_type,
		quantity: final_quantity_u64,
		price: price_i32,
		volume: volume_decimal,
		user_id,
		privy_id: privy_id.to_string(),
		outcome_name: outcome_name.to_string(),
	};
	let order_input_msg = OrderInputMessage::SubmitOrder(submit_order_msg);

	let msg_json = serde_json::to_string(&order_input_msg).map_err(|e| {
		tracing::error!("{} request_id={}, privy_id={}, order_id={} - Failed to serialize order message: {}", crate::consts::LOG_ORDER_NOT_PUSHED, client_info.request_id, privy_id, order_id, e);
		InternalError
	})?;

	// 使用 engine_input_mq 连接 (DB 0) 发送订单到 match_engine
	let mut conn = redis_pool::get_engine_input_mq_connection().await.map_err(|e| {
		tracing::error!("{} request_id={}, privy_id={}, order_id={} - Failed to get redis connection: {}", crate::consts::LOG_ORDER_NOT_PUSHED, client_info.request_id, privy_id, order_id, e);
		InternalError
	})?;

	let _: String = conn.xadd(ORDER_INPUT_STREAM, "*", &[(ORDER_INPUT_MSG_KEY, msg_json.as_str())]).await.map_err(|e| {
		tracing::error!("{} request_id={}, privy_id={}, order_id={} - Failed to send order to match engine: {}", crate::consts::LOG_ORDER_NOT_PUSHED, client_info.request_id, privy_id, order_id, e);
		InternalError
	})?;

	info!("request_id={}, privy_id={}, order_id={} - Order sent to match engine", client_info.request_id, privy_id, order_id);
	Ok(Json(ApiResponse::success(order_id)))
}

/// 取消订单
pub async fn handle_cancel_order(Extension(client_info): Extension<ClientInfo>, Json(params): Json<crate::api_types::CancelOrderRequest>) -> Result<Json<CancelOrderResponse>, InternalError> {
	let auth_result = match get_user_id_from_auth(&client_info, &client_info.request_id).await {
		Ok(result) => result,
		Err(e) => return Ok(Json(ApiResponse::error(e.to_api_error_code()))),
	};
	let user_id = auth_result.user_id;
	let privy_id = auth_result.privy_id;

	// 打印请求参数日志
	info!("[CancelOrder] request_id={}, privy_id={}, order_id={}", client_info.request_id, privy_id, params.order_id);

	let order_uuid = match sqlx::types::Uuid::parse_str(&params.order_id) {
		Ok(uuid) => uuid,
		Err(_) => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::InvalidParameter)));
		}
	};

	let read_pool = db::get_db_read_pool().map_err(|e| {
		tracing::error!("request_id={}, privy_id={} - Failed to get db pool: {}", client_info.request_id, privy_id, e);
		InternalError
	})?;

	let order: Option<common::model::Orders> = sqlx::query_as("SELECT * FROM orders WHERE id = $1 AND user_id = $2 AND (status = $3 OR status = $4) AND order_type = $5")
		.bind(order_uuid)
		.bind(user_id)
		.bind(OrderStatus::New)
		.bind(OrderStatus::PartiallyFilled)
		.bind(OrderType::Limit)
		.fetch_optional(&read_pool)
		.await
		.map_err(|e| {
			tracing::error!("request_id={}, privy_id={} - Failed to query order: {}", client_info.request_id, privy_id, e);
			InternalError
		})?;

	let Some(order) = order else {
		return Ok(Json(ApiResponse::customer_error("Order not found or cannot be cancelled".to_string())));
	};
	let symbol = PredictionSymbol { event_id: order.event_id, market_id: order.market_id, token_id: order.token_id };
	let order_id_str = params.order_id.clone();
	let cancel_order_msg = CancelOrderMessage { symbol, order_id: params.order_id };
	let order_input_msg = OrderInputMessage::CancelOrder(cancel_order_msg);

	let msg_json = serde_json::to_string(&order_input_msg).map_err(|e| {
		tracing::error!("request_id={}, privy_id={} - Failed to serialize cancel message: {}", client_info.request_id, privy_id, e);
		InternalError
	})?;

	// 使用 engine_input_mq 连接 (DB 0) 发送取消订单到 match_engine
	let mut conn = redis_pool::get_engine_input_mq_connection().await.map_err(|e| {
		tracing::error!("request_id={}, privy_id={} - Failed to get redis connection: {}", client_info.request_id, privy_id, e);
		InternalError
	})?;

	let _: String = conn.xadd(ORDER_INPUT_STREAM, "*", &[(ORDER_INPUT_MSG_KEY, msg_json.as_str())]).await.map_err(|e| {
		tracing::error!("request_id={}, privy_id={} - Failed to send cancel order to match engine: {}", client_info.request_id, privy_id, e);
		InternalError
	})?;

	info!("request_id={}, privy_id={}, order_id={} - Cancel order sent to match engine", client_info.request_id, privy_id, order_id_str);
	Ok(Json(ApiResponse::success(())))
}

/// 取消用户所有订单
pub async fn handle_cancel_all_orders(Extension(client_info): Extension<ClientInfo>) -> Result<Json<CancelAllOrdersResponse>, InternalError> {
	let auth_result = match get_user_id_from_auth(&client_info, &client_info.request_id).await {
		Ok(result) => result,
		Err(e) => return Ok(Json(ApiResponse::error(e.to_api_error_code()))),
	};
	let user_id = auth_result.user_id;
	let privy_id = auth_result.privy_id;

	// 打印请求参数日志
	info!("[CancelAllOrders] request_id={}, privy_id={}", client_info.request_id, privy_id);

	let read_pool = db::get_db_read_pool().map_err(|e| {
		tracing::error!("request_id={}, privy_id={} - Failed to get db pool: {}", client_info.request_id, privy_id, e);
		InternalError
	})?;

	let orders: Vec<common::model::Orders> = sqlx::query_as("SELECT * FROM orders WHERE user_id = $1 AND (status = $2 OR status = $3) AND order_type = $4")
		.bind(user_id)
		.bind(OrderStatus::New)
		.bind(OrderStatus::PartiallyFilled)
		.bind(OrderType::Limit)
		.fetch_all(&read_pool)
		.await
		.map_err(|e| {
			tracing::error!("request_id={}, privy_id={} - Failed to query orders: {}", client_info.request_id, privy_id, e);
			InternalError
		})?;

	if orders.is_empty() {
		return Ok(Json(ApiResponse::success(())));
	}

	// 使用 engine_input_mq 连接 (DB 0) 发送批量取消订单到 match_engine
	let mut conn = redis_pool::get_engine_input_mq_connection().await.map_err(|e| {
		tracing::error!("request_id={}, privy_id={} - Failed to get redis connection: {}", client_info.request_id, privy_id, e);
		InternalError
	})?;

	let mut pipe = redis::pipe();
	for order in orders.iter() {
		let symbol = PredictionSymbol { event_id: order.event_id, market_id: order.market_id, token_id: order.token_id.clone() };
		let cancel_order_msg = CancelOrderMessage { symbol, order_id: order.id.to_string() };
		let order_input_msg = OrderInputMessage::CancelOrder(cancel_order_msg);

		let msg_json = serde_json::to_string(&order_input_msg).map_err(|e| {
			tracing::error!("request_id={}, privy_id={} - Failed to serialize cancel message: {}", client_info.request_id, privy_id, e);
			InternalError
		})?;

		pipe.xadd(ORDER_INPUT_STREAM, "*", &[(ORDER_INPUT_MSG_KEY, msg_json.as_str())]);
	}

	let _: Vec<String> = pipe.query_async(&mut conn).await.map_err(|e| {
		tracing::error!("request_id={}, privy_id={} - Failed to send cancel orders to match engine: {}", client_info.request_id, privy_id, e);
		InternalError
	})?;

	info!("request_id={}, privy_id={}, count={} - Cancelled all orders", client_info.request_id, privy_id, orders.len());
	Ok(Json(ApiResponse::success(())))
}

// ============ Query Handlers (from api_query) ============

/// 获取用户数据
pub async fn handle_user_data(Extension(client_info): Extension<ClientInfo>, Query(params): Query<UserDataRequest>) -> Result<Json<ApiResponse<UserDataResponse>>, InternalError> {
	// api_key 用户不支持此接口（用于新用户注册场景）
	if !client_info.api_key.is_empty() && client_info.privy_id.is_none() {
		return Ok(Json(ApiResponse::error(ApiErrorCode::AuthFailed)));
	}

	let privy_id = match &client_info.privy_id {
		Some(id) => id.clone(),
		None => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::AuthFailed)));
		}
	};

	let read_pool = db::get_db_read_pool().map_err(|e| {
		tracing::error!("{}", e);
		InternalError
	})?;
	let user_result: Option<Users> = sqlx::query_as::<_, Users>(
		"SELECT id, privy_id, privy_evm_address, privy_email, privy_x, privy_x_image, name, bio, profile_image, last_login_ip, last_login_region, last_login_at, registered_ip, registered_region, created_at FROM users WHERE privy_id = $1",
	)
	.bind(&privy_id)
	.fetch_optional(&read_pool)
	.await
	.map_err(|e| {
		tracing::error!("Database query error: {}", e);
		InternalError
	})?;

	let user = match user_result {
		Some(mut user) => {
			let now = Utc::now();
			let time_diff = now.signed_duration_since(user.last_login_at);

			if time_diff.num_seconds() >= 3600 {
				let region = match get_ip_info(ReqGetIpInfo { ip: client_info.ip.clone() }).await {
					Ok(ip_info) => ip_info.region,
					Err(e) => {
						tracing::warn!("Failed to get ip info for IP {}: {}, using empty region", client_info.ip, e);
						String::new()
					}
				};

				let write_pool = db::get_db_write_pool().map_err(|e| {
					tracing::error!("{}", e);
					InternalError
				})?;
				sqlx::query("UPDATE users SET last_login_ip = $1, last_login_region = $2, last_login_at = $3 WHERE id = $4")
					.bind(&client_info.ip)
					.bind(&region)
					.bind(now)
					.bind(user.id)
					.execute(&write_pool)
					.await
					.map_err(|e| {
						tracing::error!("Failed to update user login info: {}", e);
						InternalError
					})?;

				user.last_login_ip = client_info.ip.clone();
				user.last_login_region = region;
				user.last_login_at = now;
			}

			user
		}
		None => {
			// 新用户注册流程
			let privy_info = get_privy_user_info(ReqGetPrivyUserInfo { privy_id: privy_id.clone() }).await.map_err(|e| {
				tracing::error!("Failed to get privy user info: {}", e);
				InternalError
			})?;

			if privy_info.privy_evm_address.is_empty() {
				tracing::warn!("Privy no address for privy_id: {}", privy_id);
				return Ok(Json(ApiResponse::error(ApiErrorCode::PrivyNoAddress)));
			}

			let region = match get_ip_info(ReqGetIpInfo { ip: client_info.ip.clone() }).await {
				Ok(ip_info) => ip_info.region,
				Err(e) => {
					tracing::warn!("Failed to get ip info for IP {}: {}, using empty region", client_info.ip, e);
					String::new()
				}
			};

			// 查找邀请人（邀请码即 privy_id，直接用现有缓存查找）
			let inviter_id = if let Some(ref invite_code) = params.invite_code { cache::get_user_id_by_privy_id(invite_code).await.ok() } else { None };

			let now = Utc::now();
			let write_pool = db::get_db_write_pool().map_err(|e| {
				tracing::error!("{}", e);
				InternalError
			})?;

			// 使用事务处理用户注册和邀请记录
			let mut tx = write_pool.begin().await.map_err(|e| {
				tracing::error!("Failed to begin transaction: {}", e);
				InternalError
			})?;

			// 插入用户
			let user_id: i64 = sqlx::query_scalar(
				"INSERT INTO users (privy_id, privy_evm_address, privy_email, privy_x, privy_x_image, name, bio, profile_image, last_login_ip, last_login_region, last_login_at, registered_ip, registered_region, created_at) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14) RETURNING id",
			)
			.bind(&privy_info.privy_id)
			.bind(&privy_info.privy_evm_address)
			.bind(&privy_info.privy_email)
			.bind(&privy_info.privy_x)
			.bind(&privy_info.privy_x_profile_image)
			.bind(&privy_info.privy_evm_address)
			.bind("")
			.bind("")
			.bind(&client_info.ip)
			.bind(&region)
			.bind(now)
			.bind(&client_info.ip)
			.bind(&region)
			.bind(now)
			.fetch_one(&mut *tx)
			.await
			.map_err(|e| {
				tracing::error!("Failed to insert user: {}", e);
				InternalError
			})?;

			// 插入被邀请人的邀请记录（积分由 statistics 服务计算）
			sqlx::query("INSERT INTO user_invitations (user_id, inviter_id, created_at) VALUES ($1, $2, $3)").bind(user_id).bind(inviter_id).bind(now).execute(&mut *tx).await.map_err(|e| {
				tracing::error!("Failed to insert invitation record: {}", e);
				InternalError
			})?;

			// 插入积分记录
			sqlx::query("INSERT INTO points (user_id, created_at, updated_at) VALUES ($1, $2, $3)").bind(user_id).bind(now).bind(now).execute(&mut *tx).await.map_err(|e| {
				tracing::error!("Failed to insert points record: {}", e);
				InternalError
			})?;

			// 提交事务
			tx.commit().await.map_err(|e| {
				tracing::error!("Failed to commit transaction: {}", e);
				InternalError
			})?;

			// 更新内存缓存
			cache::insert_user_id_cache(&privy_info.privy_id, user_id).await;

			let new_user_msg = common::onchain_msg_types::NewUser { user_id, privy_id: privy_info.privy_id.clone(), privy_evm_address: privy_info.privy_evm_address.clone() };
			if let Ok(msg_json) = serde_json::to_string(&new_user_msg)
				&& let Ok(mut conn) = redis_pool::get_common_mq_connection().await
			{
				let _: Result<String, _> = conn.xadd(NEW_USER_STREAM, "*", &[(NEW_USER_MSG_KEY, msg_json.as_str())]).await;
				// 延迟 6s 推送空投消息，确保扫块程序先消费新用户注册消息以检测充值
				tokio::spawn(async move {
					tokio::time::sleep(std::time::Duration::from_secs(6)).await;
					if let Ok(mut conn) = redis_pool::get_common_mq_connection().await {
						let _: Result<String, redis::RedisError> = conn.xadd(AIRDROP_STREAM, "*", &[(AIRDROP_MSG_KEY, msg_json.as_str())]).await;
					}
				});
			}

			Users {
				id: user_id,
				privy_id: privy_info.privy_id,
				privy_evm_address: privy_info.privy_evm_address,
				privy_email: privy_info.privy_email,
				privy_x: privy_info.privy_x,
				privy_x_image: privy_info.privy_x_profile_image,
				name: String::new(),
				bio: String::new(),
				profile_image: String::new(),
				last_login_ip: client_info.ip.clone(),
				last_login_region: region.clone(),
				last_login_at: now,
				registered_ip: client_info.ip.clone(),
				registered_region: region,
				created_at: now,
			}
		}
	};

	Ok(Json(ApiResponse::success(UserDataResponse::from(user))))
}

/// 获取用户邀请统计（积分比例、boost倍数、邀请获得积分、被邀请人总交易量、邀请人数）
pub async fn handle_invite_stats(Extension(client_info): Extension<ClientInfo>) -> Result<Json<ApiResponse<InviteStatsResponse>>, InternalError> {
	let auth_result = match get_user_id_from_auth(&client_info, &client_info.request_id).await {
		Ok(result) => result,
		Err(e) => return Ok(Json(ApiResponse::error(e.to_api_error_code()))),
	};

	let read_pool = db::get_db_read_pool().map_err(|e| {
		tracing::error!("{}", e);
		InternalError
	})?;

	// 1. 从 point_metadata 获取用户的 invite_points_ratio 和 invitee_boost（没有则用默认值）
	let metadata: Option<(i32, Decimal)> =
		sqlx::query_as("SELECT invite_points_ratio, invitee_boost FROM point_metadata WHERE user_id = $1").bind(auth_result.user_id).fetch_optional(&read_pool).await.map_err(|e| {
			tracing::error!("Database query error: {}", e);
			InternalError
		})?;

	let (invite_points_ratio, invitee_boost) = match metadata {
		Some((ratio, boost)) => (ratio, boost.to_string()),
		None => (DEFAULT_INVITE_POINTS_RATIO, DEFAULT_INVITEE_BOOST.to_string()),
	};

	// 2. 从 points 表获取 invite_earned_points
	let invite_earned_points: i64 = sqlx::query_scalar("SELECT COALESCE(invite_earned_points, 0) FROM points WHERE user_id = $1")
		.bind(auth_result.user_id)
		.fetch_optional(&read_pool)
		.await
		.map_err(|e| {
			tracing::error!("Database query error: {}", e);
			InternalError
		})?
		.unwrap_or(0);

	// 3. 查询所有被邀请人的 accumulated_volume 之和 (通过 user_invitations 关联 points)
	let invitees_total_volume: Decimal = sqlx::query_scalar("SELECT COALESCE(SUM(p.accumulated_volume), 0) FROM user_invitations ui JOIN points p ON ui.user_id = p.user_id WHERE ui.inviter_id = $1")
		.bind(auth_result.user_id)
		.fetch_one(&read_pool)
		.await
		.map_err(|e| {
			tracing::error!("Database query error: {}", e);
			InternalError
		})?;

	// 4. 查询邀请人数
	let invite_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM user_invitations WHERE inviter_id = $1").bind(auth_result.user_id).fetch_one(&read_pool).await.map_err(|e| {
		tracing::error!("Database query error: {}", e);
		InternalError
	})?;

	Ok(Json(ApiResponse::success(InviteStatsResponse { invite_points_ratio, invitee_boost, invite_earned_points, invitees_total_volume: invitees_total_volume.to_string(), invite_count })))
}

/// 获取当前赛季信息
pub async fn handle_current_season() -> Result<Json<ApiResponse<CurrentSeasonResponse>>, InternalError> {
	let group = singleflight::get_current_season_group().await;
	let result = group
		.work("current", async {
			let read_pool = db::get_db_read_pool()?;
			let season: Option<(i32, String, chrono::DateTime<Utc>, chrono::DateTime<Utc>)> =
				sqlx::query_as("SELECT id, name, start_time, end_time FROM seasons WHERE active = true ORDER BY id DESC LIMIT 1").fetch_optional(&read_pool).await?;
			Ok(season.map(|(season_id, name, start_time, end_time)| CurrentSeasonResponse { season_id, name, start_time: start_time.timestamp(), end_time: end_time.timestamp() }))
		})
		.await;

	match result {
		Ok(Some(season)) => Ok(Json(ApiResponse::success(season))),
		Ok(None) => Ok(Json(ApiResponse::error(ApiErrorCode::SeasonNotFound))),
		Err(Some(e)) => {
			tracing::error!("Current season query error: {}", e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Current season query cancelled");
			Err(InternalError)
		}
	}
}

/// 获取所有赛季列表
pub async fn handle_seasons_list() -> Result<Json<ApiResponse<SeasonsListResponse>>, InternalError> {
	let group = singleflight::get_seasons_list_group().await;
	let result = group
		.work("list", async {
			let read_pool = db::get_db_read_pool()?;
			let rows: Vec<(i32, chrono::DateTime<Utc>, chrono::DateTime<Utc>)> = sqlx::query_as("SELECT id, start_time, end_time FROM seasons ORDER BY id ASC").fetch_all(&read_pool).await?;
			Ok(rows.into_iter().map(|(season_id, start_time, end_time)| SeasonItem { season_id, start_time: start_time.timestamp(), end_time: end_time.timestamp() }).collect())
		})
		.await;

	match result {
		Ok(seasons) => Ok(Json(ApiResponse::success(SeasonsListResponse { seasons }))),
		Err(Some(e)) => {
			tracing::error!("Seasons list query error: {}", e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Seasons list query cancelled");
			Err(InternalError)
		}
	}
}

/// 内部函数: 获取当前最新赛季ID (使用 singleflight)
async fn get_current_season_id() -> anyhow::Result<Option<i32>> {
	let group = singleflight::get_current_season_id_group().await;
	let result = group
		.work("current_id", async {
			let read_pool = db::get_db_read_pool()?;
			let season_id: Option<i32> = sqlx::query_scalar("SELECT id FROM seasons WHERE active = true ORDER BY id DESC LIMIT 1").fetch_optional(&read_pool).await?;
			Ok(season_id)
		})
		.await;

	match result {
		Ok(season_id) => Ok(season_id),
		Err(Some(e)) => Err(e),
		Err(None) => Err(anyhow::anyhow!("Query cancelled")),
	}
}

/// 内部函数: 检查赛季是否存在 (使用 singleflight)
async fn season_exists(season_id: i32) -> anyhow::Result<bool> {
	let group = singleflight::get_season_exists_group().await;
	let key = season_id.to_string();
	let result = group
		.work(&key, async move {
			let read_pool = db::get_db_read_pool()?;
			let exists: Option<i32> = sqlx::query_scalar("SELECT id FROM seasons WHERE id = $1").bind(season_id).fetch_optional(&read_pool).await?;
			Ok(exists.is_some())
		})
		.await;

	match result {
		Ok(exists) => Ok(exists),
		Err(Some(e)) => Err(e),
		Err(None) => Err(anyhow::anyhow!("Query cancelled")),
	}
}

/// 获取积分排行榜（前200名，支持历史赛季查询）
pub async fn handle_leaderboard_points(Extension(client_info): Extension<ClientInfo>, Query(params): Query<LeaderboardRequest>) -> Result<Json<ApiResponse<LeaderboardPointsResponse>>, InternalError> {
	// 可选认证 - 用于获取当前用户排名
	let auth_result = get_user_id_from_auth(&client_info, &client_info.request_id).await.ok();

	// 获取当前赛季ID
	let current_season_id = get_current_season_id().await.map_err(|e| {
		tracing::error!("Failed to get current season: {}", e);
		InternalError
	})?;

	// 确定查询的赛季ID
	let (query_season_id, is_current_season) = match params.season_id {
		Some(season_id) => {
			// 传了赛季ID，检查是否存在
			let exists = season_exists(season_id).await.map_err(|e| {
				tracing::error!("Failed to check season exists: {}", e);
				InternalError
			})?;
			if !exists {
				return Ok(Json(ApiResponse::error(ApiErrorCode::SeasonNotFound)));
			}
			(season_id, current_season_id == Some(season_id))
		}
		None => {
			// 没传赛季ID，使用当前赛季
			match current_season_id {
				Some(id) => (id, true),
				None => return Ok(Json(ApiResponse::error(ApiErrorCode::SeasonNotFound))),
			}
		}
	};

	// 使用 singleflight 获取前200名 (公共数据)
	let group = singleflight::get_leaderboard_points_group().await;
	let singleflight_key = format!("points:{}", query_season_id);
	let is_current = is_current_season;

	let result = group
		.work(&singleflight_key, async move {
			let read_pool = db::get_db_read_pool()?;

			let rows: Vec<(i64, i64, String, String)> = if is_current {
				// 当前赛季从 points 表查询
				sqlx::query_as(
					r#"
					SELECT p.user_id, p.total_points, u.name, u.profile_image
					FROM points p
					JOIN users u ON u.id = p.user_id
					WHERE p.total_points > 0
					ORDER BY p.total_points DESC
					LIMIT $1
					"#,
				)
				.bind(LEADERBOARD_MAX_DISPLAY)
				.fetch_all(&read_pool)
				.await?
			} else {
				// 历史赛季从 season_history 查询
				sqlx::query_as(
					r#"
					SELECT sh.user_id, sh.total_points, u.name, u.profile_image
					FROM season_history sh
					JOIN users u ON u.id = sh.user_id
					WHERE sh.season_id = $1 AND sh.total_points > 0
					ORDER BY sh.total_points DESC
					LIMIT $2
					"#,
				)
				.bind(query_season_id)
				.bind(LEADERBOARD_MAX_DISPLAY)
				.fetch_all(&read_pool)
				.await?
			};

			let leaderboard: Vec<LeaderboardPointsItem> = rows
				.into_iter()
				.enumerate()
				.map(|(idx, (user_id, total_points, name, profile_image))| LeaderboardPointsItem { rank: (idx + 1) as i32, user_id, name, profile_image, total_points })
				.collect();

			Ok(leaderboard)
		})
		.await;

	let leaderboard = match result {
		Ok(leaderboard) => leaderboard,
		Err(Some(e)) => {
			tracing::error!("Leaderboard points query error: {}", e);
			return Err(InternalError);
		}
		Err(None) => {
			tracing::error!("Leaderboard points query cancelled");
			return Err(InternalError);
		}
	};

	// 获取当前用户排名和积分 - 直接遍历已获取的结果
	let (my_rank, my_total_points) = if let Some(auth) = auth_result {
		// 在前200名中查找用户
		if let Some(item) = leaderboard.iter().find(|item| item.user_id == auth.user_id) {
			(Some(item.rank.to_string()), Some(item.total_points))
		} else {
			// 不在前200名，排名显示">200"，但仍需查询用户积分
			let read_pool = db::get_db_read_pool().map_err(|e| {
				tracing::error!("{}", e);
				InternalError
			})?;

			let points: Option<i64> = if is_current_season {
				sqlx::query_scalar("SELECT total_points FROM points WHERE user_id = $1").bind(auth.user_id).fetch_optional(&read_pool).await
			} else {
				sqlx::query_scalar("SELECT total_points FROM season_history WHERE season_id = $1 AND user_id = $2").bind(query_season_id).bind(auth.user_id).fetch_optional(&read_pool).await
			}
			.map_err(|e| {
				tracing::error!("Database query error: {}", e);
				InternalError
			})?;

			(Some(format!(">{}", LEADERBOARD_MAX_DISPLAY)), points)
		}
	} else {
		(None, None)
	};

	Ok(Json(ApiResponse::success(LeaderboardPointsResponse { leaderboard, my_rank, my_total_points })))
}

/// 获取交易量排行榜（前200名，支持历史赛季查询）
pub async fn handle_leaderboard_volume(Extension(client_info): Extension<ClientInfo>, Query(params): Query<LeaderboardRequest>) -> Result<Json<ApiResponse<LeaderboardVolumeResponse>>, InternalError> {
	// 可选认证 - 用于获取当前用户排名
	let auth_result = get_user_id_from_auth(&client_info, &client_info.request_id).await.ok();

	// 获取当前赛季ID
	let current_season_id = get_current_season_id().await.map_err(|e| {
		tracing::error!("Failed to get current season: {}", e);
		InternalError
	})?;

	// 确定查询的赛季ID
	let (query_season_id, is_current_season) = match params.season_id {
		Some(season_id) => {
			// 传了赛季ID，检查是否存在
			let exists = season_exists(season_id).await.map_err(|e| {
				tracing::error!("Failed to check season exists: {}", e);
				InternalError
			})?;
			if !exists {
				return Ok(Json(ApiResponse::error(ApiErrorCode::SeasonNotFound)));
			}
			(season_id, current_season_id == Some(season_id))
		}
		None => {
			// 没传赛季ID，使用当前赛季
			match current_season_id {
				Some(id) => (id, true),
				None => return Ok(Json(ApiResponse::error(ApiErrorCode::SeasonNotFound))),
			}
		}
	};

	// 使用 singleflight 获取前200名 (公共数据)
	let group = singleflight::get_leaderboard_volume_group().await;
	let singleflight_key = format!("volume:{}", query_season_id);
	let is_current = is_current_season;

	let result = group
		.work(&singleflight_key, async move {
			let read_pool = db::get_db_read_pool()?;

			let rows: Vec<(i64, Decimal, String, String)> = if is_current {
				// 当前赛季从 points 表查询
				sqlx::query_as(
					r#"
					SELECT p.user_id, p.accumulated_volume, u.name, u.profile_image
					FROM points p
					JOIN users u ON u.id = p.user_id
					WHERE p.accumulated_volume > 0
					ORDER BY p.accumulated_volume DESC
					LIMIT $1
					"#,
				)
				.bind(LEADERBOARD_MAX_DISPLAY)
				.fetch_all(&read_pool)
				.await?
			} else {
				// 历史赛季从 season_history 查询
				sqlx::query_as(
					r#"
					SELECT sh.user_id, sh.accumulated_volume, u.name, u.profile_image
					FROM season_history sh
					JOIN users u ON u.id = sh.user_id
					WHERE sh.season_id = $1 AND sh.accumulated_volume > 0
					ORDER BY sh.accumulated_volume DESC
					LIMIT $2
					"#,
				)
				.bind(query_season_id)
				.bind(LEADERBOARD_MAX_DISPLAY)
				.fetch_all(&read_pool)
				.await?
			};

			let leaderboard: Vec<LeaderboardVolumeItem> = rows
				.into_iter()
				.enumerate()
				.map(|(idx, (user_id, accumulated_volume, name, profile_image))| {
					LeaderboardVolumeItem { rank: (idx + 1) as i32, user_id, name, profile_image, accumulated_volume: accumulated_volume.to_string() }
				})
				.collect();

			Ok(leaderboard)
		})
		.await;

	let leaderboard = match result {
		Ok(leaderboard) => leaderboard,
		Err(Some(e)) => {
			tracing::error!("Leaderboard volume query error: {}", e);
			return Err(InternalError);
		}
		Err(None) => {
			tracing::error!("Leaderboard volume query cancelled");
			return Err(InternalError);
		}
	};

	// 获取当前用户排名和交易量 - 直接遍历已获取的结果
	let (my_rank, my_accumulated_volume) = if let Some(auth) = auth_result {
		// 在前200名中查找用户
		if let Some(item) = leaderboard.iter().find(|item| item.user_id == auth.user_id) {
			(Some(item.rank.to_string()), Some(item.accumulated_volume.clone()))
		} else {
			// 不在前200名，排名显示">200"，但仍需查询用户交易量
			let read_pool = db::get_db_read_pool().map_err(|e| {
				tracing::error!("{}", e);
				InternalError
			})?;

			let volume: Option<Decimal> = if is_current_season {
				sqlx::query_scalar("SELECT accumulated_volume FROM points WHERE user_id = $1").bind(auth.user_id).fetch_optional(&read_pool).await
			} else {
				sqlx::query_scalar("SELECT accumulated_volume FROM season_history WHERE season_id = $1 AND user_id = $2").bind(query_season_id).bind(auth.user_id).fetch_optional(&read_pool).await
			}
			.map_err(|e| {
				tracing::error!("Database query error: {}", e);
				InternalError
			})?;

			(Some(format!(">{}", LEADERBOARD_MAX_DISPLAY)), volume.map(|v| v.to_string()))
		}
	} else {
		(None, None)
	};

	Ok(Json(ApiResponse::success(LeaderboardVolumeResponse { leaderboard, my_rank, my_accumulated_volume })))
}

/// 获取邀请记录列表（前10条，按时间倒序）
pub async fn handle_invite_records(Extension(client_info): Extension<ClientInfo>) -> Result<Json<ApiResponse<InviteRecordsResponse>>, InternalError> {
	let auth_result = match get_user_id_from_auth(&client_info, &client_info.request_id).await {
		Ok(result) => result,
		Err(e) => return Ok(Json(ApiResponse::error(e.to_api_error_code()))),
	};

	let read_pool = db::get_db_read_pool().map_err(|e| {
		tracing::error!("{}", e);
		InternalError
	})?;

	// 查询被邀请用户信息（取前10条）
	let records: Vec<(i64, String, String, chrono::DateTime<Utc>)> = sqlx::query_as(
		r#"
		SELECT u.id, u.name, u.profile_image, ui.created_at
		FROM user_invitations ui
		JOIN users u ON u.id = ui.user_id
		WHERE ui.inviter_id = $1
		ORDER BY ui.created_at DESC
		LIMIT 10
		"#,
	)
	.bind(auth_result.user_id)
	.fetch_all(&read_pool)
	.await
	.map_err(|e| {
		tracing::error!("Database query error: {}", e);
		InternalError
	})?;

	let records = records.into_iter().map(|(user_id, name, profile_image, created_at)| InviteRecordItem { user_id, name, profile_image, created_at }).collect();

	Ok(Json(ApiResponse::success(InviteRecordsResponse { records })))
}

/// 获取用户图片上传签名
pub async fn handle_image_sign(Extension(client_info): Extension<ClientInfo>, Query(params): Query<GetImageSignRequest>) -> Result<Json<ApiResponse<ResGetGoogleImageSign>>, InternalError> {
	let auth_result = match get_user_id_from_auth(&client_info, &client_info.request_id).await {
		Ok(result) => result,
		Err(e) => return Ok(Json(ApiResponse::error(e.to_api_error_code()))),
	};
	let privy_id = auth_result.privy_id;

	if let Err(err_msg) = params.validate() {
		return Ok(Json(ApiResponse::customer_error(err_msg)));
	}

	let internal_req = ReqGetGoogleImageSign { uid: privy_id, bucket_type: params.bucket_type, image_type: params.image_type };

	let result = get_google_image_sign(internal_req).await.map_err(|e| {
		tracing::error!("Failed to get google image sign: {}", e);
		InternalError
	})?;

	Ok(Json(ApiResponse::success(result)))
}

/// 获取活跃的市场主题列表
/// 使用 singleflight 确保并发请求共享同一个数据库查询
pub async fn handle_topics() -> Result<Json<ApiResponse<Vec<String>>>, InternalError> {
	let result = get_topics_group()
		.await
		.work("topics", async {
			let read_pool = db::get_db_read_pool()?;
			let topics: Vec<String> = sqlx::query_scalar("SELECT topic FROM event_topics WHERE active = true").fetch_all(&read_pool).await.map_err(|e| {
				tracing::error!("Failed to query event topics: {}", e);
				anyhow::anyhow!(e)
			})?;
			Ok(topics)
		})
		.await;

	match result {
		Ok(topics) => Ok(Json(ApiResponse::success(topics))),
		Err(Some(e)) => {
			tracing::error!("Singleflight leader failed: {}", e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight leader dropped");
			Err(InternalError)
		}
	}
}

/// 生成 events 查询的 singleflight key
/// 格式: topic_sortBy_page_pageSize
fn build_events_singleflight_key(params: &EventsRequest) -> String {
	let mut parts = Vec::new();

	if let Some(topic) = &params.topic {
		parts.push(format!("topic:{}", topic));
	}

	if let Some(title) = &params.title {
		parts.push(format!("title:{}", title));
	}

	if params.ending_soon == Some(true) {
		parts.push("ending_soon".to_string());
	} else if params.newest == Some(true) {
		parts.push("newest".to_string());
	}
	// volume 是默认排序，不需要加入 key

	parts.push(format!("p{}", params.page));
	parts.push(format!("s{}", params.page_size));

	if parts.len() == 2 { format!("all_{}", parts.join("_")) } else { parts.join("_") }
}

/// 计算 chance 值
/// 如果 |best_bid - best_ask| < 0.1，使用 latest_trade_price
/// 否则使用 (best_bid + best_ask) / 2
fn calculate_chance(best_bid: &str, best_ask: &str, latest_trade_price: &str) -> String {
	let bid = Decimal::from_str(best_bid).unwrap_or(Decimal::ZERO);
	let ask = Decimal::from_str(best_ask).unwrap_or(Decimal::ZERO);

	// 如果 best_bid 或 best_ask 有一个是 0，直接使用最新成交价
	if bid.is_zero() || ask.is_zero() {
		if latest_trade_price.is_empty() {
			return "0".to_string();
		}
		return latest_trade_price.to_string();
	}

	let diff = (bid - ask).abs();
	let threshold = Decimal::new(1, 1); // 0.1

	if diff < threshold {
		// 使用 latest_trade_price
		if latest_trade_price.is_empty() {
			// 如果没有成交价，回退到中间价
			((bid + ask) / Decimal::TWO).normalize().to_string()
		} else {
			latest_trade_price.to_string()
		}
	} else {
		// 使用中间价
		((bid + ask) / Decimal::TWO).normalize().to_string()
	}
}

/// 获取预测事件列表
pub async fn handle_events(Query(params): Query<EventsRequest>) -> Result<Json<ApiResponse<EventsResponse>>, InternalError> {
	let singleflight_key = build_events_singleflight_key(&params);

	let result = get_events_group()
		.await
		.work(&singleflight_key, async {
			let read_pool = db::get_db_read_pool()?;

			// 构建 WHERE 子句
			// created_at 过滤: 只显示创建时间在 EVENT_DISPLAY_DELAY_SECS 秒之前的事件
			let mut where_clause = format!("WHERE closed = false AND resolved = false AND created_at < NOW() - INTERVAL '{} seconds'", crate::consts::EVENT_DISPLAY_DELAY_SECS);
			let mut bind_values: Vec<String> = Vec::new();

			if let Some(topic) = &params.topic {
				where_clause.push_str(&format!(" AND topic = ${}", bind_values.len() + 1));
				bind_values.push(topic.clone());
			}

			if let Some(title) = &params.title {
				where_clause.push_str(&format!(" AND title LIKE '%' || ${} || '%'", bind_values.len() + 1));
				bind_values.push(title.clone());
			}

			// 1. 先查询总数
			let count_sql = format!("SELECT COUNT(*) as count FROM events {}", where_clause);
			let total: i64 = match bind_values.len() {
				0 => sqlx::query_scalar(&count_sql).fetch_one(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?,
				1 => sqlx::query_scalar(&count_sql).bind(&bind_values[0]).fetch_one(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?,
				2 => sqlx::query_scalar(&count_sql).bind(&bind_values[0]).bind(&bind_values[1]).fetch_one(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?,
				_ => return Err(anyhow::anyhow!("Too many bind values")),
			};
			let total = total as i16;

			// 2. 构建分页查询 SQL
			let order_clause = if params.ending_soon == Some(true) {
				"ORDER BY end_date ASC NULLS LAST"
			} else if params.newest == Some(true) {
				"ORDER BY created_at DESC"
			} else {
				// 默认按交易量降序
				"ORDER BY volume DESC"
			};

			let offset = (params.page - 1).max(0) as i64 * params.page_size as i64;
			let limit = params.page_size as i64;
			let next_param = bind_values.len() + 1;

			let data_sql = format!("SELECT * FROM events {} {} OFFSET ${} LIMIT ${}", where_clause, order_clause, next_param, next_param + 1);

			// 执行分页查询
			let events: Vec<Events> = match bind_values.len() {
				0 => sqlx::query_as(&data_sql).bind(offset).bind(limit).fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?,
				1 => sqlx::query_as(&data_sql).bind(&bind_values[0]).bind(offset).bind(limit).fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?,
				2 => sqlx::query_as(&data_sql).bind(&bind_values[0]).bind(&bind_values[1]).bind(offset).bind(limit).fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?,
				_ => return Err(anyhow::anyhow!("Too many bind values")),
			};

			// 获取 Redis 连接，批量获取价格信息
			let mut conn = redis_pool::get_cache_redis_connection().await.map_err(|e| anyhow::anyhow!(e))?;

			// 收集所有需要查询的 field
			let mut fields: Vec<String> = Vec::new();
			for event in &events {
				for market_id_str in event.markets.keys() {
					let market_id: i16 = market_id_str.parse().unwrap_or(0);
					fields.push(market_field(event.id, market_id));
				}
			}

			// 批量 HMGET 获取价格信息
			let price_values: Vec<Option<String>> = if !fields.is_empty() { conn.hget(PRICE_CACHE_KEY, &fields).await.unwrap_or_else(|_| vec![None; fields.len()]) } else { Vec::new() };

			// 构建 field -> price_info 映射
			let mut price_map: std::collections::HashMap<String, CacheMarketPriceInfo> = std::collections::HashMap::new();
			for (field, value) in fields.iter().zip(price_values.iter()) {
				if let Some(v) = value
					&& let Ok(info) = serde_json::from_str::<CacheMarketPriceInfo>(v)
				{
					price_map.insert(field.clone(), info);
				}
			}

			// 批量查询所有 event 是否有 LIVE 状态的 streamer
			// 从 PostgreSQL live_rooms 表查询 status = 'LIVE' 且 event_id 在事件列表中的记录
			let mut event_ids_with_live: std::collections::HashSet<i64> = std::collections::HashSet::new();
			if !events.is_empty() {
				let event_id_strings: Vec<String> = events.iter().map(|e| e.id.to_string()).collect();
				let live_rooms_query = "SELECT DISTINCT event_id FROM live_rooms WHERE status = 'LIVE' AND event_id = ANY($1)";
				let live_event_ids: Vec<String> =
					sqlx::query_scalar(live_rooms_query).bind(&event_id_strings).fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!("Failed to query live_rooms: {}", e))?;

				for event_id_str in live_event_ids {
					if let Ok(event_id) = event_id_str.parse::<i64>() {
						event_ids_with_live.insert(event_id);
					}
				}
			}

			// 转换为响应格式
			let mut event_responses = Vec::new();
			for event in events {
				if let Some(end_date) = event.end_date
					&& end_date < Utc::now()
				{
					continue;
				}
				let mut markets = Vec::new();

				for (market_id_str, market) in event.markets.iter() {
					if market.closed {
						continue;
					}
					let market_id: i16 = market_id_str.parse().unwrap_or(0);
					let field = market_field(event.id, market_id);
					let price_info = price_map.get(&field);

					// 获取 token 0 和 token 1 的价格信息
					let (outcome_0_chance, outcome_1_chance, outcome_0_best_bid, outcome_0_best_ask, outcome_1_best_bid, outcome_1_best_ask) = if let Some(info) = price_info {
						let token_0_id = market.token_ids.first().cloned().unwrap_or_default();
						let token_1_id = market.token_ids.get(1).cloned().unwrap_or_default();

						let token_0_info = info.prices.get(&token_0_id);
						let token_1_info = info.prices.get(&token_1_id);

						let (t0_bid, t0_ask, t0_ltp) = token_0_info.map(|i| (i.best_bid.as_str(), i.best_ask.as_str(), i.latest_trade_price.as_str())).unwrap_or(("0", "0", ""));
						let (t1_bid, t1_ask, t1_ltp) = token_1_info.map(|i| (i.best_bid.as_str(), i.best_ask.as_str(), i.latest_trade_price.as_str())).unwrap_or(("0", "0", ""));

						(calculate_chance(t0_bid, t0_ask, t0_ltp), calculate_chance(t1_bid, t1_ask, t1_ltp), t0_bid.to_string(), t0_ask.to_string(), t1_bid.to_string(), t1_ask.to_string())
					} else {
						("0".to_string(), "0".to_string(), "0".to_string(), "0".to_string(), "0".to_string(), "0".to_string())
					};

					markets.push(EventMarketResponse {
						market_id,
						title: market.title.clone(),
						question: market.question.clone(),
						outcome_0_name: market.outcomes.first().cloned().unwrap_or_default(),
						outcome_1_name: market.outcomes.get(1).cloned().unwrap_or_default(),
						outcome_0_token_id: market.token_ids.first().cloned().unwrap_or_default(),
						outcome_1_token_id: market.token_ids.get(1).cloned().unwrap_or_default(),
						outcome_0_chance,
						outcome_1_chance,
						outcome_0_best_bid,
						outcome_0_best_ask,
						outcome_1_best_bid,
						outcome_1_best_ask,
					});
				}

				if markets.is_empty() {
					continue;
				}
				// 按 market_id 从小到大排序
				markets.sort_by_key(|m| m.market_id);

				// 检查该 event 是否有 LIVE 状态的 streamer
				let has_streamer = event_ids_with_live.contains(&event.id);

				event_responses.push(SingleEventResponse {
					event_id: event.id,
					slug: event.slug,
					image: event.image,
					title: event.title,
					volume: event.volume.normalize(),
					topic: event.topic,
					markets,
					has_streamer,
				});
			}

			let has_more = (params.page as i64 * params.page_size as i64) < total as i64;
			Ok(EventsResponse { events: event_responses, total, has_more })
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Singleflight events leader failed: {}", e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight events leader dropped");
			Err(InternalError)
		}
	}
}

/// 获取预测事件详情
pub async fn handle_event_detail(Query(params): Query<EventDetailRequest>) -> Result<Json<ApiResponse<EventDetailResponse>>, InternalError> {
	let event_id = params.event_id;
	let singleflight_key = event_id.to_string();

	let result = get_event_detail_group()
		.await
		.work(&singleflight_key, async {
			// 1. 先查询数据库获取事件信息
			let read_pool = db::get_db_read_pool()?;
			let event: Events = sqlx::query_as("SELECT * FROM events WHERE id = $1")
				.bind(event_id)
				.fetch_optional(&read_pool)
				.await
				.map_err(|e| anyhow::anyhow!(e))?
				.ok_or_else(|| anyhow::anyhow!("Event not found"))?;

			// 2. 获取价格信息
			let mut conn = redis_pool::get_cache_redis_connection().await.map_err(|e| anyhow::anyhow!(e))?;

			// 收集需要查询的 price fields
			let price_fields: Vec<String> = event
				.markets
				.keys()
				.map(|market_id_str| {
					let market_id: i16 = market_id_str.parse().unwrap_or(0);
					market_field(event.id, market_id)
				})
				.collect();

			// 批量获取价格信息
			let price_values: Vec<Option<String>> =
				if !price_fields.is_empty() { conn.hget(PRICE_CACHE_KEY, &price_fields).await.unwrap_or_else(|_| vec![None; price_fields.len()]) } else { Vec::new() };

			// 构建 field -> price_info 映射
			let mut price_map: std::collections::HashMap<String, CacheMarketPriceInfo> = std::collections::HashMap::new();
			for (field, value) in price_fields.iter().zip(price_values.iter()) {
				if let Some(v) = value
					&& let Ok(info) = serde_json::from_str::<CacheMarketPriceInfo>(v)
				{
					price_map.insert(field.clone(), info);
				}
			}

			// 转换为响应格式
			let mut markets = Vec::new();
			for (market_id_str, market) in event.markets.iter() {
				let market_id_i16: i16 = market_id_str.parse().unwrap_or(0);
				let field = market_field(event.id, market_id_i16);
				let price_info = price_map.get(&field);

				let (outcome_0_chance, outcome_1_chance, outcome_0_best_bid, outcome_0_best_ask, outcome_1_best_bid, outcome_1_best_ask) = if let Some(info) = price_info {
					let token_0_id = market.token_ids.first().cloned().unwrap_or_default();
					let token_1_id = market.token_ids.get(1).cloned().unwrap_or_default();

					let token_0_info = info.prices.get(&token_0_id);
					let token_1_info = info.prices.get(&token_1_id);

					let (t0_bid, t0_ask, t0_ltp) = token_0_info.map(|i| (i.best_bid.as_str(), i.best_ask.as_str(), i.latest_trade_price.as_str())).unwrap_or(("0", "0", ""));
					let (t1_bid, t1_ask, t1_ltp) = token_1_info.map(|i| (i.best_bid.as_str(), i.best_ask.as_str(), i.latest_trade_price.as_str())).unwrap_or(("0", "0", ""));

					let t0_chance = Decimal::from_str(&calculate_chance(t0_bid, t0_ask, t0_ltp)).unwrap_or(Decimal::ZERO).normalize();
					let t1_chance = Decimal::from_str(&calculate_chance(t1_bid, t1_ask, t1_ltp)).unwrap_or(Decimal::ZERO).normalize();
					let t0_bid_dec = Decimal::from_str(t0_bid).unwrap_or(Decimal::ZERO).normalize();
					let t0_ask_dec = Decimal::from_str(t0_ask).unwrap_or(Decimal::ZERO).normalize();
					let t1_bid_dec = Decimal::from_str(t1_bid).unwrap_or(Decimal::ZERO).normalize();
					let t1_ask_dec = Decimal::from_str(t1_ask).unwrap_or(Decimal::ZERO).normalize();

					(t0_chance, t1_chance, t0_bid_dec, t0_ask_dec, t1_bid_dec, t1_ask_dec)
				} else {
					(Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO, Decimal::ZERO)
				};

				markets.push(EventMarketDetail {
					title: market.title.clone(),
					question: market.question.clone(),
					image: market.image.clone(),
					market_id: market_id_i16,
					volume: market.volume.normalize(),
					condition_id: market.condition_id.clone(),
					parent_collection_id: market.parent_collection_id.clone(),
					outcome_0_name: market.outcomes.first().cloned().unwrap_or_default(),
					outcome_1_name: market.outcomes.get(1).cloned().unwrap_or_default(),
					outcome_0_token_id: market.token_ids.first().cloned().unwrap_or_default(),
					outcome_1_token_id: market.token_ids.get(1).cloned().unwrap_or_default(),
					outcome_0_chance,
					outcome_1_chance,
					outcome_0_best_bid,
					outcome_0_best_ask,
					outcome_1_best_bid,
					outcome_1_best_ask,
					closed: market.closed,
					winner_outcome_name: market.win_outcome_name.clone(),
					winner_outcome_token_id: market.win_outcome_token_id.clone(),
				});
			}

			let endtime = event.end_date.map(|d| d.timestamp()).unwrap_or(0);
			let starttime = event.created_at.timestamp();

			// 按 market_id 从小到大排序
			markets.sort_by_key(|m| m.market_id);

			Ok(EventDetailResponse {
				event_id: event.id,
				slug: event.slug,
				image: event.image,
				title: event.title,
				rules: event.description,
				volume: event.volume.normalize(),
				endtime,
				starttime,
				closed: event.closed,
				resolved: event.resolved,
				markets,
			})
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Singleflight event_detail leader failed: {}", e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight event_detail leader dropped");
			Err(InternalError)
		}
	}
}

/// 获取直播事件列表（以 streamer 为单位）
///
/// 流程：
/// 1. 从 PostgreSQL 的 live_rooms 表查询所有数据
/// 2. 从 PostgreSQL 查询对应的 events 数据（确保 event 存在）
/// 3. 构建 streamer-event 对，每个 streamer 对应一个 event
///
/// 使用 singleflight 确保并发请求共享同一个查询
pub async fn handle_lives(Query(params): Query<LivesRequest>) -> Result<Json<ApiResponse<LivesResponse>>, InternalError> {
	// 构建 singleflight key
	let singleflight_key = format!(
		"lives:{}:{}:{}:{}:{}:{}",
		params.ending_soon.unwrap_or(false),
		params.newest.unwrap_or(false),
		params.topic.as_deref().unwrap_or(""),
		params.volume.unwrap_or(false),
		params.page,
		params.page_size
	);

	let group = get_lives_group().await;
	let result = group
		.work(&singleflight_key, async {
			let read_pool = db::get_db_read_pool().map_err(|e| anyhow::anyhow!("Failed to get db read pool: {}", e))?;

			let live_rooms: Vec<crate::api_types::LiveRoomRow> = sqlx::query_as(
				"SELECT id, room_id, room_name, room_desc, status, event_id, create_date::timestamptz, picture_url, created_at::timestamptz, updated_at::timestamptz FROM live_rooms WHERE status = 'LIVE'",
			)
			.fetch_all(&read_pool)
			.await
			.map_err(|e| anyhow::anyhow!("Failed to query live_rooms: {}", e))?;

			if live_rooms.is_empty() {
				return Ok(LivesResponse { streamers: Vec::new(), total: 0, has_more: false });
			}

			// 收集所有 event_id（从 live_rooms 表中）
			let mut event_ids: Vec<i64> = live_rooms.iter().filter_map(|room| room.event_id.as_ref().and_then(|eid| eid.parse::<i64>().ok())).collect();
			event_ids.sort();
			event_ids.dedup();

			if event_ids.is_empty() {
				return Ok(LivesResponse { streamers: Vec::new(), total: 0, has_more: false });
			}

			// 构建 WHERE 子句
			let mut where_clause = format!(
				"WHERE closed = false AND resolved = false AND (end_date IS NULL OR end_date > NOW()) AND created_at < NOW() - INTERVAL '{} seconds' AND id = ANY($1)",
				crate::consts::EVENT_DISPLAY_DELAY_SECS
			);
			let mut bind_values: Vec<String> = Vec::new();

			if let Some(topic) = &params.topic {
				where_clause.push_str(&format!(" AND topic = ${}", bind_values.len() + 2));
				bind_values.push(topic.clone());
			}

			// 查询所有符合条件的 events
			let data_sql = format!("SELECT * FROM events {}", where_clause);
			let all_events: Vec<Events> = if bind_values.is_empty() {
				sqlx::query_as(&data_sql).bind(&event_ids).fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!("Failed to query events: {}", e))?
			} else {
				let mut query = sqlx::query_as(&data_sql).bind(&event_ids);
				for value in &bind_values {
					query = query.bind(value);
				}
				query.fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!("Failed to query events: {}", e))?
			};

			// 构建 event_id -> Events 映射
			let mut events_map: std::collections::HashMap<i64, Events> = std::collections::HashMap::new();
			for event in all_events {
				events_map.insert(event.id, event);
			}

			// 过滤出 event 存在的 streamer-event 对
			let mut valid_pairs: Vec<(crate::api_types::LiveRoomRow, i64, Events)> = Vec::new();
			for live_room_row in live_rooms {
				if let Some(event_id_str) = live_room_row.event_id.as_ref()
					&& let Ok(event_id) = event_id_str.parse::<i64>()
					&& let Some(event) = events_map.get(&event_id)
				{
					valid_pairs.push((live_room_row, event_id, event.clone()));
				}
			}

			// 应用排序
			if params.ending_soon == Some(true) {
				valid_pairs.sort_by(|a, b| {
					let event_a = &a.2;
					let event_b = &b.2;
					let end_a = event_a.end_date.map(|d| d.timestamp()).unwrap_or(i64::MAX);
					let end_b = event_b.end_date.map(|d| d.timestamp()).unwrap_or(i64::MAX);
					end_a.cmp(&end_b)
				});
			} else if params.newest == Some(true) {
				valid_pairs.sort_by(|a, b| {
					let event_a = &a.2;
					let event_b = &b.2;
					event_b.created_at.cmp(&event_a.created_at)
				});
			} else {
				// 默认按交易量降序
				valid_pairs.sort_by(|a, b| {
					let event_a = &a.2;
					let event_b = &b.2;
					event_b.volume.cmp(&event_a.volume)
				});
			}

			// 计算总数
			let total = valid_pairs.len() as i16;

			// 应用分页
			let offset = ((params.page - 1).max(0) as usize) * (params.page_size as usize);
			let limit = params.page_size as usize;
			let paginated_pairs: Vec<_> = valid_pairs.into_iter().skip(offset).take(limit).collect();

			// 获取 Redis 连接，批量获取价格信息
			let mut conn = redis_pool::get_cache_redis_connection().await.map_err(|e| anyhow::anyhow!("Failed to get redis connection: {}", e))?;

			// 收集所有需要查询的 field
			let mut fields: Vec<String> = Vec::new();
			for (_, _, event) in &paginated_pairs {
				for market_id_str in event.markets.keys() {
					let market_id: i16 = market_id_str.parse().unwrap_or(0);
					fields.push(market_field(event.id, market_id));
				}
			}

			// 批量 HMGET 获取价格信息
			let price_values: Vec<Option<String>> = if !fields.is_empty() { conn.hget(PRICE_CACHE_KEY, &fields).await.unwrap_or_else(|_| vec![None; fields.len()]) } else { Vec::new() };

			// 构建 field -> price_info 映射
			let mut price_map: std::collections::HashMap<String, CacheMarketPriceInfo> = std::collections::HashMap::new();
			for (field, value) in fields.iter().zip(price_values.iter()) {
				if let Some(v) = value
					&& let Ok(info) = serde_json::from_str::<CacheMarketPriceInfo>(v)
				{
					price_map.insert(field.clone(), info);
				}
			}

			// 转换为响应格式
			let mut streamer_responses = Vec::new();
			for (live_room_row, _event_id, event) in paginated_pairs {
				// 构建 event 的 markets
				let mut markets = Vec::new();
				for (market_id_str, market) in event.markets.iter() {
					let market_id: i16 = market_id_str.parse().unwrap_or(0);
					let field = market_field(event.id, market_id);
					let price_info = price_map.get(&field);

					// 获取 token 0 和 token 1 的价格信息
					let (outcome_0_chance, outcome_1_chance, outcome_0_best_bid, outcome_0_best_ask, outcome_1_best_bid, outcome_1_best_ask) = if let Some(info) = price_info {
						let token_0_id = market.token_ids.first().cloned().unwrap_or_default();
						let token_1_id = market.token_ids.get(1).cloned().unwrap_or_default();

						let token_0_info = info.prices.get(&token_0_id);
						let token_1_info = info.prices.get(&token_1_id);

						let (t0_bid, t0_ask, t0_ltp) = token_0_info.map(|i| (i.best_bid.as_str(), i.best_ask.as_str(), i.latest_trade_price.as_str())).unwrap_or(("0", "0", ""));
						let (t1_bid, t1_ask, t1_ltp) = token_1_info.map(|i| (i.best_bid.as_str(), i.best_ask.as_str(), i.latest_trade_price.as_str())).unwrap_or(("0", "0", ""));

						(calculate_chance(t0_bid, t0_ask, t0_ltp), calculate_chance(t1_bid, t1_ask, t1_ltp), t0_bid.to_string(), t0_ask.to_string(), t1_bid.to_string(), t1_ask.to_string())
					} else {
						("0".to_string(), "0".to_string(), "0".to_string(), "0".to_string(), "0".to_string(), "0".to_string())
					};

					markets.push(EventMarketResponse {
						market_id,
						title: market.title.clone(),
						question: market.question.clone(),
						outcome_0_name: market.outcomes.first().cloned().unwrap_or_default(),
						outcome_1_name: market.outcomes.get(1).cloned().unwrap_or_default(),
						outcome_0_token_id: market.token_ids.first().cloned().unwrap_or_default(),
						outcome_1_token_id: market.token_ids.get(1).cloned().unwrap_or_default(),
						outcome_0_chance,
						outcome_1_chance,
						outcome_0_best_bid,
						outcome_0_best_ask,
						outcome_1_best_bid,
						outcome_1_best_ask,
					});
				}

				// 按 market_id 从小到大排序
				markets.sort_by_key(|m| m.market_id);

				// room_id 就是 streamer_uid
				let streamer_uid = live_room_row.room_id.clone().unwrap_or_default();
				let live_room = live_room_row.clone();

				streamer_responses.push(StreamerEventResponse {
					streamer_uid: streamer_uid.clone(),
					live_room,
					user: None, // 将在后面批量查询并填充
					event: SingleEventResponse {
						event_id: event.id,
						slug: event.slug,
						image: event.image,
						title: event.title,
						volume: event.volume.normalize(),
						topic: event.topic,
						markets,
						has_streamer: live_room_row.status.as_ref().map(|s| s == "LIVE").unwrap_or(false),
					},
				});
			}

			// 批量查询用户信息（通过 streamerUid，假设 streamerUid 就是 privy_id）
			let streamer_uids: Vec<String> = streamer_responses.iter().map(|s| s.streamer_uid.clone()).collect();
			let mut users_map: std::collections::HashMap<String, common::model::Users> = std::collections::HashMap::new();

			if !streamer_uids.is_empty() {
				let query = "SELECT * FROM users WHERE privy_id = ANY($1)";
				let users: Vec<common::model::Users> = sqlx::query_as(query).bind(&streamer_uids).fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!("Failed to query users: {}", e))?;

				for user in users {
					users_map.insert(user.privy_id.clone(), user);
				}
			}

			// 填充用户信息
			for streamer_response in &mut streamer_responses {
				if let Some(user) = users_map.get(&streamer_response.streamer_uid) {
					streamer_response.user = Some(StreamerUserInfo::from_user(user));
				}
			}

			let has_more = (params.page as i64 * params.page_size as i64) < total as i64;
			Ok(LivesResponse { streamers: streamer_responses, total, has_more })
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Singleflight lives leader failed: {}", e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight lives leader dropped");
			Err(InternalError)
		}
	}
}

/// 获取 event 对应的正在直播的主播列表，包含直播间人数
///
/// 流程：
/// 0. 检查 event 是否存在且未关闭/解决/过期
/// 1. 从 PostgreSQL 的 live_rooms 表查询该 event 关联的所有 status 为 "LIVE" 的记录
/// 2. 并发获取每个主播的直播间人数（限制 200/s）
/// 3. 按直播间人数从高到低排序返回
///
/// 使用 singleflight 确保并发请求共享同一个查询
pub async fn handle_event_streamers(Query(params): Query<EventStreamersRequest>) -> Result<Json<ApiResponse<EventStreamersResponse>>, InternalError> {
	let event_id = params.event_id;
	let singleflight_key = format!("event_streamers:{}", event_id);

	let group = get_event_streamers_group().await;
	let result = group
		.work(&singleflight_key, async move {
			let read_pool = db::get_db_read_pool().map_err(|e| anyhow::anyhow!("Failed to get db read pool: {}", e))?;

			// 0. 检查 event 是否存在且未关闭/解决/过期
			let event_exists: Option<bool> =
				sqlx::query_scalar("SELECT true FROM events WHERE id = $1 AND closed = false AND resolved = false AND (end_date IS NULL OR end_date > NOW())")
					.bind(event_id)
					.fetch_optional(&read_pool)
					.await
					.map_err(|e| anyhow::anyhow!("Failed to check event status: {}", e))?;

			if event_exists.is_none() {
				return Ok(EventStreamersResponse { streamers: Vec::new() });
			}

			// 1. 从 PostgreSQL 查询该 event 关联的所有 LIVE 状态的直播间

			let event_id_str = event_id.to_string();
			let live_rooms: Vec<crate::api_types::LiveRoomRow> = sqlx::query_as(
				"SELECT id, room_id, room_name, room_desc, status, event_id, create_date::timestamptz, picture_url, created_at::timestamptz, updated_at::timestamptz FROM live_rooms WHERE event_id = $1 AND status = 'LIVE'",
			)
			.bind(&event_id_str)
			.fetch_all(&read_pool)
			.await
			.map_err(|e| anyhow::anyhow!("Failed to query live_rooms: {}", e))?;

			if live_rooms.is_empty() {
				return Ok(EventStreamersResponse { streamers: Vec::new() });
			}

			// 2. 并发获取每个主播的直播间人数（限制 200/s）
			// 使用令牌桶限制每秒最多 200 个请求
			use tokio::{
				sync::Semaphore,
				time::{interval, Duration},
			};

			let semaphore = Arc::new(Semaphore::new(200));
			let mut interval_timer = interval(Duration::from_millis(5)); // 每 5ms 释放一个令牌（即每秒 200 个）
			interval_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

			let mut tasks = Vec::new();

			for live_room_row in &live_rooms {
				// 等待令牌（每 5ms 释放一个，实现 200/s 的限制）
				interval_timer.tick().await;
				let permit = semaphore.clone().acquire_owned().await.map_err(|e| anyhow::anyhow!("Failed to acquire semaphore permit: {}", e))?;

				// room_id 就是 streamer_uid，也是直播间 ID
				let room_id = live_room_row.room_id.clone().unwrap_or_default();
				let live_room_row_clone = live_room_row.clone();
				let streamer_uid = room_id.clone();

				let task = tokio::spawn(async move {
					let _permit = permit;

					// 使用 room_id 获取在线人数
					let viewer_count = match rpc_chat_service::get_online_member_num(&room_id).await {
						Ok(count) => count,
						Err(e) => {
							tracing::warn!("Failed to get online member num for streamer {} (room_id: {}): {}", streamer_uid, room_id, e);
							0
						}
					};

					(streamer_uid, live_room_row_clone, viewer_count)
				});

				tasks.push(task);
			}

			// 等待所有任务完成
			let mut streamer_results = Vec::new();
			for task in tasks {
				match task.await {
					Ok(result) => streamer_results.push(result),
					Err(e) => {
						tracing::error!("Task failed: {}", e);
					}
				}
			}

			// 3. 按直播间人数从高到低排序
			streamer_results.sort_by(|a, b| b.2.cmp(&a.2));

			// 4. 批量查询用户信息（通过 streamerUid，假设 streamerUid 就是 privy_id）
			let streamer_uids: Vec<String> = streamer_results.iter().map(|(uid, ..)| uid.clone()).collect();
			let mut users_map: std::collections::HashMap<String, common::model::Users> = std::collections::HashMap::new();

			if !streamer_uids.is_empty() {
				// 批量查询用户信息（streamerUid 作为 privy_id）
				let query = "SELECT * FROM users WHERE privy_id = ANY($1)";
				let users: Vec<common::model::Users> =
					sqlx::query_as(query).bind(&streamer_uids).fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!("Failed to query users: {}", e))?;

				for user in users {
					users_map.insert(user.privy_id.clone(), user);
				}
			}

			// 转换为响应格式
			let mut streamers = Vec::new();
			for (streamer_uid, live_room_row, viewer_count) in streamer_results {
				let live_room = live_room_row.clone();

				// 获取用户信息
				let user = users_map.get(&streamer_uid).map(StreamerUserInfo::from_user);

				streamers.push(StreamerWithViewerCount { streamer_uid: streamer_uid.clone(), viewer_count, live_room, user });
			}

			Ok(EventStreamersResponse { streamers })
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Singleflight event_streamers leader failed: {}", e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight event_streamers leader dropped");
			Err(InternalError)
		}
	}
}

/// 获取主播详情（包含用户信息、直播间信息和事件信息）
///
/// 流程：
/// 1. 从 PostgreSQL 的 live_rooms 表查询该 streamerUid（使用 room_id 字段）的直播间信息
/// 2. 从 live_rooms 表获取 event_id
/// 3. 从 PostgreSQL 查询对应的 event 信息（如果存在）
/// 4. 从 PostgreSQL 查询用户信息（通过 streamerUid 作为 privy_id）
///
/// 使用 singleflight 确保并发请求共享同一个查询
pub async fn handle_streamer_detail(Query(params): Query<StreamerDetailRequest>) -> Result<Json<ApiResponse<StreamerDetailResponse>>, InternalError> {
	let streamer_uid = params.streamer_uid.clone();
	let singleflight_key = format!("streamer_detail:{}", streamer_uid);

	let group = get_streamer_detail_group().await;
	let result = group
		.work(&singleflight_key, async move {
			let streamer_uid = params.streamer_uid;

			// 1. 从 PostgreSQL 查询 live_rooms 表获取直播间信息（room_id 就是 streamer_uid）
			let read_pool = db::get_db_read_pool().map_err(|e| anyhow::anyhow!("Failed to get db read pool: {}", e))?;

			let live_room_row: Option<crate::api_types::LiveRoomRow> = sqlx::query_as(
				"SELECT id, room_id, room_name, room_desc, status, event_id, create_date::timestamptz, picture_url, created_at::timestamptz, updated_at::timestamptz FROM live_rooms WHERE room_id = $1",
			)
			.bind(&streamer_uid)
			.fetch_optional(&read_pool)
			.await
			.map_err(|e| anyhow::anyhow!("Failed to query live_rooms: {}", e))?;

			// 2. 从 live_room_row 获取 event_id
			let event_id = live_room_row.as_ref().and_then(|row| row.event_id.as_ref().and_then(|eid| eid.parse::<i64>().ok()));
			let live_room = live_room_row.clone();

			// 3. 从 PostgreSQL 查询事件信息（如果 event_id 存在，且未关闭、未解决、未过期）
			let event_response = if let Some(eid) = event_id {
				let event: Option<Events> = sqlx::query_as("SELECT * FROM events WHERE id = $1 AND closed = false AND resolved = false AND (end_date IS NULL OR end_date > NOW())")
					.bind(eid)
					.fetch_optional(&read_pool)
					.await
					.map_err(|e| anyhow::anyhow!("Failed to query event: {}", e))?;

				if let Some(event) = event {
					// 获取价格信息
					let mut conn = redis_pool::get_cache_redis_connection().await.map_err(|e| anyhow::anyhow!("Failed to get redis connection: {}", e))?;

					// 收集需要查询的 price fields
					let price_fields: Vec<String> = event
						.markets
						.keys()
						.map(|market_id_str| {
							let market_id: i16 = market_id_str.parse().unwrap_or(0);
							common::key::market_field(event.id, market_id)
						})
						.collect();

					// 批量获取价格信息
					let price_values: Vec<Option<String>> =
						if !price_fields.is_empty() { conn.hget(common::key::PRICE_CACHE_KEY, &price_fields).await.unwrap_or_else(|_| vec![None; price_fields.len()]) } else { Vec::new() };

					// 构建 field -> price_info 映射
					let mut price_map: std::collections::HashMap<String, CacheMarketPriceInfo> = std::collections::HashMap::new();
					for (field, value) in price_fields.iter().zip(price_values.iter()) {
						if let Some(v) = value
							&& let Ok(info) = serde_json::from_str::<CacheMarketPriceInfo>(v)
						{
							price_map.insert(field.clone(), info);
						}
					}

					// 转换为响应格式
					let mut markets = Vec::new();
					for (market_id_str, market) in event.markets.iter() {
						let market_id: i16 = market_id_str.parse().unwrap_or(0);
						let field = common::key::market_field(event.id, market_id);
						let price_info = price_map.get(&field);

						// 获取 token 0 和 token 1 的价格信息
						let (outcome_0_chance, outcome_1_chance, outcome_0_best_bid, outcome_0_best_ask, outcome_1_best_bid, outcome_1_best_ask) = if let Some(info) = price_info {
							let token_0_id = market.token_ids.first().cloned().unwrap_or_default();
							let token_1_id = market.token_ids.get(1).cloned().unwrap_or_default();

							let token_0_info = info.prices.get(&token_0_id);
							let token_1_info = info.prices.get(&token_1_id);

							let (t0_bid, t0_ask, t0_ltp) = token_0_info.map(|i| (i.best_bid.as_str(), i.best_ask.as_str(), i.latest_trade_price.as_str())).unwrap_or(("0", "0", ""));
							let (t1_bid, t1_ask, t1_ltp) = token_1_info.map(|i| (i.best_bid.as_str(), i.best_ask.as_str(), i.latest_trade_price.as_str())).unwrap_or(("0", "0", ""));

							(calculate_chance(t0_bid, t0_ask, t0_ltp), calculate_chance(t1_bid, t1_ask, t1_ltp), t0_bid.to_string(), t0_ask.to_string(), t1_bid.to_string(), t1_ask.to_string())
						} else {
							("0".to_string(), "0".to_string(), "0".to_string(), "0".to_string(), "0".to_string(), "0".to_string())
						};

						markets.push(EventMarketResponse {
							market_id,
							title: market.title.clone(),
							question: market.question.clone(),
							outcome_0_name: market.outcomes.first().cloned().unwrap_or_default(),
							outcome_1_name: market.outcomes.get(1).cloned().unwrap_or_default(),
							outcome_0_token_id: market.token_ids.first().cloned().unwrap_or_default(),
							outcome_1_token_id: market.token_ids.get(1).cloned().unwrap_or_default(),
							outcome_0_chance,
							outcome_1_chance,
							outcome_0_best_bid,
							outcome_0_best_ask,
							outcome_1_best_bid,
							outcome_1_best_ask,
						});
					}

					// 按 market_id 从小到大排序
					markets.sort_by_key(|m| m.market_id);

					Some(SingleEventResponse {
						event_id: event.id,
						slug: event.slug,
						image: event.image,
						title: event.title,
						volume: event.volume.normalize(),
						topic: event.topic,
						markets,
						has_streamer: true, // 因为是通过 streamerUid 查询的，所以肯定有主播
					})
				} else {
					None
				}
			} else {
				None
			};

			// 4. 从 PostgreSQL 查询用户信息（通过 streamerUid 作为 privy_id）
			let user: Option<Users> =
				sqlx::query_as("SELECT * FROM users WHERE privy_id = $1").bind(&streamer_uid).fetch_optional(&read_pool).await.map_err(|e| anyhow::anyhow!("Failed to query user: {}", e))?;

			let user_info = user.as_ref().map(StreamerUserInfo::from_user);

			Ok(StreamerDetailResponse { streamer_uid, live_room, user: user_info, event: event_response })
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Singleflight streamer_detail leader failed: {}", e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight streamer_detail leader dropped");
			Err(InternalError)
		}
	}
}

/// 获取主播直播间在线观看人数
pub async fn handle_streamer_viewer_count(Query(params): Query<StreamerViewerCountRequest>) -> Result<Json<ApiResponse<StreamerViewerCountResponse>>, InternalError> {
	let streamer_id = params.streamer_id;

	let result = get_streamer_viewer_count_group()
		.await
		.work(&streamer_id, async {
			let viewer_count = rpc_chat_service::get_online_member_num(&streamer_id).await.map_err(|e| anyhow::anyhow!(e))?;
			Ok(viewer_count)
		})
		.await;

	match result {
		Ok(viewer_count) => Ok(Json(ApiResponse::success(StreamerViewerCountResponse { viewer_count }))),
		Err(Some(e)) => {
			tracing::error!("Failed to get streamer viewer count: {}", e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight streamer_viewer_count leader dropped");
			Err(InternalError)
		}
	}
}

/// 获取用户 USDC 余额
pub async fn handle_balance(Query(params): Query<BalanceRequest>) -> Result<Json<ApiResponse<BalanceResponse>>, InternalError> {
	let user_id = params.uid;
	let group = singleflight::get_balance_group().await;
	let key = user_id.to_string();

	let result = group
		.work(&key, async move {
			let read_pool = db::get_db_read_pool()?;
			let balance: Option<Decimal> =
				sqlx::query_scalar("SELECT balance FROM positions WHERE user_id = $1 AND token_id = 'usdc'").bind(user_id).fetch_optional(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?;
			Ok(balance.unwrap_or(Decimal::ZERO))
		})
		.await;

	match result {
		Ok(balance) => Ok(Json(ApiResponse::success(BalanceResponse { balance }))),
		Err(Some(e)) => {
			tracing::error!("Failed to get balance for user {}: {}", user_id, e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight balance leader dropped for user {}", user_id);
			Err(InternalError)
		}
	}
}

/// 获取用户积分信息
/// GET /points?uid=xxx
/// 返回 boost_points, invite_points, total_points, rank
pub async fn handle_points(Query(params): Query<PointsRequest>) -> Result<Json<ApiResponse<PointsResponse>>, InternalError> {
	let user_id = params.uid;
	let group = get_points_group().await;
	let key = user_id.to_string();

	let result = group
		.work(&key, async move {
			let read_pool = db::get_db_read_pool()?;
			let row: Option<(i64, i64, i64)> = sqlx::query_as("SELECT boost_points, invite_earned_points, total_points FROM points WHERE user_id = $1")
				.bind(user_id)
				.fetch_optional(&read_pool)
				.await
				.map_err(|e| anyhow::anyhow!(e))?;

			let (boost_points, invite_points, total_points) = row.unwrap_or((0, 0, 0));

			// 计算排名: 查询前200名用户ID，按 total_points 降序排列
			let top_200: Vec<i64> = sqlx::query_scalar("SELECT user_id FROM points ORDER BY total_points DESC LIMIT 200").fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?;

			// 查找用户在列表中的位置
			let rank_str = match top_200.iter().position(|&uid| uid == user_id) {
				Some(pos) => (pos + 1).to_string(), // position 是 0-based，排名是 1-based
				None => ">200".to_string(),
			};

			Ok(PointsResponse { boost_points, invite_points, total_points, rank: rank_str })
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Failed to get points for user {}: {}", user_id, e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight points leader dropped for user {}", user_id);
			Err(InternalError)
		}
	}
}

/// 获取用户简单信息（公开接口，使用 uid 参数）
pub async fn handle_simple_info(Query(params): Query<SimpleInfoRequest>) -> Result<Json<ApiResponse<SimpleInfoResponse>>, InternalError> {
	let user_id = params.uid;
	let group = get_simple_info_group().await;
	let key = user_id.to_string();

	let result = group
		.work(&key, async move {
			let read_pool = db::get_db_read_pool()?;
			let row: Option<(String, String)> = sqlx::query_as("SELECT name, profile_image FROM users WHERE id = $1").bind(user_id).fetch_optional(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?;

			Ok(match row {
				Some((name, profile_image)) => SimpleInfoResponse { name, profile_image },
				None => SimpleInfoResponse { name: String::new(), profile_image: String::new() },
			})
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Failed to get simple info for user {}: {}", user_id, e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight simple info leader dropped for user {}", user_id);
			Err(InternalError)
		}
	}
}

/// 获取用户赛季积分和交易量（需要认证）
pub async fn handle_user_points_volumes(
	Extension(client_info): Extension<ClientInfo>,
	Query(params): Query<UserPointsVolumesRequest>,
) -> Result<Json<ApiResponse<UserPointsVolumesResponse>>, InternalError> {
	// 鉴权
	let auth_result = match get_user_id_from_auth(&client_info, &client_info.request_id).await {
		Ok(result) => result,
		Err(e) => return Ok(Json(ApiResponse::error(e.to_api_error_code()))),
	};
	let user_id = auth_result.user_id;

	// 获取当前赛季ID
	let current_season_id = get_current_season_id().await.map_err(|e| {
		tracing::error!("Failed to get current season: {}", e);
		InternalError
	})?;

	// 确定查询的赛季ID
	let (query_season_id, is_current_season) = match params.season_id {
		Some(season_id) => {
			// 传了赛季ID，检查是否存在
			let exists = season_exists(season_id).await.map_err(|e| {
				tracing::error!("Failed to check season exists: {}", e);
				InternalError
			})?;
			if !exists {
				return Ok(Json(ApiResponse::error(ApiErrorCode::SeasonNotFound)));
			}
			(season_id, current_season_id == Some(season_id))
		}
		None => {
			// 没传赛季ID，使用当前赛季
			match current_season_id {
				Some(id) => (id, true),
				None => return Ok(Json(ApiResponse::error(ApiErrorCode::SeasonNotFound))),
			}
		}
	};

	let key = format!("{}:{}", user_id, query_season_id);
	let group = get_user_points_volumes_group().await;

	let result = group
		.work(&key, async move {
			let read_pool = db::get_db_read_pool()?;

			if is_current_season {
				// 当前赛季从 points 表获取
				let row: Option<(i64, Decimal)> =
					sqlx::query_as("SELECT total_points, accumulated_volume FROM points WHERE user_id = $1").bind(user_id).fetch_optional(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?;

				Ok(match row {
					Some((total_points, accumulated_volume)) => UserPointsVolumesResponse { total_points, accumulated_volume },
					None => UserPointsVolumesResponse { total_points: 0, accumulated_volume: Decimal::ZERO },
				})
			} else {
				// 历史赛季从 season_history 表获取
				let row: Option<(i64, Decimal)> = sqlx::query_as("SELECT total_points, accumulated_volume FROM season_history WHERE user_id = $1 AND season_id = $2")
					.bind(user_id)
					.bind(query_season_id)
					.fetch_optional(&read_pool)
					.await
					.map_err(|e| anyhow::anyhow!(e))?;

				Ok(match row {
					Some((total_points, accumulated_volume)) => UserPointsVolumesResponse { total_points, accumulated_volume },
					None => UserPointsVolumesResponse { total_points: 0, accumulated_volume: Decimal::ZERO },
				})
			}
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Failed to get user points volumes for user {}, season {}: {}", user_id, query_season_id, e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight user points volumes leader dropped for user {}, season {}", user_id, query_season_id);
			Err(InternalError)
		}
	}
}
