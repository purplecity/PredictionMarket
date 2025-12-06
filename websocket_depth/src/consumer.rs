use {
	crate::{
		consts::{DEPTH_BUILD_WINDOW_SECS, DEPTH_EVENT_BATCH_SIZE},
		storage::DepthStorage,
	},
	common::{
		consts::{WEBSOCKET_MSG_KEY, WEBSOCKET_STREAM},
		redis_pool,
		websocket_types::{WebSocketDepth, WebSocketPriceChanges},
	},
	redis::{AsyncCommands, from_redis_value, streams::StreamReadOptions},
	std::sync::Arc,
	tokio::sync::{OnceCell, broadcast, oneshot},
	tracing::{error, info, warn},
};

static SHUTDOWN: OnceCell<broadcast::Sender<()>> = OnceCell::const_new();

/// 初始化 shutdown sender
pub fn init_shutdown() {
	let (tx, _) = broadcast::channel(1);
	let _ = SHUTDOWN.set(tx);
}

/// 获取 shutdown sender
fn get_shutdown() -> &'static broadcast::Sender<()> {
	SHUTDOWN.get().expect("SHUTDOWN not initialized")
}

/// 获取 shutdown receiver
pub fn get_shutdown_receiver() -> broadcast::Receiver<()> {
	get_shutdown().subscribe()
}

/// 发送 shutdown 信号
pub fn send_shutdown() {
	let _ = get_shutdown().send(());
}

/// 从Redis缓存加载初始深度数据
pub async fn load_initial_depths(storage: Arc<DepthStorage>) -> anyhow::Result<()> {
	info!("Loading initial depths from Redis cache...");

	let mut conn = redis_pool::get_cache_redis_connection().await?;

	// 使用 scan_match 迭代扫描所有 depth:: 开头的key
	let mut iter: redis::AsyncIter<String> = conn.scan_match("depth::*").await?;
	let mut keys = Vec::new();

	// 先收集所有keys（迭代器持有连接的借用）
	while let Some(key_result) = iter.next_item().await {
		match key_result {
			Ok(key) => keys.push(key),
			Err(e) => warn!("Failed to scan key: {}", e),
		}
	}
	drop(iter); // 释放迭代器，归还连接借用

	let mut depth_count = 0;
	let key_count = keys.len();

	for key in keys {
		// key格式: depth::event_id::market_id
		// value是WebSocketDepth JSON（market级别，包含两个token）
		let depth_json: Option<String> = conn.get(&key).await?;

		if let Some(json) = depth_json {
			match serde_json::from_str::<WebSocketDepth>(&json) {
				Ok(ws_depth) => {
					storage.init_depth(ws_depth);
					depth_count += 1;
				}
				Err(e) => {
					warn!("Failed to parse depth for key {}: {}", key, e);
				}
			}
		}
	}

	info!("Loaded {} depth snapshots from {} keys", depth_count, key_count);
	Ok(())
}

/// 启动消费任务
/// startup_complete_tx: 用于通知main函数启动窗口已完成
pub async fn start_consumer_task(storage: Arc<DepthStorage>, startup_complete_tx: oneshot::Sender<()>) {
	let mut shutdown_receiver = get_shutdown_receiver();

	// 获取Redis连接
	let mut conn = match redis_pool::get_websocket_mq_connection().await {
		Ok(conn) => conn,
		Err(e) => {
			error!("Failed to get redis connection: {}", e);
			return;
		}
	};

	let mut last_id = "$".to_string();

	// 20s启动窗口定时器（只触发一次）
	let startup_deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(DEPTH_BUILD_WINDOW_SECS);
	let mut startup_timer = std::pin::pin!(tokio::time::sleep_until(startup_deadline));
	let mut startup_complete_tx = Some(startup_complete_tx);

	/*
	对每个分支的 future 调用 .poll()
	如果某个 future 返回 Poll::Ready 且 guard 条件为 true → 执行对应代码
	如果 future 返回 Poll::Ready 但 guard 为 false → 视为“未就绪”，继续 poll 其他分支
	如果所有分支都未就绪 → 当前任务挂起，等待任意一个 future 就绪
		*/
	loop {
		tokio::select! {
			result = read_messages(&mut conn, &storage, &mut last_id) => {
				match result {
					Ok(_) => {
						// 成功处理一批消息
					}
					Err(e) => {
						error!("Error reading messages: {}, reconnecting...", e);
						match redis_pool::get_websocket_mq_connection().await {
							Ok(new_conn) => {
								conn = new_conn;
								info!("Redis connection reestablished");
							}
							Err(e) => {
								error!("Failed to reconnect: {}", e);
								tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
							}
						}
					}
				}
			}

			_ = &mut startup_timer, if startup_complete_tx.is_some() => {
				// 20s窗口结束，完成所有深度副本的构建
				storage.finalize_all_depths();
				storage.mark_startup_ready();
				// 通知main函数启动窗口已完成，可以启动WebSocket服务器
				if let Some(tx) = startup_complete_tx.take() {
					let _ = tx.send(());
				}
			}

			_ = shutdown_receiver.recv() => {
				info!("Consumer received shutdown signal");
				break;
			}
		}
	}
}

/// 读取并处理消息
async fn read_messages<C: redis::AsyncCommands>(conn: &mut C, storage: &Arc<DepthStorage>, last_id: &mut String) -> anyhow::Result<()> {
	// 使用 XREAD 批量读取消息，使用短超时以便检查启动窗口
	let options = StreamReadOptions::default().count(DEPTH_EVENT_BATCH_SIZE).block(1000);
	let keys = vec![WEBSOCKET_STREAM];
	let ids = vec![last_id.as_str()];
	let result: redis::RedisResult<Option<redis::streams::StreamReadReply>> = conn.xread_options(&keys, &ids, &options).await;

	match result {
		Ok(Some(reply)) => {
			let mut message_ids = Vec::new();

			for stream_key in reply.keys {
				if stream_key.key == WEBSOCKET_STREAM {
					for stream_id in stream_key.ids {
						let msg_id = stream_id.id.clone();
						message_ids.push(msg_id.clone());

						if let Some(fields) = stream_id.map.get(WEBSOCKET_MSG_KEY) {
							match from_redis_value::<String>(fields) {
								Ok(value) => {
									// 尝试解析为不同的消息类型
									process_message(storage, &value);
								}
								Err(e) => {
									error!("Failed to convert redis value to string: {}", e);
								}
							}
						}
					}
				}
			}

			// 使用 XDEL 删除消息
			if !message_ids.is_empty()
				&& let Some(last_msg_id) = message_ids.last()
			{
				*last_id = last_msg_id.clone();
			}

			// 多副本 采用自动保留长度的方式
			// let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
			// let _: redis::RedisResult<usize> = conn.xdel(WEBSOCKET_STREAM, &ids).await;
		}
		Ok(None) => {
			// 没有消息，继续等待
		}
		Err(e) => {
			error!("Redis XREAD error: {}", e);
		}
	}

	Ok(())
}

/// 处理单条消息
fn process_message(storage: &Arc<DepthStorage>, value: &str) {
	// 首先尝试解析为 WebSocketDepth
	if let Ok(depth) = serde_json::from_str::<WebSocketDepth>(value) {
		storage.handle_depth_event(depth);
		return;
	}

	// 然后尝试解析为 WebSocketPriceChanges
	if let Ok(changes) = serde_json::from_str::<WebSocketPriceChanges>(value) {
		storage.handle_price_changes_event(changes);
	}

	// 无法识别的消息类型，忽略
	// 可能是其他类型的websocket消息（如用户事件）
}
