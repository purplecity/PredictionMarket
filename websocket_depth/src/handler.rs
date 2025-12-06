use {
	crate::{
		consts::{HEARTBEAT_CHECK_INTERVAL_SECS, HEARTBEAT_TIMEOUT_SECS},
		errors::SubscriptionError,
		storage::DepthStorage,
	},
	axum::{
		extract::{
			State, WebSocketUpgrade,
			ws::{Message, WebSocket},
		},
		response::IntoResponse,
	},
	futures_util::{SinkExt, StreamExt},
	serde::{Deserialize, Serialize},
	std::{collections::HashMap, sync::Arc, time::Duration},
	tokio::sync::mpsc,
	tracing::{error, info, warn},
};

#[derive(Serialize)]
struct ConnectedMessage<'a> {
	event_type: &'a str,
	id: &'a str,
}

/// 客户端消息类型
#[derive(Debug, Deserialize)]
#[serde(tag = "action")]
enum ClientMessage {
	#[serde(rename = "subscribe")]
	Subscribe { event_id: i64, market_id: i16 },
	#[serde(rename = "unsubscribe")]
	Unsubscribe { event_id: i64, market_id: i16 },
}

/// 服务端响应消息
#[derive(Debug, Serialize)]
struct ServerResponse {
	event_type: &'static str,
	success: bool,
	#[serde(skip_serializing_if = "Option::is_none")]
	message: Option<String>,
}

/// 应用状态
#[derive(Clone)]
pub struct AppState {
	pub storage: Arc<DepthStorage>,
}

/// WebSocket升级处理
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
	ws.on_upgrade(|socket| handle_socket(socket, state))
}

/// 处理单个WebSocket连接
async fn handle_socket(socket: WebSocket, state: AppState) {
	let conn_id = uuid::Uuid::new_v4().to_string();
	info!("New WebSocket connection: {}", conn_id);

	let (mut ws_sender, mut ws_receiver) = socket.split();

	// 发送连接成功消息
	let connected_msg = ConnectedMessage { event_type: "connected", id: &conn_id };
	let connected_json = serde_json::to_string(&connected_msg).expect("serialize connected message");
	if ws_sender.send(Message::Text(connected_json.into())).await.is_err() {
		info!("Connection {} failed to send connected message", conn_id);
		return;
	}

	// 创建消息通道用于向客户端发送消息
	// 元组: (Option<(event_id, market_id, update_id)>, json_string)
	// - 价格变化: Some((event_id, market_id, update_id))
	// - 深度快照: None（handler自己发送，不通过此channel）
	let (tx, mut rx) = mpsc::unbounded_channel::<(Option<(i64, i16, u64)>, String)>();

	// 注册连接
	state.storage.register_connection(conn_id.clone(), tx);

	// 心跳追踪
	let mut last_heartbeat = tokio::time::Instant::now();
	let mut heartbeat_interval = tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(HEARTBEAT_CHECK_INTERVAL_SECS), Duration::from_secs(HEARTBEAT_CHECK_INTERVAL_SECS));

	// 订阅的update_id追踪: (event_id, market_id) -> last_update_id
	// 用于过滤过期的价格变化消息，确保只推送update_id大于上次的消息
	let mut subscription_update_ids: HashMap<(i64, i16), u64> = HashMap::new();

	loop {
		tokio::select! {
			// 处理来自storage的推送消息（价格变化）
			Some((meta, msg)) = rx.recv() => {
				// meta: Some((event_id, market_id, update_id)) 表示价格变化，None 表示其他消息
				let should_send = if let Some((event_id, market_id, update_id)) = meta {
					let key = (event_id, market_id);
					if let Some(last_update_id) = subscription_update_ids.get_mut(&key) {
						if update_id > *last_update_id {
							*last_update_id = update_id;
							true
						} else {
							false // update_id不大于上次，跳过
						}
					} else {
						false // 未订阅该event/market，跳过
					}
				} else {
					true // 非价格变化消息，直接发送
				};

				if should_send && ws_sender.send(Message::Text(msg.into())).await.is_err() {
					break;
				}
			}

			// 处理来自客户端的消息
			Some(result) = ws_receiver.next() => {
				match result {
					Ok(msg) => {
						match msg {
							Message::Text(text) => {
								let text_str = text.to_string();

								// 处理心跳消息
								if text_str == "ping" {
									last_heartbeat = tokio::time::Instant::now();
									continue;
								}

								// 处理客户端消息
								match serde_json::from_str::<ClientMessage>(&text_str) {
									Ok(ClientMessage::Subscribe { event_id, market_id }) => {
										match handle_subscribe(&state, &conn_id, event_id, market_id, &mut ws_sender).await {
											Some(update_id) if update_id > 0 => {
												// 订阅成功，初始化update_id追踪
												subscription_update_ids.insert((event_id, market_id), update_id);
												last_heartbeat = tokio::time::Instant::now();
											}
											Some(_) => {
												// 订阅失败但不需要断开连接（如超过订阅限制、深度不可用）
												last_heartbeat = tokio::time::Instant::now();
											}
											None => {
												// 订阅失败导致需要断开连接
												break;
											}
										}
									}
									Ok(ClientMessage::Unsubscribe { event_id, market_id }) => {
										handle_unsubscribe(&state, &conn_id, event_id, market_id, &mut ws_sender).await;
										// 取消订阅时移除update_id追踪
										subscription_update_ids.remove(&(event_id, market_id));
										last_heartbeat = tokio::time::Instant::now();
									}
									Err(_) => {
										// 无法识别的消息格式，断开连接
										warn!("Connection {} sent invalid message format, disconnecting", conn_id);
										let response = ServerResponse {
											event_type: "error",
											success: false,
											message: Some("Invalid message format".to_string()),
										};
										let _ = ws_sender.send(Message::Text(serde_json::to_string(&response).expect("serialize response").into())).await;
										break;
									}
								}
							}
							Message::Close(_) => {
								info!("Connection {} received close message", conn_id);
								break;
							}
							_ => {} // 忽略其他消息类型
						}
					}
					Err(e) => {
						error!("Connection {} websocket error: {}", conn_id, e);
						break;
					}
				}
			}

			// 检查心跳超时
			_ = heartbeat_interval.tick() => {
				if last_heartbeat.elapsed() > Duration::from_secs(HEARTBEAT_TIMEOUT_SECS) {
					warn!("Connection {} heartbeat timeout, disconnecting", conn_id);
					break;
				}
			}
		}
	}

	// 清理连接
	state.storage.unregister_connection(&conn_id);
	info!("Connection {} closed", conn_id);
}

/// 处理订阅请求
/// 返回 Some(update_id) 表示订阅成功，None 表示失败需要断开连接
async fn handle_subscribe<S>(state: &AppState, conn_id: &str, event_id: i64, market_id: i16, ws_sender: &mut S) -> Option<u64>
where
	S: SinkExt<Message> + Unpin,
	<S as futures_util::Sink<Message>>::Error: std::fmt::Debug,
{
	match state.storage.subscribe(conn_id, event_id, market_id) {
		Ok(depth) => {
			let update_id = depth.update_id;

			// 发送订阅成功响应
			let response = ServerResponse { event_type: "subscribed", success: true, message: None };

			if ws_sender.send(Message::Text(serde_json::to_string(&response).expect("serialize response").into())).await.is_err() {
				return None;
			}

			// 发送当前深度快照（market级别，包含两个token）
			match serde_json::to_string(&depth) {
				Ok(depth_json) => {
					if ws_sender.send(Message::Text(depth_json.into())).await.is_err() {
						return None;
					}
				}
				Err(e) => {
					error!("Failed to serialize depth: {}", e);
				}
			}

			info!("Connection {} subscribed to event_id={}, market_id={}", conn_id, event_id, market_id);
			Some(update_id)
		}
		Err(err) => {
			// 发送错误响应
			let response = ServerResponse { event_type: "subscribe_error", success: false, message: Some(err.to_string()) };

			let _ = ws_sender.send(Message::Text(serde_json::to_string(&response).expect("serialize response").into())).await;

			match err {
				SubscriptionError::MaxSubscriptionsExceeded | SubscriptionError::DepthNotAvailable => {
					warn!("Connection {} subscribe failed: {}", conn_id, err);
					// 不断开连接，只是拒绝新订阅，返回特殊值表示不需要断开但订阅失败
					Some(0) // 返回0表示订阅失败但不断开，调用方不会插入到map中因为后续逻辑会处理
				}
				_ => None,
			}
		}
	}
}

/// 处理取消订阅请求
/*
S 是一个泛型类型参数，代表 WebSocket 的发送端（sender）
Sink发送 Stream接收
SinkExt 是 futures_util::sink::SinkExt/StreamExt 异步发送和接收
Sink 提供了便利方法，比如 .send().await
StreamExt 提供了便利方法，比如 .next().await

Rust 的 future 默认是 !Unpin（不能移动） 3重关系 pin(固定也就是不能移动)->unpin(不固定,可以移动)->!Unpin(又变成了不能移动 !再次否的意思)
单纯&mut S类型是不要求移动的 但是你加了async 那就要求Unpin了 因为tokio调度会把future可能会跨线程 所以要求unpin

<S as futures_util::Sink<Message>>::Error: std::fmt::Debug
把 S 当作 Sink<Message> 来看待
每个 Sink<Item> 都有一个关联类型 Error，表示发送失败时的错误类型
例如：网络断开、序列化失败等
: std::fmt::Debug
要求这个 Error 类型必须实现 Debug
因为 let _ = ws_sender.send(xxx)
Rust 要求：如果错误类型不能 Debug，编译器会警告"可能丢失重要错误信息"
加上这个约束后，即使你忽略错误（let _ = ...），编译器也知道这个错误至少能被打印出来（用于日志、panic 等）。
*/
async fn handle_unsubscribe<S>(state: &AppState, conn_id: &str, event_id: i64, market_id: i16, ws_sender: &mut S)
where
	S: SinkExt<Message> + Unpin,
	<S as futures_util::Sink<Message>>::Error: std::fmt::Debug,
{
	match state.storage.unsubscribe(conn_id, event_id, market_id) {
		Ok(()) => {
			let response = ServerResponse { event_type: "unsubscribed", success: true, message: None };
			let _ = ws_sender.send(Message::Text(serde_json::to_string(&response).expect("serialize response").into())).await;
			info!("Connection {} unsubscribed from event_id={}, market_id={}", conn_id, event_id, market_id);
		}
		Err(err) => {
			let response = ServerResponse { event_type: "unsubscribe_error", success: false, message: Some(err.to_string()) };
			let _ = ws_sender.send(Message::Text(serde_json::to_string(&response).expect("serialize response").into())).await;
		}
	}
}
