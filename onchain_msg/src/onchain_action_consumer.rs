use {
	crate::{cache, config, consts, rpc_client},
	common::{
		consts::{ONCHAIN_ACTION_MSG_KEY, ONCHAIN_ACTION_STREAM, USDC_TOKEN_ID},
		onchain_msg_types::{Deposit, Merge, Redeem, Split, Withdraw},
		redis_pool,
	},
	redis::{AsyncCommands, streams::StreamReadOptions},
	rust_decimal::Decimal,
	serde::{Deserialize, Serialize},
	std::{str::FromStr, time::Duration},
	tracing::{error, info},
	uuid::Uuid,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "types", rename_all = "snake_case")]
enum OnchainAction {
	Deposit(Deposit),
	Withdraw(Withdraw),
	Split(Split),
	Merge(Merge),
	Redeem(Redeem),
}

/// Normalize raw blockchain amount (with 18 decimals) to Decimal string
fn normalize_amount(raw_amount: &str) -> anyhow::Result<String> {
	let amount = Decimal::from_str(raw_amount)?;
	let divisor = Decimal::from(10u64.pow(18));
	let normalized = amount.checked_div(divisor).ok_or_else(|| anyhow::anyhow!("Division overflow"))?.normalize();
	Ok(normalized.to_string())
}

pub async fn start_consumers() -> anyhow::Result<()> {
	let config = config::get_config();
	let consumer_count = config.onchain_action_mq.consumer_count;
	let batch_size = config.onchain_action_mq.batch_size;

	let group_name = consts::ONCHAIN_ACTION_CONSUMER_GROUP;
	let consumer_prefix = consts::ONCHAIN_ACTION_CONSUMER_PREFIX;

	// 确保消费者组存在
	let mut conn = redis_pool::get_common_mq_connection().await?;
	let _: Result<(), _> = conn.xgroup_create_mkstream(ONCHAIN_ACTION_STREAM, group_name, "0").await;

	// 在启动消费者之前，先认领并处理所有 pending 消息
	claim_and_process_pending_messages(&mut conn, group_name).await?;

	// 启动多个消费者任务
	let consumer_id_base = Uuid::new_v4().to_string();
	let mut started_count = 0;
	for i in 0..consumer_count {
		let consumer_id = format!("{}-{}-{}", consumer_prefix, consumer_id_base, i);
		let group_name_owned = group_name.to_string();

		// 创建消费者
		let res: Result<bool, _> = conn.xgroup_createconsumer(ONCHAIN_ACTION_STREAM, group_name, &consumer_id).await;
		if let Err(e) = res {
			error!("Failed to create onchain action consumer {}: {}", consumer_id, e);
			continue;
		}

		started_count += 1;

		tokio::spawn(async move {
			if let Err(e) = consumer_task(group_name_owned, consumer_id, batch_size).await {
				error!("OnchainAction consumer error: {}", e);
			}
		});
	}

	info!("Started {} onchain action consumers in group {}", started_count, group_name);
	Ok(())
}

/// 认领并处理所有 pending 消息
async fn claim_and_process_pending_messages(conn: &mut impl AsyncCommands, group_name: &str) -> anyhow::Result<()> {
	// 使用 XPENDING 查看所有 pending 消息
	let pending_result: redis::RedisResult<redis::streams::StreamPendingReply> = conn.xpending_count(ONCHAIN_ACTION_STREAM, group_name, "-", "+", 10000).await;

	match pending_result {
		Ok(redis::streams::StreamPendingReply::Data(data)) => {
			if data.count > 0 {
				info!("Found {} pending messages, claiming and processing...", data.count);

				// 使用 XAUTOCLAIM 自动认领所有 pending 消息
				let temp_consumer = format!("temp-claim-{}", Uuid::new_v4());
				let options = redis::streams::StreamAutoClaimOptions::default().count(data.count.min(10000));
				let autoclaim_result: redis::RedisResult<redis::streams::StreamAutoClaimReply> = conn.xautoclaim_options(ONCHAIN_ACTION_STREAM, group_name, &temp_consumer, 0u64, "0-0", options).await;

				match autoclaim_result {
					Ok(reply) => {
						let mut message_ids = Vec::new();

						// 处理认领的消息
						for stream_id in reply.claimed {
							let msg_id = stream_id.id.clone();
							message_ids.push(msg_id.clone());

							if let Some(value) = stream_id.map.get(ONCHAIN_ACTION_MSG_KEY)
								&& let Ok(json_str) = redis::from_redis_value::<String>(value)
								&& let Err(e) = process_message(&json_str).await
							{
								error!("Failed to process pending message {}: {}", msg_id, e);
							}
						}

						// ACK 并删除所有认领的消息
						if !message_ids.is_empty() {
							let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
							let _: Result<Vec<redis::streams::XAckDelStatusCode>, _> = conn.xack_del(ONCHAIN_ACTION_STREAM, group_name, &ids, redis::streams::StreamDeletionPolicy::DelRef).await;
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
				info!("OnchainAction consumer {} received shutdown signal", consumer_id);
				break;
			}
		}
	}

	Ok(())
}

async fn process_new_messages(conn: &mut impl AsyncCommands, group: &str, consumer: &str, batch_size: usize) -> anyhow::Result<()> {
	let opts = StreamReadOptions::default().group(group, consumer).count(batch_size).block(1000);

	let reply: redis::streams::StreamReadReply = conn.xread_options(&[ONCHAIN_ACTION_STREAM], &[">"], &opts).await?;

	for stream_key in reply.keys {
		for stream_id in stream_key.ids {
			let msg_id = stream_id.id.clone();

			if let Some(value) = stream_id.map.get(ONCHAIN_ACTION_MSG_KEY)
				&& let Ok(json_str) = redis::from_redis_value::<String>(value)
			{
				if let Err(e) = process_message(&json_str).await {
					error!("Failed to process onchain action message {}: {}", msg_id, e);
				}

				// ACK message
				let result: Result<Vec<redis::streams::XAckDelStatusCode>, _> = conn.xack_del(ONCHAIN_ACTION_STREAM, group, &[&msg_id], redis::streams::StreamDeletionPolicy::DelRef).await;
				if let Err(e) = result {
					error!("Failed to XACKDEL onchain action messages: {}", e);
				}
			}
		}
	}

	Ok(())
}

async fn process_message(json_str: &str) -> anyhow::Result<()> {
	let action: OnchainAction = serde_json::from_str(json_str)?;

	match action {
		OnchainAction::Deposit(deposit) => handle_deposit(deposit).await?,
		OnchainAction::Withdraw(withdraw) => handle_withdraw(withdraw).await?,
		OnchainAction::Split(split) => handle_split(split).await?,
		OnchainAction::Merge(merge) => handle_merge(merge).await?,
		OnchainAction::Redeem(redeem) => handle_redeem(redeem).await?,
	}

	Ok(())
}

async fn handle_deposit(deposit: Deposit) -> anyhow::Result<()> {
	info!("Handling Deposit: user_id={}, token_id={}, amount={}", deposit.user_id, deposit.token_id, deposit.amount);

	// For USDC deposits, event_id and market_id are both 0
	// For other tokens, query token_id to get event_id, market_id, and outcome_name
	let (event_id, market_id, outcome_name) = if deposit.token_id == USDC_TOKEN_ID { (0, 0, String::new()) } else { cache::query_token_id(&deposit.token_id).await? };

	// Normalize amount
	let normalized_amount = normalize_amount(&deposit.amount)?;

	let request = proto::DepositRequest {
		user_id: deposit.user_id,
		amount: normalized_amount,
		token_id: deposit.token_id,
		tx_hash: deposit.tx_hash,
		event_id,
		market_id: market_id as i32,
		privy_id: deposit.privy_id,
		outcome_name,
	};

	let response = rpc_client::deposit(request).await?;
	if !response.success {
		error!("Asset Deposit RPC failed: {}", response.reason);
	}

	Ok(())
}

async fn handle_withdraw(withdraw: Withdraw) -> anyhow::Result<()> {
	info!("Handling Withdraw: user_id={}, token_id={}, amount={}", withdraw.user_id, withdraw.token_id, withdraw.amount);

	// For USDC withdrawals, event_id and market_id are both 0
	// For other tokens, query token_id to get event_id, market_id, and outcome_name
	let (event_id, market_id, outcome_name) = if withdraw.token_id == USDC_TOKEN_ID { (0, 0, String::new()) } else { cache::query_token_id(&withdraw.token_id).await? };

	// Normalize amount
	let normalized_amount = normalize_amount(&withdraw.amount)?;

	let request = proto::WithdrawRequest {
		user_id: withdraw.user_id,
		amount: normalized_amount,
		token_id: withdraw.token_id,
		tx_hash: withdraw.tx_hash,
		event_id,
		market_id: market_id as i32,
		privy_id: withdraw.privy_id,
		outcome_name,
	};

	let response = rpc_client::withdraw(request).await?;
	if !response.success {
		error!("Asset Withdraw RPC failed: {}", response.reason);
	}

	Ok(())
}

async fn handle_split(split: Split) -> anyhow::Result<()> {
	info!("Handling Split: user_id={}, condition_id={}, amount={}", split.user_id, split.condition_id, split.amount);

	// Query condition_id to get event_id, market_id, and token_ids
	let (event_id, market_id, token_ids) = cache::query_condition_id(&split.condition_id).await?;

	// Extract token_ids
	if token_ids.len() != 2 {
		return Err(anyhow::anyhow!("Expected 2 token IDs for condition {}, got {}", split.condition_id, token_ids.len()));
	}
	let token_0_id = &token_ids[0];
	let token_1_id = &token_ids[1];

	// Query outcome names
	let outcome_name_0 = cache::query_outcome_name(token_0_id).await?;
	let outcome_name_1 = cache::query_outcome_name(token_1_id).await?;

	// Normalize amount
	let normalized_amount = normalize_amount(&split.amount)?;

	let request = proto::SplitRequest {
		user_id: split.user_id,
		usdc_amount: normalized_amount.clone(),
		token_0_id: token_0_id.clone(),
		token_1_id: token_1_id.clone(),
		token_0_amount: normalized_amount.clone(),
		token_1_amount: normalized_amount,
		tx_hash: split.tx_hash,
		event_id,
		market_id: market_id as i32,
		privy_id: split.privy_id,
		outcome_name_0,
		outcome_name_1,
	};

	let response = rpc_client::split(request).await?;
	if !response.success {
		error!("Asset Split RPC failed: {}", response.reason);
	}

	Ok(())
}

async fn handle_merge(merge: Merge) -> anyhow::Result<()> {
	info!("Handling Merge: user_id={}, condition_id={}, amount={}", merge.user_id, merge.condition_id, merge.amount);

	// Query condition_id to get event_id, market_id, and token_ids
	let (event_id, market_id, token_ids) = cache::query_condition_id(&merge.condition_id).await?;

	// Extract token_ids
	if token_ids.len() != 2 {
		return Err(anyhow::anyhow!("Expected 2 token IDs for condition {}, got {}", merge.condition_id, token_ids.len()));
	}
	let token_0_id = &token_ids[0];
	let token_1_id = &token_ids[1];

	// Query outcome names
	let outcome_name_0 = cache::query_outcome_name(token_0_id).await?;
	let outcome_name_1 = cache::query_outcome_name(token_1_id).await?;

	// Normalize amount
	let normalized_amount = normalize_amount(&merge.amount)?;

	let request = proto::MergeRequest {
		user_id: merge.user_id,
		token_0_id: token_0_id.clone(),
		token_1_id: token_1_id.clone(),
		token_0_amount: normalized_amount.clone(),
		token_1_amount: normalized_amount.clone(),
		usdc_amount: normalized_amount,
		tx_hash: merge.tx_hash,
		event_id,
		market_id: market_id as i32,
		privy_id: merge.privy_id,
		outcome_name_0,
		outcome_name_1,
	};

	let response = rpc_client::merge(request).await?;
	if !response.success {
		error!("Asset Merge RPC failed: {}", response.reason);
	}

	Ok(())
}

async fn handle_redeem(redeem: Redeem) -> anyhow::Result<()> {
	info!("Handling Redeem: user_id={}, condition_id={}, payout={}", redeem.user_id, redeem.condition_id, redeem.payout);

	// Query condition_id to get event_id, market_id, and token_ids
	let (event_id, market_id, token_ids) = cache::query_condition_id(&redeem.condition_id).await?;

	// Extract token_ids
	if token_ids.len() != 2 {
		return Err(anyhow::anyhow!("Expected 2 token IDs for condition {}, got {}", redeem.condition_id, token_ids.len()));
	}
	let token_0_id = &token_ids[0];
	let token_1_id = &token_ids[1];

	// Query outcome names
	let outcome_name_0 = cache::query_outcome_name(token_0_id).await?;
	let outcome_name_1 = cache::query_outcome_name(token_1_id).await?;

	// Normalize payout amount
	let normalized_payout = normalize_amount(&redeem.payout)?;

	let request = proto::RedeemRequest {
		user_id: redeem.user_id,
		token_id_0: token_0_id.clone(),
		token_id_1: token_1_id.clone(),
		usdc_amount: normalized_payout,
		tx_hash: redeem.tx_hash,
		event_id,
		market_id: market_id as i32,
		privy_id: redeem.privy_id,
		outcome_name_0,
		outcome_name_1,
	};

	let response = rpc_client::redeem(request).await?;
	if !response.success {
		error!("Asset Redeem RPC failed: {}", response.reason);
	}

	Ok(())
}
