use {
	crate::handlers,
	common::consts::USDC_TOKEN_ID,
	proto::asset_service_server::{AssetService, AssetServiceServer},
	rust_decimal::Decimal,
	sqlx::types::Uuid,
	std::{
		str::FromStr,
		sync::atomic::{AtomicBool, Ordering},
	},
	tonic::{Request, Response, Status},
	tracing::error,
};

static STOP_RECEIVING_RPC: AtomicBool = AtomicBool::new(false);

const SERVICE_SHUTTING_DOWN_MSG: &str = "Service is shutting down";

/// 设置停止接收 RPC 请求
pub fn set_stop_receiving_rpc(stop: bool) {
	STOP_RECEIVING_RPC.store(stop, Ordering::Release);
}

/// 检查是否停止接收 RPC 请求
fn is_stop_receiving_rpc() -> bool {
	STOP_RECEIVING_RPC.load(Ordering::Acquire)
}

pub struct AssetServiceImpl;

#[tonic::async_trait]
impl AssetService for AssetServiceImpl {
	/// Deposit RPC
	async fn deposit(&self, request: Request<proto::DepositRequest>) -> Result<Response<proto::Response>, Status> {
		if is_stop_receiving_rpc() {
			return Err(Status::unavailable(SERVICE_SHUTTING_DOWN_MSG));
		}

		let req = request.into_inner();

		// 参数验证和转换
		let amount = Decimal::from_str(&req.amount).map_err(|e| Status::invalid_argument(format!("Invalid amount: {}", e)))?;

		// 类型转换
		let market_id = i16::try_from(req.market_id).map_err(|e| Status::invalid_argument(format!("Invalid market_id: {}", e)))?;

		// 对于 USDC，event_id 和 market_id 应该为 None
		let (event_id, market_id) = if req.token_id == USDC_TOKEN_ID { (None, None) } else { (Some(req.event_id), Some(market_id)) };

		// 调用 handler (privy_id, outcome_name 用于创建position时使用)
		let privy_id = if req.privy_id.is_empty() { None } else { Some(req.privy_id.clone()) };
		let outcome_name = if req.outcome_name.is_empty() { None } else { Some(req.outcome_name.clone()) };
		match handlers::handle_deposit(req.user_id, &req.token_id, amount, &req.tx_hash, event_id, market_id, privy_id, outcome_name, None).await {
			Ok(_) => Ok(Response::new(proto::Response { success: true, reason: String::new() })),
			Err(e) => {
				error!("Deposit failed: user_id={}, token_id={}, amount={}, error={}", req.user_id, req.token_id, req.amount, e);
				Ok(Response::new(proto::Response { success: false, reason: e.to_string() }))
			}
		}
	}

	/// Withdraw RPC
	async fn withdraw(&self, request: Request<proto::WithdrawRequest>) -> Result<Response<proto::Response>, Status> {
		if is_stop_receiving_rpc() {
			return Err(Status::unavailable(SERVICE_SHUTTING_DOWN_MSG));
		}

		let req = request.into_inner();

		// 参数验证和转换
		let amount = Decimal::from_str(&req.amount).map_err(|e| Status::invalid_argument(format!("Invalid amount: {}", e)))?;
		let market_id = i16::try_from(req.market_id).map_err(|e| Status::invalid_argument(format!("Invalid market_id: {}", e)))?;

		// 对于 USDC，event_id 和 market_id 应该为 None
		let (event_id, market_id) = if req.token_id == USDC_TOKEN_ID { (None, None) } else { (Some(req.event_id), Some(market_id)) };

		// 调用 handler (privy_id, outcome_name 用于创建position时使用)
		let privy_id = if req.privy_id.is_empty() { None } else { Some(req.privy_id.clone()) };
		let outcome_name = if req.outcome_name.is_empty() { None } else { Some(req.outcome_name.clone()) };
		match handlers::handle_withdraw(req.user_id, &req.token_id, amount, &req.tx_hash, event_id, market_id, privy_id, outcome_name, None).await {
			Ok(_) => Ok(Response::new(proto::Response { success: true, reason: String::new() })),
			Err(e) => {
				error!("Withdraw failed: user_id={}, token_id={}, amount={}, error={}", req.user_id, req.token_id, req.amount, e);
				Ok(Response::new(proto::Response { success: false, reason: e.to_string() }))
			}
		}
	}

	/// CreateOrder RPC
	async fn create_order(&self, request: Request<proto::CreateOrderRequest>) -> Result<Response<proto::CreateOrderResponse>, Status> {
		if is_stop_receiving_rpc() {
			return Err(Status::unavailable(SERVICE_SHUTTING_DOWN_MSG));
		}

		let req = request.into_inner();

		// 参数验证和转换
		let price = Decimal::from_str(&req.price).map_err(|e| Status::invalid_argument(format!("Invalid price: {}", e)))?;
		let quantity = Decimal::from_str(&req.quantity).map_err(|e| Status::invalid_argument(format!("Invalid quantity: {}", e)))?;
		let volume = Decimal::from_str(&req.volume).map_err(|e| Status::invalid_argument(format!("Invalid volume: {}", e)))?;
		let market_id = i16::try_from(req.market_id).map_err(|e| Status::invalid_argument(format!("Invalid market_id: {}", e)))?;

		// 解析 order_side
		let order_side = match req.order_side.as_str() {
			common::consts::ORDER_SIDE_BUY => common::engine_types::OrderSide::Buy,
			common::consts::ORDER_SIDE_SELL => common::engine_types::OrderSide::Sell,
			_ => return Err(Status::invalid_argument(format!("Invalid order_side: {}", req.order_side))),
		};

		let order_type = match req.order_type.as_str() {
			common::consts::ORDER_TYPE_LIMIT => common::engine_types::OrderType::Limit,
			common::consts::ORDER_TYPE_MARKET => common::engine_types::OrderType::Market,
			_ => return Err(Status::invalid_argument(format!("Invalid order_type: {}", req.order_type))),
		};

		// 转换 SignatureOrderMsg
		let signature_order_msg = common::model::SignatureOrderMsg {
			expiration: req.signature_order_msg.as_ref().map(|s| s.expiration.clone()).unwrap_or_default(),
			fee_rate_bps: req.signature_order_msg.as_ref().map(|s| s.fee_rate_bps.clone()).unwrap_or_default(),
			maker: req.signature_order_msg.as_ref().map(|s| s.maker.clone()).unwrap_or_default(),
			maker_amount: req.signature_order_msg.as_ref().map(|s| s.maker_amount.clone()).unwrap_or_default(),
			nonce: req.signature_order_msg.as_ref().map(|s| s.nonce.clone()).unwrap_or_default(),
			salt: req.signature_order_msg.as_ref().map(|s| s.salt).unwrap_or_default(),
			side: req.signature_order_msg.as_ref().map(|s| s.side.clone()).unwrap_or_default(),
			signature: req.signature_order_msg.as_ref().map(|s| s.signature.clone()).unwrap_or_default(),
			signature_type: req.signature_order_msg.as_ref().map(|s| s.signature_type as i16).unwrap_or_default(),
			signer: req.signature_order_msg.as_ref().map(|s| s.signer.clone()).unwrap_or_default(),
			taker: req.signature_order_msg.as_ref().map(|s| s.taker.clone()).unwrap_or_default(),
			taker_amount: req.signature_order_msg.as_ref().map(|s| s.taker_amount.clone()).unwrap_or_default(),
			token_id: req.signature_order_msg.as_ref().map(|s| s.token_id.clone()).unwrap_or_default(),
		};

		// 调用 handler
		match handlers::handle_create_order(req.user_id, req.event_id, market_id, &req.token_id, &req.outcome, order_side, order_type, price, quantity, volume, signature_order_msg, None).await {
			Ok(order_id) => Ok(Response::new(proto::CreateOrderResponse { success: true, reason: String::new(), order_id: order_id.to_string(), update_id: 1 })),
			Err(e) => {
				error!("CreateOrder failed: user_id={}, error={}", req.user_id, e);
				Ok(Response::new(proto::CreateOrderResponse { success: false, reason: e.to_string(), order_id: String::new(), update_id: 0 }))
			}
		}
	}

	/// OrderRejected RPC
	async fn order_rejected(&self, request: Request<proto::OrderRejectedRequest>) -> Result<Response<proto::Response>, Status> {
		if is_stop_receiving_rpc() {
			return Err(Status::unavailable(SERVICE_SHUTTING_DOWN_MSG));
		}

		let req = request.into_inner();

		// 参数验证和转换
		let order_id = Uuid::parse_str(&req.order_id).map_err(|e| Status::invalid_argument(format!("Invalid order_id: {}", e)))?;

		// 调用 handler
		match handlers::handle_order_rejected(req.user_id, order_id, None).await {
			Ok(_) => Ok(Response::new(proto::Response { success: true, reason: String::new() })),
			Err(e) => {
				error!("OrderRejected failed: user_id={}, order_id={}, error={}", req.user_id, req.order_id, e);
				Ok(Response::new(proto::Response { success: false, reason: e.to_string() }))
			}
		}
	}

	/// CancelOrder RPC
	async fn cancel_order(&self, request: Request<proto::CancelOrderRequest>) -> Result<Response<proto::ResponseWithUpdateId>, Status> {
		if is_stop_receiving_rpc() {
			return Err(Status::unavailable(SERVICE_SHUTTING_DOWN_MSG));
		}

		let req = request.into_inner();

		// 参数验证和转换
		let cancelled_quantity = Decimal::from_str(&req.cancelled_quantity).map_err(|e| Status::invalid_argument(format!("Invalid cancelled_quantity: {}", e)))?;
		let volume = Decimal::from_str(&req.volume).map_err(|e| Status::invalid_argument(format!("Invalid volume: {}", e)))?;
		let order_id = Uuid::parse_str(&req.order_id).map_err(|e| Status::invalid_argument(format!("Invalid order_id: {}", e)))?;

		// 调用 handler
		match handlers::handle_cancel_order(req.user_id, order_id, cancelled_quantity, volume, None).await {
			Ok(update_id) => Ok(Response::new(proto::ResponseWithUpdateId { success: true, reason: String::new(), update_id })),
			Err(e) => {
				error!("CancelOrder failed: user_id={}, order_id={}, error={}", req.user_id, req.order_id, e);
				Ok(Response::new(proto::ResponseWithUpdateId { success: false, reason: e.to_string(), update_id: 0 }))
			}
		}
	}

	/// Split RPC
	async fn split(&self, request: Request<proto::SplitRequest>) -> Result<Response<proto::Response>, Status> {
		if is_stop_receiving_rpc() {
			return Err(Status::unavailable(SERVICE_SHUTTING_DOWN_MSG));
		}

		let req = request.into_inner();

		// 参数验证和转换
		let usdc_amount = Decimal::from_str(&req.usdc_amount).map_err(|e| Status::invalid_argument(format!("Invalid usdc_amount: {}", e)))?;
		let token_0_amount = Decimal::from_str(&req.token_0_amount).map_err(|e| Status::invalid_argument(format!("Invalid token_0_amount: {}", e)))?;
		let token_1_amount = Decimal::from_str(&req.token_1_amount).map_err(|e| Status::invalid_argument(format!("Invalid token_1_amount: {}", e)))?;

		// 调用 handler (假设 event_id 和 market_id 都是 0，实际应该从请求中获取)
		match handlers::handle_split(
			req.user_id,
			req.event_id,
			req.market_id as i16,
			usdc_amount,
			&req.token_0_id,
			&req.token_1_id,
			token_0_amount,
			token_1_amount,
			&req.tx_hash,
			&req.privy_id,
			&req.outcome_name_0,
			&req.outcome_name_1,
			None,
		)
		.await
		{
			Ok(_) => Ok(Response::new(proto::Response { success: true, reason: String::new() })),
			Err(e) => {
				error!("Split failed: user_id={}, error={}", req.user_id, e);
				Ok(Response::new(proto::Response { success: false, reason: e.to_string() }))
			}
		}
	}

	/// Merge RPC
	async fn merge(&self, request: Request<proto::MergeRequest>) -> Result<Response<proto::Response>, Status> {
		if is_stop_receiving_rpc() {
			return Err(Status::unavailable(SERVICE_SHUTTING_DOWN_MSG));
		}

		let req = request.into_inner();

		// 参数验证和转换
		let token_0_amount = Decimal::from_str(&req.token_0_amount).map_err(|e| Status::invalid_argument(format!("Invalid token_0_amount: {}", e)))?;
		let token_1_amount = Decimal::from_str(&req.token_1_amount).map_err(|e| Status::invalid_argument(format!("Invalid token_1_amount: {}", e)))?;
		let usdc_amount = Decimal::from_str(&req.usdc_amount).map_err(|e| Status::invalid_argument(format!("Invalid usdc_amount: {}", e)))?;

		// 调用 handler
		match handlers::handle_merge(
			req.user_id,
			req.event_id,
			req.market_id as i16,
			&req.token_0_id,
			&req.token_1_id,
			token_0_amount,
			token_1_amount,
			usdc_amount,
			&req.tx_hash,
			&req.privy_id,
			&req.outcome_name_0,
			&req.outcome_name_1,
			None,
		)
		.await
		{
			Ok(_) => Ok(Response::new(proto::Response { success: true, reason: String::new() })),
			Err(e) => {
				error!("Merge failed: user_id={}, error={}", req.user_id, e);
				Ok(Response::new(proto::Response { success: false, reason: e.to_string() }))
			}
		}
	}

	/// Redeem RPC
	async fn redeem(&self, request: Request<proto::RedeemRequest>) -> Result<Response<proto::Response>, Status> {
		if is_stop_receiving_rpc() {
			return Err(Status::unavailable(SERVICE_SHUTTING_DOWN_MSG));
		}

		let req = request.into_inner();

		// 参数验证和转换
		let usdc_amount = Decimal::from_str(&req.usdc_amount).map_err(|e| Status::invalid_argument(format!("Invalid usdc_amount: {}", e)))?;

		// 调用 handler
		match handlers::handle_redeem(
			req.user_id,
			req.event_id,
			req.market_id as i16,
			&req.token_id_0,
			&req.token_id_1,
			usdc_amount,
			&req.tx_hash,
			&req.privy_id,
			&req.outcome_name_0,
			&req.outcome_name_1,
			None,
		)
		.await
		{
			Ok(_) => Ok(Response::new(proto::Response { success: true, reason: String::new() })),
			Err(e) => {
				error!("Redeem failed: user_id={}, error={}", req.user_id, e);
				Ok(Response::new(proto::Response { success: false, reason: e.to_string() }))
			}
		}
	}

	/// Trade RPC
	async fn trade(&self, request: Request<proto::TradeRequest>) -> Result<Response<proto::TradeResponse>, Status> {
		if is_stop_receiving_rpc() {
			return Err(Status::unavailable(SERVICE_SHUTTING_DOWN_MSG));
		}

		let req = request.into_inner();

		// 解析 trade_id
		let trade_id = Uuid::parse_str(&req.trade_id).map_err(|e| Status::invalid_argument(format!("Invalid trade_id: {}", e)))?;

		// 解析 taker_trade_info
		let taker_proto = req.taker_trade_info.ok_or_else(|| Status::invalid_argument("Missing taker_trade_info"))?;
		let taker_order_id = Uuid::parse_str(&taker_proto.taker_order_id).map_err(|e| Status::invalid_argument(format!("Invalid taker_order_id: {}", e)))?;
		let taker_order_side = match taker_proto.taker_order_side.as_str() {
			common::consts::ORDER_SIDE_BUY => common::engine_types::OrderSide::Buy,
			common::consts::ORDER_SIDE_SELL => common::engine_types::OrderSide::Sell,
			_ => return Err(Status::invalid_argument(format!("Invalid taker_order_side: {}", taker_proto.taker_order_side))),
		};
		let taker_usdc_amount = Decimal::from_str(&taker_proto.taker_usdc_amount).map_err(|e| Status::invalid_argument(format!("Invalid taker_usdc_amount: {}", e)))?;
		let taker_token_amount = Decimal::from_str(&taker_proto.taker_token_amount).map_err(|e| Status::invalid_argument(format!("Invalid taker_token_amount: {}", e)))?;

		let taker = handlers::TakerTradeInfo { taker_id: taker_proto.taker_id, taker_order_id, taker_order_side, taker_token_id: taker_proto.taker_token_id, taker_usdc_amount, taker_token_amount };

		// 解析 maker_trade_infos
		let mut makers = Vec::new();
		for maker_proto in req.maker_trade_infos {
			let maker_order_id = Uuid::parse_str(&maker_proto.maker_order_id).map_err(|e| Status::invalid_argument(format!("Invalid maker_order_id: {}", e)))?;
			let maker_order_side = match maker_proto.maker_order_side.as_str() {
				common::consts::ORDER_SIDE_BUY => common::engine_types::OrderSide::Buy,
				common::consts::ORDER_SIDE_SELL => common::engine_types::OrderSide::Sell,
				_ => return Err(Status::invalid_argument(format!("Invalid maker_order_side: {}", maker_proto.maker_order_side))),
			};
			let maker_usdc_amount = Decimal::from_str(&maker_proto.maker_usdc_amount).map_err(|e| Status::invalid_argument(format!("Invalid maker_usdc_amount: {}", e)))?;
			let maker_token_amount = Decimal::from_str(&maker_proto.maker_token_amount).map_err(|e| Status::invalid_argument(format!("Invalid maker_token_amount: {}", e)))?;
			let maker_price = Decimal::from_str(&maker_proto.maker_price).map_err(|e| Status::invalid_argument(format!("Invalid maker_price: {}", e)))?;

			makers.push(handlers::MakerTradeInfo {
				maker_id: maker_proto.maker_id,
				maker_order_id,
				maker_order_side,
				maker_token_id: maker_proto.maker_token_id,
				maker_usdc_amount,
				maker_token_amount,
				maker_price,
			});
		}

		// 调用 handler
		match handlers::handle_trade(trade_id, req.timestamp, req.event_id, req.market_id as i16, taker, makers, None).await {
			Ok((signature_order_msgs, order_update_ids)) => {
				// 转换 SignatureOrderMsg 为 proto
				let proto_signature_msgs = signature_order_msgs
					.into_iter()
					.map(|msg| {
						proto::SignatureOrderMsg {
							expiration: msg.expiration,
							fee_rate_bps: msg.fee_rate_bps,
							maker: msg.maker,
							maker_amount: msg.maker_amount,
							nonce: msg.nonce,
							salt: msg.salt,
							side: msg.side,
							signature: msg.signature,
							signature_type: msg.signature_type as i32,
							signer: msg.signer,
							taker: msg.taker,
							taker_amount: msg.taker_amount,
							token_id: msg.token_id,
						}
					})
					.collect();

				Ok(Response::new(proto::TradeResponse { success: true, reason: String::new(), signature_order_msgs: proto_signature_msgs, order_update_ids }))
			}
			Err(e) => {
				error!("Trade failed: trade_id={}, error={}", req.trade_id, e);
				Ok(Response::new(proto::TradeResponse { success: false, reason: e.to_string(), signature_order_msgs: vec![], order_update_ids: vec![] }))
			}
		}
	}

	/// TradeOnchainSendResult RPC
	async fn trade_onchain_send_result(&self, request: Request<proto::TradeOnchainSendResultRequest>) -> Result<Response<proto::Response>, Status> {
		if is_stop_receiving_rpc() {
			return Err(Status::unavailable(SERVICE_SHUTTING_DOWN_MSG));
		}

		let req = request.into_inner();

		// 解析 trade_id
		let trade_id = Uuid::parse_str(&req.trade_id).map_err(|e| Status::invalid_argument(format!("Invalid trade_id: {}", e)))?;

		// 解析 taker 信息
		let taker_info = req.taker_trade_info.ok_or_else(|| Status::invalid_argument("Missing taker_trade_info"))?;
		let taker_side = match taker_info.taker_side.as_str() {
			common::consts::ORDER_SIDE_BUY => common::engine_types::OrderSide::Buy,
			common::consts::ORDER_SIDE_SELL => common::engine_types::OrderSide::Sell,
			_ => return Err(Status::invalid_argument(format!("Invalid taker_side: {}", taker_info.taker_side))),
		};
		let taker_usdc_amount = Decimal::from_str(&taker_info.taker_usdc_amount).map_err(|e| Status::invalid_argument(format!("Invalid taker_usdc_amount: {}", e)))?;
		let taker_token_amount = Decimal::from_str(&taker_info.taker_token_amount).map_err(|e| Status::invalid_argument(format!("Invalid taker_token_amount: {}", e)))?;
		let real_taker_usdc_amount = Decimal::from_str(&taker_info.real_taker_usdc_amount).map_err(|e| Status::invalid_argument(format!("Invalid real_taker_usdc_amount: {}", e)))?;
		let real_taker_token_amount = Decimal::from_str(&taker_info.real_taker_token_amount).map_err(|e| Status::invalid_argument(format!("Invalid real_taker_token_amount: {}", e)))?;
		let taker_unfreeze_amount = Decimal::from_str(&taker_info.taker_unfreeze_amount).map_err(|e| Status::invalid_argument(format!("Invalid taker_unfreeze_amount: {}", e)))?;
		let taker_order_id = Uuid::parse_str(&taker_info.taker_order_id).map_err(|e| Status::invalid_argument(format!("Invalid taker_order_id: {}", e)))?;

		// 解析 maker 信息
		let mut maker_infos = Vec::new();
		for maker_info in req.maker_trade_infos {
			let maker_side = match maker_info.maker_side.as_str() {
				common::consts::ORDER_SIDE_BUY => common::engine_types::OrderSide::Buy,
				common::consts::ORDER_SIDE_SELL => common::engine_types::OrderSide::Sell,
				_ => return Err(Status::invalid_argument(format!("Invalid maker_side: {}", maker_info.maker_side))),
			};
			let maker_usdc_amount = Decimal::from_str(&maker_info.maker_usdc_amount).map_err(|e| Status::invalid_argument(format!("Invalid maker_usdc_amount: {}", e)))?;
			let maker_token_amount = Decimal::from_str(&maker_info.maker_token_amount).map_err(|e| Status::invalid_argument(format!("Invalid maker_token_amount: {}", e)))?;
			let real_maker_usdc_amount = Decimal::from_str(&maker_info.real_maker_usdc_amount).map_err(|e| Status::invalid_argument(format!("Invalid real_maker_usdc_amount: {}", e)))?;
			let real_maker_token_amount = Decimal::from_str(&maker_info.real_maker_token_amount).map_err(|e| Status::invalid_argument(format!("Invalid real_maker_token_amount: {}", e)))?;
			let maker_order_id = Uuid::parse_str(&maker_info.maker_order_id).map_err(|e| Status::invalid_argument(format!("Invalid maker_order_id: {}", e)))?;

			maker_infos.push(handlers::MakerOnchainInfo {
				maker_side,
				maker_user_id: maker_info.maker_user_id,
				maker_usdc_amount,
				maker_token_amount,
				maker_token_id: maker_info.maker_token_id,
				maker_order_id,
				real_maker_usdc_amount,
				real_maker_token_amount,
				maker_privy_user_id: maker_info.maker_privy_user_id,
				maker_outcome_name: maker_info.maker_outcome_name,
			});
		}

		// 调用 handler
		match handlers::handle_trade_onchain_send_result(handlers::TradeOnchainSendParams {
			trade_id,
			event_id: req.event_id,
			market_id: req.market_id as i16,
			taker_side,
			taker_user_id: taker_info.taker_user_id,
			taker_usdc_amount,
			taker_token_amount,
			taker_token_id: &taker_info.taker_token_id,
			taker_order_id,
			real_taker_usdc_amount,
			real_taker_token_amount,
			taker_unfreeze_amount,
			taker_privy_user_id: &taker_info.taker_privy_user_id,
			taker_outcome_name: &taker_info.taker_outcome_name,
			maker_infos,
			tx_hash: &req.tx_hash,
			success: req.success,
			pool: None,
		})
		.await
		{
			Ok(_) => Ok(Response::new(proto::Response { success: true, reason: String::new() })),
			Err(e) => {
				error!("TradeOnchainSendResult failed: trade_id={}, error={}", req.trade_id, e);
				Ok(Response::new(proto::Response { success: false, reason: e.to_string() }))
			}
		}
	}
}

pub fn create_asset_service() -> AssetServiceServer<AssetServiceImpl> {
	AssetServiceServer::new(AssetServiceImpl)
}
