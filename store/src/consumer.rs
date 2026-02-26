use {
	crate::{config::get_config, storage::OrderStorage},
	common::{
		consts::{STORE_MSG_KEY, STORE_STREAM},
		redis_pool,
		store_types::OrderChangeEvent,
	},
	redis::from_redis_value,
	serde_json,
	std::sync::Arc,
	tokio::sync::{OnceCell, broadcast},
	tracing::{error, info},
};

//因为消息都是内存处理 还是直接单task处理 没必要再dispatch多个task去处理
//如果真的这个stream消息很多 那就应该本身就是分成多个stream去处理的

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

/// 消费者任务
pub async fn consumer_task(storage: Arc<OrderStorage>, initial_last_id: Option<String>) {
	let config = get_config();
	let batch_size = config.engine_output_mq.batch_size;

	let mut shutdown_receiver = get_shutdown_receiver();

	// 在 loop 外部获取连接
	let mut conn = match redis_pool::get_engine_output_mq_connection().await {
		Ok(conn) => conn,
		Err(e) => {
			error!("Failed to get redis connection: {}", e);
			return;
		}
	};

	// 从快照加载的消息 ID 开始，如果没有则从 "0" 开始
	let mut last_id = initial_last_id.unwrap_or_else(|| "0".to_string());

	// 如果有初始消息 ID，先清理小于该 ID 的消息
	if last_id != "0"
		&& let Err(e) = cleanup_old_messages(&mut conn, &last_id).await
	{
		error!("Failed to cleanup old messages on startup: {}", e);
	}

	loop {
		tokio::select! {
			// 没有 future 交叉同时 await，所以借用是安全的
			result = read_messages(batch_size,&mut conn, &mut last_id, storage.clone()) => {
				match result {
					Ok(_) => {
						// 成功处理一批消息，继续下一批
					}
					Err(e) => {
						error!("Error reading messages: {}, reconnecting...", e);
						// 只有可能 redis 命令连接可能失败，重新获取连接
						match redis_pool::get_engine_output_mq_connection().await {
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
async fn read_messages<C: redis::AsyncCommands>(batch_size: usize, conn: &mut C, last_id: &mut String, storage: Arc<OrderStorage>) -> anyhow::Result<()> {
	// 使用 XREAD 读取消息，从配置文件读取 batch_size
	let options = redis::streams::StreamReadOptions::default().count(batch_size).block(5000); // 一直阻塞直到收到消息
	let keys = vec![STORE_STREAM];
	let ids = vec![last_id.as_str()];
	let result: redis::RedisResult<Option<redis::streams::StreamReadReply>> = conn.xread_options(&keys, &ids, &options).await;

	match result {
		Ok(Some(reply)) => {
			// 解析消息
			for stream_key in reply.keys {
				if stream_key.key == STORE_STREAM {
					for stream_id in stream_key.ids {
						let msg_id = stream_id.id.clone();
						// 直接更新 last_id，循环结束后 last_id 就是最后一条消息的 ID
						*last_id = msg_id.clone();

						// 查找消息内容
						if let Some(fields) = stream_id.map.get(STORE_MSG_KEY) {
							match from_redis_value::<String>(fields) {
								Ok(value) => {
									match serde_json::from_str::<OrderChangeEvent>(&value) {
										Ok(event) => {
											// handle_order_change 内部会更新消息 ID
											storage.handle_order_change(event, msg_id).await;
										}
										Err(e) => {
											error!("Failed to deserialize order change event: {}", e);
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
		}
		Ok(None) => {
			// 没有消息，继续等待
		}
		Err(e) => {
			error!("Redis XREAD error: {}", e);
			return Err(e.into());
		}
	}

	Ok(())
}

/// 删除 Redis stream 中小于等于指定消息 ID 的所有消息
async fn cleanup_old_messages<C: redis::AsyncCommands>(conn: &mut C, last_message_id: &str) -> anyhow::Result<()> {
	// 使用 XTRIM 删除小于 last_message_id 的消息
	// XTRIM 的 MINID 选项会删除 ID 小于指定 ID 的消息（不包括 last_message_id 本身）
	let options = redis::streams::StreamTrimOptions::minid(redis::streams::StreamTrimmingMode::Exact, last_message_id);
	let deleted_count: redis::RedisResult<usize> = conn.xtrim_options(STORE_STREAM, &options).await;

	match deleted_count {
		Ok(count) => {
			info!("Cleaned up {} messages up to (excluding) {} in stream {}", count, last_message_id, STORE_STREAM);
		}
		Err(e) => {
			error!("Failed to cleanup old messages with XTRIM: {}", e);
			return Err(e.into());
		}
	}

	// 单独删除 last_message_id 这条消息本身
	let del_result: redis::RedisResult<i64> = conn.xdel(STORE_STREAM, &[last_message_id]).await;
	match del_result {
		Ok(del_count) => {
			if del_count > 0 {
				info!("Deleted last_message_id {} itself from stream {}", last_message_id, STORE_STREAM);
			}
		}
		Err(e) => {
			error!("Failed to delete last_message_id {}: {}", last_message_id, e);
			return Err(e.into());
		}
	}

	Ok(())
}
