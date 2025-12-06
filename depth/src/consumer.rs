use {
	crate::storage::DepthStorage,
	common::{
		consts::{DEPTH_MSG_KEY, DEPTH_STREAM},
		redis_pool,
		websocket_types::WebSocketDepth,
	},
	redis::{from_redis_value, streams::StreamReadOptions},
	std::sync::Arc,
	tokio::sync::{OnceCell, RwLock, broadcast},
	tracing::{error, info},
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

/// 启动深度消费任务
pub async fn start_consumer_task(storage: Arc<RwLock<DepthStorage>>) {
	let mut shutdown_receiver = get_shutdown_receiver();

	// 获取配置的 batch_size
	let batch_size = crate::config::get_config().consumer.batch_size;

	// 在 loop 外部获取连接 (从 websocket_mq DB 2 读取 depth_stream)
	let mut conn = match redis_pool::get_websocket_mq_connection().await {
		Ok(conn) => conn,
		Err(e) => {
			error!("Failed to get redis connection: {}", e);
			return;
		}
	};
	//跟撮合一样单副本 从0开始 需要删除
	let mut last_id = "0".to_string();

	loop {
		tokio::select! {
			//没有future交叉同时await 所以借用是安全的
			result = read_messages(&mut conn, &storage, &mut last_id, batch_size) => {
				match result {
					Ok(_) => {
						// 成功处理一批消息，继续下一批
					}
					Err(e) => {
						error!("Error reading messages: {}, reconnecting...", e);
						// 只有可能redis命令连接可能失败，重新获取连接
						// 不在不可控边界过度防御，而在可控范围内保持简单高效。
						match redis_pool::get_websocket_mq_connection().await {
							Ok(new_conn) => {
								conn = new_conn;
								info!("Redis connection reestablished");
							}
							Err(e) => {
								error!("Failed to reconnect: {}", e);
								// 等待一段时间后重试
								tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
							}
						}
					}
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
async fn read_messages<C: redis::AsyncCommands>(conn: &mut C, storage: &Arc<RwLock<DepthStorage>>, last_id: &mut String, batch_size: usize) -> anyhow::Result<()> {
	// 使用 XREAD 读取消息，批量读取（batch_size 作为参数传入）
	let options = StreamReadOptions::default().count(batch_size).block(0); // 一直阻塞直到收到消息
	let keys = vec![DEPTH_STREAM];
	let ids = vec![last_id.as_str()];
	let result: redis::RedisResult<Option<redis::streams::StreamReadReply>> = conn.xread_options(&keys, &ids, &options).await;

	match result {
		Ok(Some(reply)) => {
			let mut message_ids = Vec::new();

			// 解析消息
			for stream_key in reply.keys {
				if stream_key.key == DEPTH_STREAM {
					for stream_id in stream_key.ids {
						let msg_id = stream_id.id.clone();
						message_ids.push(msg_id.clone());

						// 查找消息内容
						if let Some(fields) = stream_id.map.get(DEPTH_MSG_KEY) {
							match from_redis_value::<String>(fields) {
								Ok(value) => {
									match serde_json::from_str::<WebSocketDepth>(&value) {
										Ok(depth) => {
											// 存储到内存（market级别）
											let key = (depth.event_id, depth.market_id);
											storage.write().await.update_depth(key, depth);
										}
										Err(e) => {
											error!("Failed to parse WebSocketDepth {}: {}", msg_id, e);
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

			// 使用 XDEL 删除消息
			if !message_ids.is_empty() {
				// 更新最新的消息ID
				if let Some(last_msg_id) = message_ids.last() {
					*last_id = last_msg_id.clone();
				}

				let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
				let _: redis::RedisResult<usize> = conn.xdel(DEPTH_STREAM, &ids).await;
			}
		}
		Ok(None) => {
			// 没有消息，继续等待
		}
		Err(e) => {
			// 打印错误，继续循环
			error!("Redis XREAD error: {}", e);
		}
	}

	Ok(())
}
