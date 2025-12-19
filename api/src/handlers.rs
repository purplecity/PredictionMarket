use {
	crate::{
		api_error::{ApiErrorCode, InternalError},
		api_types::{
			ApiResponse, CancelAllOrdersResponse, CancelOrderResponse, EventDetailRequest, EventDetailResponse, EventMarketDetail, EventMarketResponse, EventsRequest, EventsResponse,
			GetImageSignRequest, PlaceOrderRequest, PlaceOrderResponse, SingleEventResponse, UserDataResponse, UserImageRequest, UserProfileRequest,
		},
		cache::{self, CacheError},
		db,
		internal_service::{ReqGetGoogleImageSign, ReqGetIpInfo, ReqGetPrivyUserInfo, ResGetGoogleImageSign, get_google_image_sign, get_ip_info, get_privy_user_info},
		rpc_client,
		server::ClientInfo,
		singleflight::{get_event_detail_group, get_events_group, get_market_validation_group, get_topics_group},
	},
	axum::{
		extract::{Extension, Query},
		response::Json,
	},
	chrono::Utc,
	common::{
		consts::{NEW_USER_MSG_KEY, NEW_USER_STREAM, ORDER_INPUT_MSG_KEY, ORDER_INPUT_STREAM},
		depth_types::CacheMarketPriceInfo,
		engine_types::{CancelOrderMessage, OrderInputMessage, OrderSide as EngineOrderSide, OrderStatus, OrderType, PredictionSymbol, SubmitOrderMessage},
		key::{PRICE_CACHE_KEY, market_field},
		model::{Events, SignatureOrderMsg, Users},
		redis_pool,
	},
	redis::AsyncCommands,
	rust_decimal::{Decimal, prelude::ToPrimitive},
	std::str::FromStr,
	tracing::info,
};

/// 更新用户头像
pub async fn handle_user_image(Extension(client_info): Extension<ClientInfo>, Json(params): Json<UserImageRequest>) -> Result<Json<ApiResponse<()>>, InternalError> {
	let privy_id = match &client_info.privy_id {
		Some(id) => id.clone(),
		None => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::AuthFailed)));
		}
	};

	if let Err(err_msg) = params.validate() {
		return Ok(Json(ApiResponse::customer_error(err_msg)));
	}

	let user_id = match cache::get_user_id_by_privy_id(&privy_id).await {
		Ok(id) => id,
		Err(CacheError::UserNotFound(_)) => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::UserNotFound)));
		}
		Err(e) => {
			tracing::error!("request_id={}, privy_id={} - Failed to get user id from cache: {}", client_info.request_id, privy_id, e);
			return Err(InternalError);
		}
	};

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
	let privy_id = match &client_info.privy_id {
		Some(id) => id.clone(),
		None => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::AuthFailed)));
		}
	};

	if let Err(err_msg) = params.validate() {
		return Ok(Json(ApiResponse::customer_error(err_msg)));
	}

	let user_id = match cache::get_user_id_by_privy_id(&privy_id).await {
		Ok(id) => id,
		Err(CacheError::UserNotFound(_)) => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::UserNotFound)));
		}
		Err(e) => {
			tracing::error!("Failed to get user id from cache: {}", e);
			return Err(InternalError);
		}
	};

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
	let privy_id = match &client_info.privy_id {
		Some(id) => id.clone(),
		None => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::AuthFailed)));
		}
	};

	if let Err(err_msg) = params.validate() {
		return Ok(Json(ApiResponse::customer_error(err_msg)));
	}

	// 打印请求参数日志
	info!("[PlaceOrder] request_id={}, privy_id={}, params={:?}", client_info.request_id, privy_id, params);

	// 1. 获取用户 ID
	let user_id = match cache::get_user_id_by_privy_id(&privy_id).await {
		Ok(id) => id,
		Err(CacheError::UserNotFound(_)) => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::UserNotFound)));
		}
		Err(e) => {
			tracing::error!("request_id={}, privy_id={} - Failed to get user id from cache: {}", client_info.request_id, privy_id, e);
			return Err(InternalError);
		}
	};

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
	let privy_id = match &client_info.privy_id {
		Some(id) => id.clone(),
		None => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::AuthFailed)));
		}
	};

	// 打印请求参数日志
	info!("[CancelOrder] request_id={}, privy_id={}, order_id={}", client_info.request_id, privy_id, params.order_id);

	let user_id = match cache::get_user_id_by_privy_id(&privy_id).await {
		Ok(id) => id,
		Err(CacheError::UserNotFound(_)) => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::UserNotFound)));
		}
		Err(e) => {
			tracing::error!("request_id={}, privy_id={} - Failed to get user id from cache: {}", client_info.request_id, privy_id, e);
			return Err(InternalError);
		}
	};

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
	let privy_id = match &client_info.privy_id {
		Some(id) => id.clone(),
		None => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::AuthFailed)));
		}
	};

	// 打印请求参数日志
	info!("[CancelAllOrders] request_id={}, privy_id={}", client_info.request_id, privy_id);

	let user_id = match cache::get_user_id_by_privy_id(&privy_id).await {
		Ok(id) => id,
		Err(CacheError::UserNotFound(_)) => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::UserNotFound)));
		}
		Err(e) => {
			tracing::error!("request_id={}, privy_id={} - Failed to get user id from cache: {}", client_info.request_id, privy_id, e);
			return Err(InternalError);
		}
	};

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
pub async fn handle_user_data(Extension(client_info): Extension<ClientInfo>) -> Result<Json<ApiResponse<UserDataResponse>>, InternalError> {
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
			let privy_info = get_privy_user_info(ReqGetPrivyUserInfo { privy_id: privy_id.clone() }).await.map_err(|e| {
				tracing::error!("Failed to get privy user info: {}", e);
				InternalError
			})?;

			if privy_info.privy_evm_address.is_empty() {
				return Ok(Json(ApiResponse::error(ApiErrorCode::PrivyNoAddress)));
			}

			let region = match get_ip_info(ReqGetIpInfo { ip: client_info.ip.clone() }).await {
				Ok(ip_info) => ip_info.region,
				Err(e) => {
					tracing::warn!("Failed to get ip info for IP {}: {}, using empty region", client_info.ip, e);
					String::new()
				}
			};

			let now = Utc::now();
			let write_pool = db::get_db_write_pool().map_err(|e| {
				tracing::error!("{}", e);
				InternalError
			})?;
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
			.fetch_one(&write_pool)
			.await
			.map_err(|e| {
				tracing::error!("Failed to insert user: {}", e);
				InternalError
			})?;

			// 更新内存缓存
			cache::insert_user_id_cache(&privy_info.privy_id, user_id).await;

			let new_user_msg = common::onchain_msg_types::NewUser { user_id, privy_id: privy_info.privy_id.clone(), privy_evm_address: privy_info.privy_evm_address.clone() };
			if let Ok(msg_json) = serde_json::to_string(&new_user_msg)
				&& let Ok(mut conn) = redis_pool::get_common_mq_connection().await
			{
				let _: Result<String, _> = conn.xadd(NEW_USER_STREAM, "*", &[(NEW_USER_MSG_KEY, msg_json.as_str())]).await;
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

/// 获取用户图片上传签名
pub async fn handle_image_sign(Extension(client_info): Extension<ClientInfo>, Query(params): Query<GetImageSignRequest>) -> Result<Json<ApiResponse<ResGetGoogleImageSign>>, InternalError> {
	let privy_id = match &client_info.privy_id {
		Some(id) => id.clone(),
		None => {
			return Ok(Json(ApiResponse::error(ApiErrorCode::AuthFailed)));
		}
	};

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
		parts.push(topic.clone());
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

			// 1. 先查询总数
			let count_sql = format!("SELECT COUNT(*) as count FROM events {}", where_clause);
			let total: i64 = match bind_values.len() {
				0 => sqlx::query_scalar(&count_sql).fetch_one(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?,
				1 => sqlx::query_scalar(&count_sql).bind(&bind_values[0]).fetch_one(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?,
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

			// 转换为响应格式
			let mut event_responses = Vec::new();
			for event in events {
				let mut markets = Vec::new();

				for (market_id_str, market) in event.markets.iter() {
					let market_id: i16 = market_id_str.parse().unwrap_or(0);
					let field = market_field(event.id, market_id);
					let price_info = price_map.get(&field);

					// 获取 token 0 和 token 1 的价格信息
					let (outcome_0_chance, outcome_1_chance, _outcome_0_best_bid, _outcome_0_best_ask, _outcome_1_best_bid, _outcome_1_best_ask) = if let Some(info) = price_info {
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
					});
				}

				// 按 market_id 从小到大排序
				markets.sort_by_key(|m| m.market_id);

				event_responses.push(SingleEventResponse {
					event_id: event.id,
					slug: event.slug,
					image: event.image,
					title: event.title,
					volume: event.volume.normalize(),
					topic: event.topic,
					markets,
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
