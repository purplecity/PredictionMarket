use {
	crate::{cache, config, consts, rpc_client},
	common::{
		consts::{TRADE_RESPONSE_MSG_KEY, TRADE_RESPONSE_STREAM},
		onchain_msg_types::TradeOnchainSendResponse,
		redis_pool,
	},
	redis::{AsyncCommands, streams::StreamReadOptions},
	std::time::Duration,
	tracing::{error, info},
	uuid::Uuid,
};

pub async fn start_consumers() -> anyhow::Result<()> {
	let config = config::get_config();
	let consumer_count = config.trade_response_mq.consumer_count;
	let batch_size = config.trade_response_mq.batch_size;

	let group_name = consts::TRADE_RESPONSE_CONSUMER_GROUP;
	let consumer_prefix = consts::TRADE_RESPONSE_CONSUMER_PREFIX;

	// 确保消费者组存在
	let mut conn = redis_pool::get_common_mq_connection().await?;
	let _: Result<(), _> = conn.xgroup_create_mkstream(TRADE_RESPONSE_STREAM, group_name, "0").await;

	// 在启动消费者之前，先认领并处理所有 pending 消息
	claim_and_process_pending_messages(&mut conn, group_name).await?;

	// 启动多个消费者任务
	let consumer_id_base = Uuid::new_v4().to_string();
	let mut started_count = 0;
	for i in 0..consumer_count {
		let consumer_id = format!("{}-{}-{}", consumer_prefix, consumer_id_base, i);
		let group_name_owned = group_name.to_string();

		// 创建消费者
		let res: Result<bool, _> = conn.xgroup_createconsumer(TRADE_RESPONSE_STREAM, group_name, &consumer_id).await;
		if let Err(e) = res {
			error!("Failed to create trade response consumer {}: {}", consumer_id, e);
			continue;
		}

		started_count += 1;

		tokio::spawn(async move {
			if let Err(e) = consumer_task(group_name_owned, consumer_id, batch_size).await {
				error!("TradeResponse consumer error: {}", e);
			}
		});
	}

	info!("Started {} trade response consumers in group {}", started_count, group_name);
	Ok(())
}

/// 认领并处理所有 pending 消息
async fn claim_and_process_pending_messages(conn: &mut impl AsyncCommands, group_name: &str) -> anyhow::Result<()> {
	// 使用 XPENDING 查看所有 pending 消息
	let pending_result: redis::RedisResult<redis::streams::StreamPendingReply> = conn.xpending_count(TRADE_RESPONSE_STREAM, group_name, "-", "+", 10000).await;

	match pending_result {
		Ok(redis::streams::StreamPendingReply::Data(data)) => {
			if data.count > 0 {
				info!("Found {} pending messages, claiming and processing...", data.count);

				// 使用 XAUTOCLAIM 自动认领所有 pending 消息
				let temp_consumer = format!("temp-claim-{}", Uuid::new_v4());
				let options = redis::streams::StreamAutoClaimOptions::default().count(data.count.min(10000));
				let autoclaim_result: redis::RedisResult<redis::streams::StreamAutoClaimReply> = conn.xautoclaim_options(TRADE_RESPONSE_STREAM, group_name, &temp_consumer, 0u64, "0-0", options).await;

				match autoclaim_result {
					Ok(reply) => {
						let mut message_ids = Vec::new();

						// 处理认领的消息
						for stream_id in reply.claimed {
							let msg_id = stream_id.id.clone();
							message_ids.push(msg_id.clone());

							if let Some(value) = stream_id.map.get(TRADE_RESPONSE_MSG_KEY)
								&& let Ok(json_str) = redis::from_redis_value::<String>(value)
								&& let Err(e) = process_message(&json_str).await
							{
								error!("Failed to process pending message {}: {}", msg_id, e);
							}
						}

						// ACK 并删除所有认领的消息
						if !message_ids.is_empty() {
							let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
							let _: Result<Vec<redis::streams::XAckDelStatusCode>, _> = conn.xack_del(TRADE_RESPONSE_STREAM, group_name, &ids, redis::streams::StreamDeletionPolicy::DelRef).await;
							info!("Processed and acknowledged {} pending messages", message_ids.len());
						}
					}
					Err(e) => {
						error!("Failed to autoclaim pending messages: {}", e);
					}
				}
			}
		}
		Ok(redis::streams::StreamPendingReply::Empty) => {}
		Err(e) => {
			info!("XPENDING returned error (may be expected): {}", e);
		}
	}

	Ok(())
}

async fn consumer_task(group_name: String, consumer_id: String, batch_size: usize) -> anyhow::Result<()> {
	let mut shutdown_receiver = crate::init::get_shutdown_receiver();
	let mut conn = redis_pool::get_common_mq_connection().await?;

	loop {
		tokio::select! {
			result = process_new_messages(&mut conn, &group_name, &consumer_id, batch_size) => {
				if let Err(e) = result {
					error!("Consumer {} read error: {}, reconnecting...", consumer_id, e);
					match redis_pool::get_common_mq_connection().await {
						Ok(new_conn) => {
							conn = new_conn;
							info!("Redis connection reestablished for consumer {}", consumer_id);
						}
						Err(e) => {
							error!("Failed to reconnect for consumer {}: {}", consumer_id, e);
							tokio::time::sleep(Duration::from_secs(1)).await;
						}
					}
				}
			}
			_ = shutdown_receiver.recv() => {
				info!("TradeResponse consumer {} received shutdown signal", consumer_id);
				break;
			}
		}
	}

	Ok(())
}

async fn process_new_messages(conn: &mut impl AsyncCommands, group: &str, consumer: &str, batch_size: usize) -> anyhow::Result<()> {
	let opts = StreamReadOptions::default().group(group, consumer).count(batch_size).block(5000);

	let reply: redis::streams::StreamReadReply = conn.xread_options(&[TRADE_RESPONSE_STREAM], &[">"], &opts).await?;

	for stream_key in reply.keys {
		for stream_id in stream_key.ids {
			let msg_id = stream_id.id.clone();

			if let Some(value) = stream_id.map.get(TRADE_RESPONSE_MSG_KEY)
				&& let Ok(json_str) = redis::from_redis_value::<String>(value)
			{
				if let Err(e) = process_message(&json_str).await {
					error!("Failed to process trade response message {}: {}", msg_id, e);
				}

				// ACK message
				let result: Result<Vec<redis::streams::XAckDelStatusCode>, _> = conn.xack_del(TRADE_RESPONSE_STREAM, group, &[&msg_id], redis::streams::StreamDeletionPolicy::DelRef).await;
				if let Err(e) = result {
					error!("Failed to XACKDEL trade response messages: {}", e);
				}
			}
		}
	}

	Ok(())
}

async fn process_message(json_str: &str) -> anyhow::Result<()> {
	let response: TradeOnchainSendResponse = serde_json::from_str(json_str)?;

	info!("Handling TradeOnchainSendResponse: trade_id={}, success={}", response.trade_id, response.success);

	// Look up question from cache using taker_token_id
	let question = cache::query_question(&response.taker_trade_info.taker_token_id).await.unwrap_or_default();

	// Construct TradeOnchainSendResultRequest with proto types
	let taker_info = proto::TakerTradeOnchainInfo {
		taker_side: response.taker_trade_info.taker_side,
		taker_user_id: response.taker_trade_info.taker_user_id,
		taker_usdc_amount: response.taker_trade_info.taker_usdc_amount,
		taker_token_amount: response.taker_trade_info.taker_token_amount,
		taker_token_id: response.taker_trade_info.taker_token_id,
		taker_order_id: response.taker_trade_info.taker_order_id,
		real_taker_usdc_amount: response.taker_trade_info.real_taker_usdc_amount,
		real_taker_token_amount: response.taker_trade_info.real_taker_token_amount,
		taker_unfreeze_amount: response.taker_trade_info.taker_unfreeze_amount,
		taker_privy_user_id: response.taker_trade_info.taker_privy_user_id,
		taker_outcome_name: response.taker_trade_info.taker_outcome_name,
		question,
	};

	let maker_infos: Vec<proto::MakerTradeOnchainInfo> = response
		.maker_trade_infos
		.iter()
		.map(|m| {
			proto::MakerTradeOnchainInfo {
				maker_side: m.maker_side.clone(),
				maker_user_id: m.maker_user_id,
				maker_usdc_amount: m.maker_usdc_amount.clone(),
				maker_token_amount: m.maker_token_amount.clone(),
				maker_token_id: m.maker_token_id.clone(),
				maker_order_id: m.maker_order_id.clone(),
				maker_price: m.maker_price.clone(),
				real_maker_usdc_amount: m.real_maker_usdc_amount.clone(),
				real_maker_token_amount: m.real_maker_token_amount.clone(),
				maker_privy_user_id: m.maker_privy_user_id.clone(),
				maker_outcome_name: m.maker_outcome_name.clone(),
			}
		})
		.collect();

	let request = proto::TradeOnchainSendResultRequest {
		trade_id: response.trade_id,
		event_id: response.event_id,
		market_id: response.market_id as i32,
		taker_trade_info: Some(taker_info),
		maker_trade_infos: maker_infos,
		tx_hash: response.tx_hash,
		success: response.success,
	};

	// Call asset RPC
	let rpc_response = rpc_client::trade_onchain_send_result(request).await?;
	if !rpc_response.success {
		error!("Asset TradeOnchainSendResult RPC failed: {}", rpc_response.reason);
	}

	Ok(())
}
