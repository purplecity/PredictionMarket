use {
	crate::{config, consts, rpc_client},
	common::{
		consts::{PROCESSOR_MSG_KEY, PROCESSOR_STREAM, TRADE_SEND_MSG_KEY, TRADE_SEND_STREAM, USER_EVENT_MSG_KEY, USER_EVENT_STREAM},
		onchain_msg_types::{MakerTradeInfo, MatchOrderInfo, TakerTradeInfo, TradeOnchainSendRequest},
		processor_types::{OrderCancelled, OrderRejected, OrderSubmitted, OrderTraded, ProcessorMessage, Trade},
		redis_pool,
		websocket_types::{OpenOrderChangeEvent, UserEvent, UserOpenOrders},
	},
	redis::AsyncCommands,
	rust_decimal::Decimal,
	std::str::FromStr,
	tracing::{error, info},
	uuid::Uuid,
};

pub async fn start_consumers() -> anyhow::Result<()> {
	let config = config::get_config();
	let consumer_count = config.processor_mq.consumer_count;
	let batch_size = config.processor_mq.batch_size;

	let group_name = consts::PROCESSOR_CONSUMER_GROUP;
	let consumer_name_prefix = consts::PROCESSOR_CONSUMER_NAME_PREFIX;

	// 确保消费者组存在 (从 engine_output_mq DB 1 读取)
	let mut conn = redis_pool::get_engine_output_mq_connection().await?;
	let res: redis::RedisResult<bool> = conn.xgroup_create_mkstream(PROCESSOR_STREAM, group_name, "0").await;
	if let Err(e) = res {
		error!("Failed to create processor consumer group: {}", e);
		// 忽略错误，因为组可能已经存在
	}

	// 在启动消费者之前，先认领所有 pending 消息
	let pending_messages = claim_all_pending_messages(&mut conn, group_name).await?;
	info!("Claimed {} pending messages on startup", pending_messages.len());

	// 处理 pending 消息
	for (_msg_id, message) in pending_messages {
		if let Err(e) = process_message_inner(message).await {
			error!("Failed to process pending message: {}", e);
		}
	}

	// 明确创建消费者并启动多个消费者任务
	let consumer_id = Uuid::new_v4().to_string();
	let mut started_consumers = 0;
	for i in 0..consumer_count {
		let consumer_id = format!("{}-{}-{}", consumer_name_prefix, consumer_id, i);

		let res: redis::RedisResult<bool> = conn.xgroup_createconsumer(PROCESSOR_STREAM, group_name, &consumer_id).await;
		if let Err(e) = res {
			error!("Failed to create processor consumer: {}", e);
			continue;
		}

		started_consumers += 1;

		let group_name = group_name.to_string();

		tokio::spawn(async move {
			consumer_task(group_name, consumer_id, batch_size).await;
		});
	}

	info!("Started {} processor consumers in group {}", started_consumers, group_name);
	Ok(())
}

/// 认领所有 pending 消息并存储到数组中
async fn claim_all_pending_messages<C: redis::AsyncCommands>(conn: &mut C, group_name: &str) -> anyhow::Result<Vec<(String, ProcessorMessage)>> {
	let mut pending_messages = Vec::new();

	// 使用 XPENDING 查看所有 pending 消息
	let pending_result: redis::RedisResult<redis::streams::StreamPendingReply> = conn.xpending_count(PROCESSOR_STREAM, group_name, "-", "+", 10000).await;

	match pending_result {
		Ok(redis::streams::StreamPendingReply::Data(data)) => {
			if data.count > 0 {
				// 使用 XAUTOCLAIM 自动认领所有 pending 消息
				let temp_consumer = format!("temp-claim-{}", Uuid::new_v4());
				let options = redis::streams::StreamAutoClaimOptions::default().count(data.count.min(10000));
				let autoclaim_result: redis::RedisResult<redis::streams::StreamAutoClaimReply> = conn
					.xautoclaim_options(
						PROCESSOR_STREAM,
						group_name,
						&temp_consumer,
						0u64,  // min-idle-time: 0 毫秒，立即认领
						"0-0", // start: 从最早开始
						options,
					)
					.await;

				match autoclaim_result {
					Ok(reply) => {
						let mut message_ids = Vec::new();

						// 解析认领的消息
						for stream_id in reply.claimed {
							let msg_id = stream_id.id.clone();
							message_ids.push(msg_id.clone());

							// 查找消息内容
							if let Some(fields) = stream_id.map.get(PROCESSOR_MSG_KEY) {
								match redis::from_redis_value::<String>(fields) {
									Ok(value) => {
										match serde_json::from_str::<ProcessorMessage>(&value) {
											Ok(msg) => {
												pending_messages.push((msg_id, msg));
											}
											Err(e) => {
												error!("Failed to deserialize pending processor message: {}", e);
											}
										}
									}
									Err(e) => {
										error!("Failed to convert redis value to string for pending message: {}", e);
									}
								}
							}
						}

						// 使用 XACKDEL 确认并删除所有认领的消息
						if !message_ids.is_empty() {
							let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
							let result: redis::RedisResult<Vec<redis::streams::XAckDelStatusCode>> =
								conn.xack_del(PROCESSOR_STREAM, group_name, &ids, redis::streams::StreamDeletionPolicy::DelRef).await;
							match result {
								Ok(results) => {
									let deleted_count = results.iter().filter(|r| matches!(r, redis::streams::XAckDelStatusCode::AcknowledgedAndDeleted)).count();
									info!("XACKDEL {} claimed pending messages ({} deleted)", message_ids.len(), deleted_count);
								}
								Err(e) => {
									error!("Failed to XACKDEL claimed pending messages: {}", e);
								}
							}
						}
					}
					Err(e) => {
						error!("Failed to autoclaim pending messages: {}", e);
					}
				}
			}
		}
		Ok(redis::streams::StreamPendingReply::Empty) => {
			// 没有 pending 消息
		}
		Err(e) => {
			info!("XPENDING returned error (may be expected): {}", e);
		}
	}

	Ok(pending_messages)
}

/// 单个消费者任务
async fn consumer_task(group_name: String, consumer_id: String, batch_size: usize) {
	let mut shutdown_receiver = crate::init::get_shutdown_receiver();

	// 在 loop 外部获取连接 (从 engine_output_mq DB 1 读取 processor_stream)
	let mut conn = match redis_pool::get_engine_output_mq_connection().await {
		Ok(conn) => conn,
		Err(e) => {
			error!("Failed to get redis connection for consumer {}: {}", consumer_id, e);
			return;
		}
	};

	loop {
		tokio::select! {
			result = read_processor_messages(&mut conn, &group_name, &consumer_id, batch_size) => {
				match result {
					Ok(_) => {
						// 成功处理一批消息，继续下一批
					}
					Err(e) => {
						error!("Error reading messages for consumer {}: {}, reconnecting...", consumer_id, e);
						// 重新获取连接
						match redis_pool::get_engine_output_mq_connection().await {
							Ok(new_conn) => {
								conn = new_conn;
								info!("Redis connection reestablished for consumer {}", consumer_id);
							}
							Err(e) => {
								error!("Failed to reconnect for consumer {}: {}", consumer_id, e);
								// 等待一段时间后重试
								tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
							}
						}
					}
				}
			}
			_ = shutdown_receiver.recv() => {
				info!("Processor consumer {} received shutdown signal", consumer_id);
				break;
			}
		}
	}
}

/// 读取并处理消息
async fn read_processor_messages<C: redis::AsyncCommands>(conn: &mut C, group_name: &str, consumer_id: &str, batch_size: usize) -> anyhow::Result<()> {
	// 使用 XREADGROUP 读取消息
	let options = redis::streams::StreamReadOptions::default().group(group_name, consumer_id).count(batch_size).block(1000); // 阻塞1秒
	let keys = vec![PROCESSOR_STREAM];
	let ids = vec![">"];
	let result: redis::RedisResult<Option<redis::streams::StreamReadReply>> = conn.xread_options(&keys, &ids, &options).await;

	match result {
		Ok(Some(reply)) => {
			let mut message_ids = Vec::new();

			// 解析消息
			for stream_key in reply.keys {
				if stream_key.key == PROCESSOR_STREAM {
					for stream_id in stream_key.ids {
						let msg_id = stream_id.id.clone();
						message_ids.push(msg_id.clone());

						// 查找消息内容
						if let Some(fields) = stream_id.map.get(PROCESSOR_MSG_KEY) {
							match redis::from_redis_value::<String>(fields) {
								Ok(value) => {
									match serde_json::from_str::<ProcessorMessage>(&value) {
										Ok(msg) => {
											info!("Consumer {} received processor message", consumer_id);
											if let Err(e) = process_message_inner(msg).await {
												error!("Failed to process message {}: {}", msg_id, e);
											}
										}
										Err(e) => {
											error!("Failed to deserialize processor message: {}", e);
										}
									}
								}
								Err(e) => {
									error!("Failed to convert redis value to string: {}", e);
								}
							}
						}
					}
				}
			}

			// 使用 XACKDEL 确认并删除消息
			if !message_ids.is_empty() {
				let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
				let result: redis::RedisResult<Vec<redis::streams::XAckDelStatusCode>> = conn.xack_del(PROCESSOR_STREAM, group_name, &ids, redis::streams::StreamDeletionPolicy::DelRef).await;
				if let Err(e) = result {
					error!("Failed to XACKDEL processor messages: {}", e);
				}
			}
		}
		Ok(None) => {
			// 没有消息，继续等待
		}
		Err(e) => {
			return Err(anyhow::anyhow!("Redis XREADGROUP error: {}", e));
		}
	}

	Ok(())
}

async fn process_message_inner(message: ProcessorMessage) -> anyhow::Result<()> {
	match message {
		ProcessorMessage::OrderRejected(order_rejected) => handle_order_rejected(order_rejected).await?,
		ProcessorMessage::OrderCancelled(order_cancelled) => handle_order_cancelled(order_cancelled).await?,
		ProcessorMessage::OrderSubmitted(order_submitted) => handle_order_submitted(order_submitted).await?,
		ProcessorMessage::OrderTraded(order_traded) => handle_order_traded(order_traded).await?,
	}

	Ok(())
}

async fn handle_order_rejected(msg: OrderRejected) -> anyhow::Result<()> {
	info!("Handling OrderRejected: order_id={},reason={}", msg.order_id, msg.reason);

	// 调用 asset RPC
	let request = proto::OrderRejectedRequest { user_id: msg.user_id, order_id: msg.order_id.clone() };

	let response = rpc_client::order_rejected(request).await?;
	if !response.success {
		error!("Asset OrderRejected RPC failed: {}", response.reason);
	}

	Ok(())
}

async fn handle_order_cancelled(msg: OrderCancelled) -> anyhow::Result<()> {
	info!("Handling OrderCancelled: order_id={}", msg.order_id);

	// 调用 asset RPC
	let cancelled_quantity = Decimal::from_str(&msg.cancelled_quantity).map_err(|e| anyhow::anyhow!("Invalid cancelled_quantity: {}", e))?;
	let request = proto::CancelOrderRequest { user_id: msg.user_id, order_id: msg.order_id.clone(), cancelled_quantity: cancelled_quantity.to_string(), volume: msg.cancelled_volume.clone() };

	let response = rpc_client::cancel_order(request).await?;
	if !response.success {
		error!("Asset CancelOrder RPC failed: {}", response.reason);
		return Ok(());
	}

	// 发送订单取消事件到websocket，使用 RPC 返回的 update_id
	send_order_cancelled_event(&msg, response.update_id).await;

	Ok(())
}

async fn handle_order_submitted(msg: OrderSubmitted) -> anyhow::Result<()> {
	info!("Handling OrderSubmitted: order_id={}", msg.order_id);

	// 发送订单创建事件到websocket
	send_order_created_event(&msg).await;

	Ok(())
}

async fn handle_order_traded(msg: OrderTraded) -> anyhow::Result<()> {
	info!("Handling OrderTraded: {} trades", msg.trades.len());

	// 从 symbol 获取 event_id 和 market_id
	let event_id = msg.taker_symbol.event_id;
	let market_id = msg.taker_symbol.market_id;

	// 分批处理并收集所有订单的 update_id
	let batches: Vec<&[Trade]> = msg.trades.chunks(consts::TRADE_BATCH_SIZE).collect();
	let mut all_order_update_ids: std::collections::HashMap<String, i64> = std::collections::HashMap::new();

	for batch in batches.into_iter() {
		let order_update_ids: std::collections::HashMap<String, i64> = process_trade_batch(batch, &msg, event_id, market_id).await?;
		all_order_update_ids.extend(order_update_ids);
	}

	// 推送所有 maker 订单更新事件到 websocket
	send_maker_order_updated_events(&msg.trades, event_id, market_id, &all_order_update_ids).await;

	Ok(())
}

async fn process_trade_batch(batch: &[common::processor_types::Trade], msg: &OrderTraded, event_id: i64, market_id: i16) -> anyhow::Result<std::collections::HashMap<String, i64>> {
	let trade_id = uuid::Uuid::new_v4().to_string();

	// 1. 构建 proto::TradeRequest
	// 1.1 聚合 taker 信息
	let mut taker_usdc_amount_total = Decimal::ZERO;
	let mut taker_token_amount_total = Decimal::ZERO;
	let timestamp = batch.first().ok_or_else(|| anyhow::anyhow!("Empty batch"))?.timestamp;

	for trade in batch {
		taker_usdc_amount_total += Decimal::from_str(&trade.taker_usdc_amount).map_err(|e| anyhow::anyhow!("Invalid taker_usdc_amount: {}", e))?;
		taker_token_amount_total += Decimal::from_str(&trade.quantity).map_err(|e| anyhow::anyhow!("Invalid quantity: {}", e))?;
	}

	let taker_trade_info = proto::TakerTradeInfo {
		taker_id: msg.taker_id,
		taker_order_id: msg.taker_order_id.clone(),
		taker_order_side: msg.taker_order_side.to_string(),
		taker_token_id: msg.taker_token_id.clone(),
		taker_usdc_amount: taker_usdc_amount_total.normalize().to_string(),
		taker_token_amount: taker_token_amount_total.normalize().to_string(),
	};

	// 1.2 构建 maker 信息列表
	let maker_trade_infos: Vec<proto::MakerTradeInfo> = batch
		.iter()
		.map(|t| {
			proto::MakerTradeInfo {
				maker_id: t.maker_id,
				maker_order_id: t.maker_order_id.clone(),
				maker_order_side: t.maker_order_side.to_string(),
				maker_token_id: t.maker_token_id.clone(),
				maker_usdc_amount: t.maker_usdc_amount.clone(),
				maker_token_amount: t.quantity.clone(),
				maker_price: t.maker_price.clone(),
			}
		})
		.collect();

	let trade_request = proto::TradeRequest { taker_trade_info: Some(taker_trade_info), maker_trade_infos, trade_id: trade_id.clone(), timestamp, event_id, market_id: market_id as i32 };

	// 2. 调用 asset Trade RPC，获取签名和 order_update_ids
	let trade_response = rpc_client::trade(trade_request).await?;
	if !trade_response.success {
		error!("Asset Trade RPC failed: {}", trade_response.reason);
		return Err(anyhow::anyhow!("Asset Trade RPC failed: {}", trade_response.reason));
	}

	// 提取 order_update_ids 为 HashMap<String, i64>
	let order_update_ids: std::collections::HashMap<String, i64> = trade_response.order_update_ids.into_iter().map(|o| (o.order_id, o.update_id)).collect();

	// 3. 计算 single_freeze：根据 taker_order_side 决定这笔交易应该解冻多少
	let single_freeze = match msg.taker_order_side {
		common::engine_types::OrderSide::Buy => taker_usdc_amount_total,
		common::engine_types::OrderSide::Sell => taker_token_amount_total,
	};

	// taker_unfreeze_amount 等于 single_freeze
	let taker_unfreeze_amount = single_freeze;

	// 7. 构建 TakerTradeInfo（累加）
	let taker_trade_info = TakerTradeInfo {
		taker_side: msg.taker_order_side.to_string(),
		taker_user_id: msg.taker_id,
		taker_usdc_amount: taker_usdc_amount_total.normalize().to_string(),
		taker_token_amount: taker_token_amount_total.normalize().to_string(),
		taker_token_id: msg.taker_token_id.clone(),
		taker_order_id: msg.taker_order_id.clone(),
		taker_unfreeze_amount: taker_unfreeze_amount.normalize().to_string(),
		real_taker_usdc_amount: taker_usdc_amount_total.normalize().to_string(),
		real_taker_token_amount: taker_token_amount_total.normalize().to_string(),
		taker_privy_user_id: msg.taker_privy_user_id.clone(),
		taker_outcome_name: msg.taker_outcome_name.clone(),
	};

	// 8. 构建 MakerTradeInfo（遍历）
	let maker_trade_infos: Vec<MakerTradeInfo> = batch
		.iter()
		.map(|t| {
			MakerTradeInfo {
				maker_side: t.maker_order_side.to_string(),
				maker_user_id: t.maker_id,
				maker_usdc_amount: t.maker_usdc_amount.clone(),
				maker_token_amount: t.quantity.clone(),
				maker_token_id: t.maker_token_id.clone(),
				maker_order_id: t.maker_order_id.clone(),
				maker_price: t.maker_price.clone(),
				real_maker_usdc_amount: t.maker_usdc_amount.clone(),
				real_maker_token_amount: t.quantity.clone(),
				maker_privy_user_id: t.maker_privy_user_id.clone(),
				maker_outcome_name: t.maker_outcome_name.clone(),
			}
		})
		.collect();

	// 9. 构建 MatchOrderInfo
	let mut taker_fill_amount = Decimal::ZERO;
	let mut taker_receive_amount = Decimal::ZERO;
	let mut maker_fill_amount = Vec::new();

	//fill_amount永远是send出去的那个token
	//只有usdc的变化不能累加 因为可能是不同结果相同方向的成交 这个时候的taker的fill/receive amount是要计算的
	//maker签名和maker_fill_amount严格按照顺序
	match msg.taker_order_side {
		common::engine_types::OrderSide::Buy => {
			// taker 是买方
			for trade in batch {
				// maker_fill_amount 列表
				let maker_fill = if trade.maker_order_side == common::engine_types::OrderSide::Buy {
					// maker 是买 → maker_usdc_amount
					trade.maker_usdc_amount.clone()
				} else {
					// maker 是卖 → maker_token_amount（即 quantity）
					trade.quantity.clone()
				};
				maker_fill_amount.push(maker_fill);

				// taker_receive_amount 累加 maker_token_amount
				taker_receive_amount += Decimal::from_str(&trade.quantity).map_err(|e| anyhow::anyhow!("Invalid quantity: {}", e))?;

				// taker_fill_amount 累加
				if trade.maker_order_side == common::engine_types::OrderSide::Sell {
					// maker 是卖 → 累加 maker_usdc_amount
					taker_fill_amount = taker_fill_amount
						.checked_add(Decimal::from_str(&trade.maker_usdc_amount).map_err(|e| anyhow::anyhow!("Invalid maker_usdc_amount: {}", e))?)
						.ok_or_else(|| anyhow::anyhow!("Invalid taker_fill_amount"))?
				} else {
					// maker 是买 → 累加 (maker_token_amount - maker_usdc_amount)
					taker_fill_amount = taker_fill_amount
						.checked_add(
							Decimal::from_str(&trade.quantity)
								.map_err(|e| anyhow::anyhow!("Invalid quantity: {}", e))?
								.checked_sub(Decimal::from_str(&trade.maker_usdc_amount).map_err(|e| anyhow::anyhow!("Invalid maker_usdc_amount: {}", e))?)
								.ok_or_else(|| anyhow::anyhow!("Invalid taker_fill_amount"))?,
						)
						.ok_or_else(|| anyhow::anyhow!("Invalid taker_fill_amount"))?;
				}
			}
		}
		common::engine_types::OrderSide::Sell => {
			// taker 是卖方
			for trade in batch {
				// maker_fill_amount 列表
				let maker_fill = if trade.maker_order_side == common::engine_types::OrderSide::Buy {
					// maker 是买 → maker_usdc_amount
					trade.maker_usdc_amount.clone()
				} else {
					// maker 是卖 → maker_token_amount（即 quantity）
					trade.quantity.clone()
				};
				maker_fill_amount.push(maker_fill);

				// taker_receive_amount 累加
				if trade.maker_order_side == common::engine_types::OrderSide::Buy {
					// maker 是买 → 累加 maker_usdc_amount
					taker_receive_amount = taker_receive_amount
						.checked_add(Decimal::from_str(&trade.maker_usdc_amount).map_err(|e| anyhow::anyhow!("Invalid maker_usdc_amount: {}", e))?)
						.ok_or_else(|| anyhow::anyhow!("Invalid taker_receive_amount"))?;
				} else {
					// maker 是卖 → 累加 (maker_token_amount - maker_usdc_amount)
					taker_receive_amount = taker_receive_amount
						.checked_add(Decimal::from_str(&trade.quantity).map_err(|e| anyhow::anyhow!("Invalid quantity: {}", e))?)
						.ok_or_else(|| anyhow::anyhow!("Invalid taker_receive_amount"))?
						.checked_sub(Decimal::from_str(&trade.maker_usdc_amount).map_err(|e| anyhow::anyhow!("Invalid maker_usdc_amount: {}", e))?)
						.ok_or_else(|| anyhow::anyhow!("Invalid taker_receive_amount"))?;
				}

				// taker_fill_amount 累加 maker_token_amount
				taker_fill_amount = taker_fill_amount
					.checked_add(Decimal::from_str(&trade.quantity).map_err(|e| anyhow::anyhow!("Invalid quantity: {}", e))?)
					.ok_or_else(|| anyhow::anyhow!("Invalid taker_fill_amount"))?;
			}
		}
	}

	// 10. 使用 RPC 返回的 SignatureOrderMsg
	let first_sig = trade_response.signature_order_msgs.first().ok_or_else(|| anyhow::anyhow!("Invalid signature_order_msgs"))?;
	let taker_order = common::model::SignatureOrderMsg {
		expiration: first_sig.expiration.clone(),
		fee_rate_bps: first_sig.fee_rate_bps.clone(),
		maker: first_sig.maker.clone(),
		maker_amount: first_sig.maker_amount.clone(),
		nonce: first_sig.nonce.clone(),
		salt: first_sig.salt,
		side: first_sig.side.clone(),
		signature: first_sig.signature.clone(),
		signature_type: first_sig.signature_type as i16,
		signer: first_sig.signer.clone(),
		taker: first_sig.taker.clone(),
		taker_amount: first_sig.taker_amount.clone(),
		token_id: first_sig.token_id.clone(),
	};

	let maker_order: Vec<common::model::SignatureOrderMsg> = trade_response
		.signature_order_msgs
		.iter()
		.skip(1) // 跳过第一个（taker）
		.map(|sig| {
			common::model::SignatureOrderMsg {
				expiration: sig.expiration.clone(),
				fee_rate_bps: sig.fee_rate_bps.clone(),
				maker: sig.maker.clone(),
				maker_amount: sig.maker_amount.clone(),
				nonce: sig.nonce.clone(),
				salt: sig.salt,
				side: sig.side.clone(),
				signature: sig.signature.clone(),
				signature_type: sig.signature_type as i16,
				signer: sig.signer.clone(),
				taker: sig.taker.clone(),
				taker_amount: sig.taker_amount.clone(),
				token_id: sig.token_id.clone(),
			}
		})
		.collect();

	// 乘以 10^18 转换为链上单位
	let multiplier = Decimal::from(10_i64.pow(18));
	let taker_fill_amount_scaled = taker_fill_amount.checked_mul(multiplier).ok_or_else(|| anyhow::anyhow!("Invalid taker_fill_amount multiplication"))?;
	let taker_receive_amount_scaled = taker_receive_amount.checked_mul(multiplier).ok_or_else(|| anyhow::anyhow!("Invalid taker_receive_amount multiplication"))?;

	let maker_fill_amount_scaled: Vec<String> = maker_fill_amount
		.iter()
		.map(|amount| {
			let decimal_amount = Decimal::from_str(amount).map_err(|e| anyhow::anyhow!("Invalid maker_fill_amount: {}", e))?;
			let scaled = decimal_amount.checked_mul(multiplier).ok_or_else(|| anyhow::anyhow!("Invalid maker_fill_amount multiplication"))?;
			Ok(scaled.normalize().to_string())
		})
		.collect::<anyhow::Result<Vec<String>>>()?;

	let match_info = MatchOrderInfo {
		taker_order,
		maker_order,
		taker_fill_amount: taker_fill_amount_scaled.normalize().to_string(),
		taker_receive_amount: taker_receive_amount_scaled.normalize().to_string(),
		maker_fill_amount: maker_fill_amount_scaled,
	};

	// 11. 构建 TradeOnchainSendRequest
	let trade_send_request = TradeOnchainSendRequest { match_info, trade_id, event_id, market_id: market_id as i32, taker_trade_info, maker_trade_infos };

	// 12. 发送到 trade_send stream
	let msg_json = serde_json::to_string(&trade_send_request)?;
	let mut mq_conn = redis_pool::get_common_mq_connection().await?;
	let _: Result<String, _> = mq_conn.xadd(TRADE_SEND_STREAM, "*", &[(TRADE_SEND_MSG_KEY, msg_json.to_string())]).await;

	info!("Trade batch processed: trade_id={}", trade_send_request.trade_id);

	Ok(order_update_ids)
}

/// 发送用户事件到 USER_EVENT_STREAM
async fn send_user_event(event: UserEvent) -> anyhow::Result<()> {
	let event_json = serde_json::to_string(&event)?;
	let mut conn = redis_pool::get_websocket_mq_connection().await?;
	let _: Result<String, _> =
		conn.xadd_maxlen(USER_EVENT_STREAM, redis::streams::StreamMaxlen::Approx(common::consts::USER_EVENT_STREAM_MAXLEN), "*", &[(USER_EVENT_MSG_KEY, event_json.as_str())]).await;
	Ok(())
}

/// 发送订单创建事件
async fn send_order_created_event(msg: &OrderSubmitted) {
	// 计算 volume = price * quantity
	let volume = match (Decimal::from_str(&msg.price), Decimal::from_str(&msg.quantity)) {
		(Ok(price), Ok(quantity)) => price.checked_mul(quantity).map(|v| v.normalize().to_string()).unwrap_or_else(|| "0".to_string()),
		_ => "0".to_string(),
	};

	let user_open_order = UserOpenOrders {
		privy_id: msg.privy_id.clone(),
		event_id: msg.symbol.event_id,
		market_id: msg.symbol.market_id,
		order_id: msg.order_id.clone(),
		side: msg.side.to_string(),
		outcome_name: msg.outcome_name.clone(),
		price: msg.price.clone(),
		quantity: msg.quantity.clone(),
		volume,
		filled_quantity: msg.filled_quantity.clone(),
		update_id: 1, // 新创建的订单 update_id 为 1
		created_at: chrono::Utc::now().timestamp_millis(),
	};
	let event = UserEvent::OpenOrderChange(OpenOrderChangeEvent::OpenOrderCreated(user_open_order));
	if let Err(e) = send_user_event(event).await {
		error!("Failed to send order created event: {}", e);
	}
}

/// 发送订单取消事件
async fn send_order_cancelled_event(msg: &OrderCancelled, update_id: i64) {
	let event = UserEvent::OpenOrderChange(OpenOrderChangeEvent::OpenOrderCancelled {
		event_id: msg.symbol.event_id,
		market_id: msg.symbol.market_id,
		order_id: msg.order_id.clone(),
		privy_id: msg.privy_id.clone(),
		update_id,
	});
	if let Err(e) = send_user_event(event).await {
		error!("Failed to send order cancelled event: {}", e);
	}
}

/// 批量发送 maker 订单更新事件（使用 pipeline）
/// 如果 maker_quantity <= maker_filled_quantity，则发送 OpenOrderFilled 事件
/// 否则发送 OpenOrderUpdated 事件
async fn send_maker_order_updated_events(trades: &[Trade], event_id: i64, market_id: i16, order_update_ids: &std::collections::HashMap<String, i64>) {
	if trades.is_empty() {
		return;
	}

	// 构建所有 maker 订单更新事件
	let events: Vec<String> = trades
		.iter()
		.filter_map(|trade| {
			// 获取 update_id from the provided map
			let update_id = *order_update_ids.get(&trade.maker_order_id)?;

			// 解析 maker_quantity 和 maker_filled_quantity 判断是否完全成交
			let maker_qty = Decimal::from_str(&trade.maker_quantity).ok()?;
			let maker_filled_qty = Decimal::from_str(&trade.maker_filled_quantity).ok()?;

			let event = if maker_qty <= maker_filled_qty {
				// 完全成交，发送 OpenOrderFilled
				UserEvent::OpenOrderChange(OpenOrderChangeEvent::OpenOrderFilled {
					event_id,
					market_id,
					order_id: trade.maker_order_id.clone(),
					privy_id: trade.maker_privy_user_id.clone(),
					update_id,
				})
			} else {
				// 部分成交，发送 OpenOrderUpdated
				// 计算 volume = maker_price * maker_filled_quantity
				let volume = Decimal::from_str(&trade.maker_price).ok().and_then(|price| price.checked_mul(maker_filled_qty)).map(|v| v.normalize().to_string()).unwrap_or_else(|| "0".to_string());

				let user_open_order = UserOpenOrders {
					privy_id: trade.maker_privy_user_id.clone(),
					event_id,
					market_id,
					order_id: trade.maker_order_id.clone(),
					side: trade.maker_order_side.to_string(),
					outcome_name: trade.maker_outcome_name.clone(),
					price: trade.maker_price.clone(),
					quantity: trade.maker_quantity.clone(),
					volume,
					filled_quantity: trade.maker_filled_quantity.clone(),
					update_id,
					created_at: trade.timestamp,
				};

				UserEvent::OpenOrderChange(OpenOrderChangeEvent::OpenOrderUpdated(user_open_order))
			};

			serde_json::to_string(&event).ok()
		})
		.collect();

	if events.is_empty() {
		return;
	}

	// 分批 pipeline 推送
	for chunk in events.chunks(consts::WEBSOCKET_EVENT_BATCH_SIZE) {
		match redis_pool::get_websocket_mq_connection().await {
			Ok(mut conn) => {
				let mut pipe = redis::pipe();
				for event_json in chunk {
					pipe.xadd_maxlen(USER_EVENT_STREAM, redis::streams::StreamMaxlen::Approx(common::consts::USER_EVENT_STREAM_MAXLEN), "*", &[(USER_EVENT_MSG_KEY, event_json.as_str())]);
				}
				if let Err(e) = pipe.query_async::<Vec<String>>(&mut conn).await {
					error!("Failed to pipeline send maker order updated events: {}", e);
				}
			}
			Err(e) => {
				error!("Failed to get websocket mq connection: {}", e);
			}
		}
	}
}
