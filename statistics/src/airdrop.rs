use {
	crate::{
		consts::{
			AIRDROP_AMOUNT_PER_USER, AIRDROP_BATCH_SIZE, AIRDROP_BLOCK_TIMEOUT_MS, AIRDROP_NATIVE_AMOUNT_PER_USER, AIRDROP_NATIVE_THRESHOLD, AIRDROP_TX_TIMEOUT_SECS, AIRDROP_USDC_THRESHOLD,
			AIRDROP_WAIT_MORE_SECS, DISPERSE_CONTRACT_ADDRESS, USDC_CONTRACT_ADDRESS,
		},
		init::get_shutdown_receiver,
	},
	alloy::{
		eips::BlockNumberOrTag,
		primitives::{Address, U256},
		providers::{Provider, ProviderBuilder},
		rpc::types::TransactionRequest,
		signers::local::PrivateKeySigner,
		sol,
	},
	common::{
		consts::{AIRDROP_MSG_KEY, AIRDROP_STREAM},
		onchain_msg_types::NewUser,
		redis_pool,
	},
	redis::{AsyncCommands, streams::StreamReadOptions},
	tracing::{error, info, warn},
};

/// 空投任务错误类型
#[derive(Debug, thiserror::Error)]
enum AirdropError {
	/// Nonce 冲突（mempool 中存在相同 nonce 的 pending 交易）
	#[error("Nonce conflict: {0}")]
	NonceConflict(String),
}

/// 检查错误消息是否为 nonce 冲突
fn is_nonce_conflict(err_msg: &str) -> bool {
	err_msg.contains("replacement transaction underpriced") || err_msg.contains("nonce too low")
}

// ERC20 合约接口（balanceOf / approve / allowance）
sol! {
	#[sol(rpc)]
	contract IERC20 {
		function balanceOf(address account) external view returns (uint256);
		function approve(address spender, uint256 value) external returns (bool);
		function allowance(address owner, address spender) external view returns (uint256);
	}
}

// Disperse 合约接口
sol! {
	#[sol(rpc)]
	contract IDisperse {
		function disperseEtherAndToken(address token, address[] recipients, uint256[] valuesETH, uint256[] valuesToken) external payable;
	}
}

/// 创建 alloy provider（带 wallet signer）
fn create_provider(private_key: &str, rpc_url: &url::Url) -> impl Provider + Clone {
	let signer: PrivateKeySigner = private_key.parse().expect("Invalid airdrop private key");
	ProviderBuilder::new().wallet(signer).connect_http(rpc_url.clone())
}

/// 启动空投消费任务
/// 从 Redis stream 中读取新用户注册消息，批量通过 Disperse 合约空投 USDC
pub async fn start_airdrop_task() -> anyhow::Result<()> {
	let env = common::common_env::get_common_env();

	let private_key = match env.airdrop_private_key.as_ref() {
		Some(key) if !key.is_empty() => key,
		_ => {
			info!("airdrop_private_key not configured, airdrop task disabled");
			return Ok(());
		}
	};
	let rpc_url = match env.evm_rpc_url.as_ref() {
		Some(url) if !url.is_empty() => url,
		_ => {
			info!("evm_rpc_url not configured, airdrop task disabled");
			return Ok(());
		}
	};

	let rpc_url: url::Url = rpc_url.parse().map_err(|e| anyhow::anyhow!("Invalid evm_rpc_url: {}", e))?;
	let mut provider = create_provider(private_key, &rpc_url);

	let signer: PrivateKeySigner = private_key.parse().map_err(|e| anyhow::anyhow!("Invalid airdrop private key: {}", e))?;
	let sender_address = signer.address();
	let usdc_address: Address = USDC_CONTRACT_ADDRESS.parse().map_err(|e| anyhow::anyhow!("Invalid USDC address: {}", e))?;
	let disperse_address: Address = DISPERSE_CONTRACT_ADDRESS.parse().map_err(|e| anyhow::anyhow!("Invalid Disperse address: {}", e))?;

	info!(
		"Airdrop task started - sender: {}, usdc: {}, disperse: {}, amount_per_user: {}, threshold: {}",
		sender_address, usdc_address, disperse_address, AIRDROP_AMOUNT_PER_USER, AIRDROP_USDC_THRESHOLD
	);

	// 启动时检查并设置 approve（只需一次，approve max）
	ensure_approval(&provider, sender_address, usdc_address, disperse_address).await;

	let mut shutdown_receiver = get_shutdown_receiver();
	let mut conn = redis_pool::get_common_mq_connection().await?;
	let mut last_id = "0-0".to_string();

	loop {
		tokio::select! {
			result = consume_and_airdrop(
				&mut conn, &mut last_id, &provider,
				sender_address, usdc_address, disperse_address,
			) => {
				match result {
					Ok(_) => {}
					Err(e) => {
						// nonce 冲突：先取消 mempool 中卡住的交易，再重建 provider
						if e.downcast_ref::<AirdropError>().is_some_and(|ae| matches!(ae, AirdropError::NonceConflict(_))) {
							warn!("{}, cancelling stuck txs...", e);
							cancel_pending_transactions(&provider, sender_address).await;
							provider = create_provider(private_key, &rpc_url);
							tokio::time::sleep(std::time::Duration::from_secs(3)).await;
							continue;
						}

						error!("Airdrop batch error: {}, reconnecting redis...", e);
						match redis_pool::get_common_mq_connection().await {
							Ok(new_conn) => {
								conn = new_conn;
								info!("Redis connection reestablished for airdrop");
							}
							Err(e) => {
								error!("Redis reconnect failed for airdrop: {}", e);
								tokio::time::sleep(std::time::Duration::from_secs(5)).await;
							}
						}
					}
				}
			}
			_ = shutdown_receiver.recv() => {
				info!("Airdrop task received shutdown signal");
				break;
			}
		}
	}

	Ok(())
}

/// 启动时检查 allowance，不足则 approve max
async fn ensure_approval(provider: &(impl Provider + Clone), sender: Address, usdc_addr: Address, disperse_addr: Address) {
	let usdc = IERC20::new(usdc_addr, provider);
	match usdc.allowance(sender, disperse_addr).call().await {
		Ok(current_allowance) => {
			if current_allowance < U256::from(u128::MAX) {
				info!("Current allowance insufficient, approving Disperse contract...");
				match usdc.approve(disperse_addr, U256::MAX).send().await {
					Ok(pending) => {
						match pending.watch().await {
							Ok(tx_hash) => info!("USDC approval confirmed, tx: {}", tx_hash),
							Err(e) => error!("Failed to watch approval tx: {}", e),
						}
					}
					Err(e) => error!("Failed to send approval tx: {}", e),
				}
			} else {
				info!("Disperse contract already approved");
			}
		}
		Err(e) => {
			error!("Failed to check allowance on startup: {}", e);
		}
	}
}

/// 取消 mempool 中卡住的 pending 交易（发 0 值交易给自己，用更高 gas 覆盖）
async fn cancel_pending_transactions(provider: &(impl Provider + Clone), sender: Address) {
	let confirmed_nonce = match provider.get_transaction_count(sender).await {
		Ok(n) => n,
		Err(e) => {
			error!("Failed to get confirmed nonce: {}", e);
			return;
		}
	};

	let pending_nonce = match provider.get_transaction_count(sender).block_id(BlockNumberOrTag::Pending.into()).await {
		Ok(n) => n,
		Err(e) => {
			error!("Failed to get pending nonce: {}", e);
			return;
		}
	};

	if pending_nonce <= confirmed_nonce {
		info!("No stuck pending transactions (confirmed={}, pending={})", confirmed_nonce, pending_nonce);
		return;
	}

	let stuck_count = pending_nonce - confirmed_nonce;
	warn!("Found {} stuck pending txs (confirmed_nonce={}, pending_nonce={}), cancelling...", stuck_count, confirmed_nonce, pending_nonce);

	// 用当前 gas price 的 2 倍来覆盖卡住的交易
	let gas_price = provider.get_gas_price().await.unwrap_or(10_000_000_000) * 2;

	for nonce in confirmed_nonce..pending_nonce {
		let tx = TransactionRequest::default().from(sender).to(sender).value(U256::ZERO).nonce(nonce).gas_limit(21_000).gas_price(gas_price);

		match provider.send_transaction(tx).await {
			Ok(pending) => {
				match pending.watch().await {
					Ok(tx_hash) => info!("Cancel tx confirmed: nonce={}, tx={}", nonce, tx_hash),
					Err(e) => error!("Cancel tx watch failed: nonce={}, err={}", nonce, e),
				}
			}
			Err(e) => {
				let msg = e.to_string();
				if msg.contains("nonce too low") {
					info!("Nonce {} already confirmed, skipping", nonce);
				} else {
					error!("Failed to send cancel tx: nonce={}, err={}", nonce, e);
				}
			}
		}
	}
}

/// 从 Redis stream 读取消息并执行批量空投
async fn consume_and_airdrop(
	conn: &mut deadpool_redis::Connection,
	last_id: &mut String,
	provider: &(impl Provider + Clone),
	sender_address: Address,
	usdc_address: Address,
	disperse_address: Address,
) -> anyhow::Result<()> {
	// 1. 阻塞读取，最多 BATCH_SIZE 条消息
	let options = StreamReadOptions::default().count(AIRDROP_BATCH_SIZE).block(AIRDROP_BLOCK_TIMEOUT_MS);
	let result: Option<redis::streams::StreamReadReply> = conn.xread_options(&[AIRDROP_STREAM], &[last_id.as_str()], &options).await?;

	let mut messages: Vec<NewUser> = Vec::new();
	let mut message_ids: Vec<String> = Vec::new();

	if let Some(reply) = result {
		parse_stream_reply(reply, &mut messages, &mut message_ids);
	}

	if messages.is_empty() {
		return Ok(());
	}

	info!("Processing airdrop batch: {} users", messages.len());

	// 2. 构建 recipients、valuesETH、valuesToken
	let mut recipients: Vec<Address> = Vec::new();
	let mut values_eth: Vec<U256> = Vec::new();
	let mut values_token: Vec<U256> = Vec::new();

	for msg in &messages {
		match msg.privy_evm_address.parse::<Address>() {
			Ok(addr) => {
				recipients.push(addr);
				values_eth.push(U256::from(AIRDROP_NATIVE_AMOUNT_PER_USER));
				values_token.push(U256::from(AIRDROP_AMOUNT_PER_USER));
			}
			Err(e) => {
				warn!("Invalid EVM address for user_id={}, privy_id={}: {}, skipping", msg.user_id, msg.privy_id, e);
			}
		}
	}

	if recipients.is_empty() {
		warn!("No valid addresses in batch, deleting messages");
		delete_messages(conn, &message_ids).await;
		update_last_id(last_id, &message_ids);
		return Ok(());
	}

	// 3. 检查 USDC 余额
	let usdc = IERC20::new(usdc_address, provider);
	let balance = usdc.balanceOf(sender_address).call().await.unwrap_or_default();
	let threshold = U256::from(AIRDROP_USDC_THRESHOLD);
	let total_needed = U256::from(AIRDROP_AMOUNT_PER_USER) * U256::from(recipients.len());

	if balance < threshold {
		error!("USDC balance {} below threshold {}, skipping airdrop batch", balance, threshold);
		return Ok(());
	}

	if balance < total_needed {
		error!("USDC balance {} insufficient for {} users (need {}), skipping", balance, recipients.len(), total_needed);
		return Ok(());
	}

	// 检查原生代币余额
	let native_balance = provider.get_balance(sender_address).await.unwrap_or_default();
	let native_threshold = U256::from(AIRDROP_NATIVE_THRESHOLD);
	if native_balance < native_threshold {
		error!("Native balance {} below threshold {}, skipping airdrop batch", native_balance, native_threshold);
		return Ok(());
	}

	// 4. 调用 disperseEtherAndToken 批量转账原生代币 + USDC
	let total_eth = U256::from(AIRDROP_NATIVE_AMOUNT_PER_USER) * U256::from(recipients.len());
	let gas_price = provider.get_gas_price().await.unwrap_or(10_000_000_000) * 3 / 2;
	info!(
		"Sending disperseEtherAndToken: token={}, recipients={:?}, native_each={}, usdc_each={}, total_native={}, gas_price={}",
		usdc_address, recipients, AIRDROP_NATIVE_AMOUNT_PER_USER, AIRDROP_AMOUNT_PER_USER, total_eth, gas_price
	);
	let disperse = IDisperse::new(disperse_address, provider);
	let timeout_duration = std::time::Duration::from_secs(AIRDROP_TX_TIMEOUT_SECS);

	match tokio::time::timeout(timeout_duration, disperse.disperseEtherAndToken(usdc_address, recipients.clone(), values_eth, values_token).value(total_eth).gas_price(gas_price).send()).await {
		Ok(Ok(pending)) => {
			match tokio::time::timeout(timeout_duration, pending.watch()).await {
				Ok(Ok(tx_hash)) => {
					info!("Airdrop tx confirmed: {}, recipients: {:?}", tx_hash, recipients);
					delete_messages(conn, &message_ids).await;
					update_last_id(last_id, &message_ids);
				}
				Ok(Err(e)) => {
					let msg = e.to_string();
					if is_nonce_conflict(&msg) {
						return Err(AirdropError::NonceConflict(msg).into());
					}
					error!("Airdrop tx watch failed: {}, will retry", e);
				}
				Err(_) => {
					warn!("Airdrop tx watch timed out after {}s, tx may still be pending", AIRDROP_TX_TIMEOUT_SECS);
					return Err(AirdropError::NonceConflict("tx watch timed out".to_string()).into());
				}
			}
		}
		Ok(Err(e)) => {
			let msg = e.to_string();
			if is_nonce_conflict(&msg) {
				return Err(AirdropError::NonceConflict(msg).into());
			}
			error!("Failed to send airdrop tx: {}, will retry", e);
		}
		Err(_) => {
			warn!("Airdrop tx send timed out after {}s", AIRDROP_TX_TIMEOUT_SECS);
			return Err(AirdropError::NonceConflict("tx send timed out".to_string()).into());
		}
	}

	// 每轮处理完后休息几秒
	tokio::time::sleep(std::time::Duration::from_secs(AIRDROP_WAIT_MORE_SECS)).await;

	Ok(())
}

/// 解析 Redis Stream 消息
fn parse_stream_reply(reply: redis::streams::StreamReadReply, messages: &mut Vec<NewUser>, message_ids: &mut Vec<String>) {
	for stream_key in reply.keys {
		for stream_id in stream_key.ids {
			message_ids.push(stream_id.id.clone());
			if let Some(value) = stream_id.map.get(AIRDROP_MSG_KEY)
				&& let Ok(value_str) = redis::from_redis_value::<String>(value)
			{
				match serde_json::from_str::<NewUser>(&value_str) {
					Ok(msg) => messages.push(msg),
					Err(e) => warn!("Failed to parse airdrop message: {}", e),
				}
			}
		}
	}
}

/// 删除已处理的 Redis Stream 消息
async fn delete_messages(conn: &mut impl AsyncCommands, message_ids: &[String]) {
	if !message_ids.is_empty() {
		let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
		let _: redis::RedisResult<usize> = conn.xdel(AIRDROP_STREAM, &ids).await;
	}
}

/// 更新 last_id 为最后一条消息的 ID
fn update_last_id(last_id: &mut String, message_ids: &[String]) {
	if let Some(last) = message_ids.last() {
		*last_id = last.clone();
	}
}
