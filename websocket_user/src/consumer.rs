use {
	crate::{
		consts::USER_EVENT_BATCH_SIZE,
		storage::{ApiKeyStorage, UserStorage},
	},
	common::{
		consts::{API_KEY_MSG_KEY, API_KEY_STREAM, USER_EVENT_MSG_KEY, USER_EVENT_STREAM},
		redis_pool,
		websocket_types::{ApiKeyEvent, UserEvent},
	},
	redis::{from_redis_value, streams::StreamReadOptions},
	std::sync::Arc,
	tokio::sync::{OnceCell, RwLock, broadcast},
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

/// 启动用户事件消费任务
pub async fn start_consumer_task(storage: Arc<RwLock<UserStorage>>) {
	let mut shutdown_receiver = get_shutdown_receiver();

	// 在 loop 外部获取连接
	let mut conn = match redis_pool::get_websocket_mq_connection().await {
		Ok(conn) => conn,
		Err(e) => {
			error!("Failed to get redis connection: {}", e);
			return;
		}
	};

	let mut last_id = "$".to_string();

	loop {
		tokio::select! {
			// 没有future交叉同时await 所以借用是安全的
			result = read_messages(&mut conn, storage.clone(), &mut last_id) => {
				match result {
					Ok(_) => {
						// 成功处理一批消息，继续下一批
					}
					Err(e) => {
						error!("Error reading messages: {}, reconnecting...", e);
						// 只有可能redis命令连接可能失败，重新获取连接
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
async fn read_messages<C: redis::AsyncCommands>(conn: &mut C, storage: Arc<RwLock<UserStorage>>, last_id: &mut String) -> anyhow::Result<()> {
	// 使用 XREAD 批量读取消息
	let options = StreamReadOptions::default().count(USER_EVENT_BATCH_SIZE).block(5000); // 一直阻塞直到收到消息
	let keys = vec![USER_EVENT_STREAM];
	let ids = vec![last_id.as_str()];
	let result: redis::RedisResult<Option<redis::streams::StreamReadReply>> = conn.xread_options(&keys, &ids, &options).await;

	match result {
		Ok(Some(reply)) => {
			let mut message_ids = Vec::new();

			// 解析消息
			for stream_key in reply.keys {
				if stream_key.key == USER_EVENT_STREAM {
					for stream_id in stream_key.ids {
						let msg_id = stream_id.id.clone();
						message_ids.push(msg_id.clone());

						// 查找消息内容
						if let Some(fields) = stream_id.map.get(USER_EVENT_MSG_KEY) {
							match from_redis_value::<String>(fields) {
								Ok(value) => {
									match serde_json::from_str::<UserEvent>(&value) {
										Ok(event) => {
											// 转发到用户的所有连接
											storage.read().await.handle_user_event(event);
										}
										Err(e) => {
											error!("Failed to parse UserEvent {}: {}", msg_id, e);
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

				// 多副本 采用自动保留长度的方式
				// let ids: Vec<&str> = message_ids.iter().map(|s| s.as_str()).collect();
				// let _: redis::RedisResult<usize> = conn.xdel(USER_EVENT_STREAM, &ids).await;
			}
		}
		Ok(None) => {
			// 没有消息，继续等待
		}
		Err(e) => {
			// 返回错误让外层重连（如 Redis 重启后 broken pipe）
			error!("Redis XREAD error: {}", e);
			return Err(e.into());
		}
	}

	Ok(())
}

/// 启动 API Key 事件消费任务
pub async fn start_api_key_consumer_task(api_key_storage: Arc<RwLock<ApiKeyStorage>>) {
	let mut shutdown_receiver = get_shutdown_receiver();

	// 获取连接
	let mut conn = match redis_pool::get_common_mq_connection().await {
		Ok(conn) => conn,
		Err(e) => {
			error!("Failed to get common_mq redis connection: {}", e);
			return;
		}
	};

	let mut last_id = "$".to_string();

	loop {
		tokio::select! {
			result = read_api_key_messages(&mut conn, &api_key_storage, &mut last_id) => {
				match result {
					Ok(_) => {}
					Err(e) => {
						error!("Error reading api_key messages: {}, reconnecting...", e);
						match redis_pool::get_common_mq_connection().await {
							Ok(new_conn) => {
								conn = new_conn;
								info!("Common MQ Redis connection reestablished");
							}
							Err(e) => {
								error!("Failed to reconnect common_mq: {}", e);
								tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
							}
						}
					}
				}
			}
			_ = shutdown_receiver.recv() => {
				info!("API Key consumer received shutdown signal");
				break;
			}
		}
	}
}

/// 读取并处理 API Key 消息
async fn read_api_key_messages<C: redis::AsyncCommands>(conn: &mut C, api_key_storage: &Arc<RwLock<ApiKeyStorage>>, last_id: &mut String) -> anyhow::Result<()> {
	// 使用 XREAD 批量读取消息
	let options = StreamReadOptions::default().count(100).block(5000); // 阻塞 1 秒
	let keys = vec![API_KEY_STREAM];
	let ids = vec![last_id.as_str()];
	let result: redis::RedisResult<Option<redis::streams::StreamReadReply>> = conn.xread_options(&keys, &ids, &options).await;

	match result {
		Ok(Some(reply)) => {
			let mut message_ids = Vec::new();

			for stream_key in reply.keys {
				if stream_key.key == API_KEY_STREAM {
					for stream_id in stream_key.ids {
						let msg_id = stream_id.id.clone();
						message_ids.push(msg_id.clone());

						if let Some(fields) = stream_id.map.get(API_KEY_MSG_KEY) {
							match from_redis_value::<String>(fields) {
								Ok(value) => {
									match serde_json::from_str::<ApiKeyEvent>(&value) {
										Ok(event) => {
											let mut storage = api_key_storage.write().await;
											match event {
												ApiKeyEvent::Add { api_key, privy_id } => {
													storage.add(api_key.clone(), privy_id.clone());
													info!("Added api_key mapping: {} -> {}", api_key, privy_id);
												}
												ApiKeyEvent::Remove { api_key } => {
													if let Some(privy_id) = storage.remove(&api_key) {
														info!("Removed api_key mapping: {} -> {}", api_key, privy_id);
													} else {
														warn!("Tried to remove non-existent api_key: {}", api_key);
													}
												}
											}
										}
										Err(e) => {
											error!("Failed to parse ApiKeyEvent {}: {}", msg_id, e);
										}
									}
								}
								Err(e) => {
									error!("Failed to convert api_key redis value to string: {}", e);
								}
							}
						}
					}
				}
			}

			// 更新最新的消息ID
			if let Some(last_msg_id) = message_ids.last() {
				*last_id = last_msg_id.clone();
			}
		}
		Ok(None) => {
			// 没有消息，继续等待
		}
		Err(e) => {
			// 返回错误让外层重连（如 Redis 重启后 broken pipe）
			error!("Redis XREAD api_key error: {}", e);
			return Err(e.into());
		}
	}

	Ok(())
}
