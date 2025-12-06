use {
	crate::{
		consts::{HEARTBEAT_CHECK_INTERVAL_SECS, HEARTBEAT_TIMEOUT_SECS},
		storage::UserStorage,
	},
	axum::{
		extract::{
			State, WebSocketUpgrade,
			ws::{Message, WebSocket},
		},
		response::IntoResponse,
	},
	common::common_env,
	futures_util::{SinkExt, StreamExt},
	serde::{Deserialize, Serialize},
	std::{collections::HashMap, sync::Arc, time::Duration},
	tokio::sync::{RwLock, mpsc},
	tracing::{error, info, warn},
};

#[derive(Serialize)]
struct ConnectedMessage<'a> {
	event_type: &'a str,
	id: &'a str,
}

#[derive(Serialize)]
struct AuthResponse {
	event_type: &'static str,
	success: bool,
}

/// 鉴权消息格式
#[derive(Debug, Deserialize)]
struct AuthMessage {
	auth: String,
}

/// 应用状态
#[derive(Clone)]
pub struct AppState {
	pub storage: Arc<RwLock<UserStorage>>,
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
	// 元组: (tracking_key, update_id, json_string)
	let (tx, mut rx) = mpsc::unbounded_channel::<(String, i64, String)>();

	// 鉴权状态
	let mut authenticated = false;
	let mut privy_id: Option<String> = None;

	// 每个连接独立维护 update_id 跟踪
	let mut last_update_ids: HashMap<String, i64> = HashMap::new();

	// 心跳追踪
	let mut last_heartbeat = tokio::time::Instant::now();
	let mut heartbeat_interval = tokio::time::interval_at(tokio::time::Instant::now() + Duration::from_secs(HEARTBEAT_CHECK_INTERVAL_SECS), Duration::from_secs(HEARTBEAT_CHECK_INTERVAL_SECS));

	// 获取JWT验证所需的环境变量
	let env = common_env::get_common_env();
	let pem_key = env.privy_pem_key.clone();
	let app_id = env.privy_app_id.clone();

	loop {
		tokio::select! {
			// 处理来自Redis stream的消息（通过channel转发）
			Some((tracking_key, update_id, json_string)) = rx.recv() => {
				// 只有鉴权成功后才发送消息
				if !authenticated {
					continue;
				}

				// 检查 update_id 是否应该发送
				let should_send = last_update_ids
					.get(&tracking_key)
					.map(|last_id| update_id > *last_id)
					.unwrap_or(true);

				if !should_send {
					// 跳过过时的消息
					continue;
				}

				// 直接发送已序列化的 JSON 字符串
				if ws_sender.send(Message::Text(json_string.into())).await.is_err() {
					break;
				}

				// 更新 last_update_id
				last_update_ids.insert(tracking_key, update_id);
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
									// 检查是否已鉴权 - 第一次收到心跳时必须已经鉴权
									if !authenticated {
										warn!("Connection {} sent heartbeat before auth, disconnecting", conn_id);
										break;
									}
									last_heartbeat = tokio::time::Instant::now();
									continue;
								}

								// 处理JSON消息
								if !authenticated {
									// 未鉴权状态，只接受鉴权消息
									match serde_json::from_str::<AuthMessage>(&text_str) {
										Ok(auth_msg) => {
											// 验证JWT token
											match common::privy_jwt::check_privy_jwt(&auth_msg.auth, &pem_key, &app_id) {
												Ok(user_id) => {
													authenticated = true;
													privy_id = Some(user_id.clone());
													last_heartbeat = tokio::time::Instant::now();

													// 注册连接到storage
													state.storage.write().await.register_connection(
														user_id.clone(),
														conn_id.clone(),
														tx.clone(),
													);

													info!("Connection {} authenticated as user {}", conn_id, user_id);

													// 发送鉴权成功响应
													let response = AuthResponse { event_type: "auth", success: true };
													if ws_sender.send(Message::Text(serde_json::to_string(&response).expect("serialize auth response").into())).await.is_err() {
														break;
													}
												}
												Err(e) => {
													warn!("Connection {} auth failed: {}", conn_id, e);
													// 发送鉴权失败响应并断开连接
													let response = AuthResponse { event_type: "auth", success: false };
													let _ = ws_sender.send(Message::Text(serde_json::to_string(&response).expect("serialize auth response").into())).await;
													break;
												}
											}
										}
										Err(_) => {
											// 第一条消息不是鉴权消息，断开连接
											warn!("Connection {} first message is not auth message, disconnecting", conn_id);
											break;
										}
									}
								} else {
									// 已鉴权状态，收到非ping的消息直接断开
									warn!("Connection {} sent unexpected message after auth, disconnecting", conn_id);
									break;
								}
							}
							Message::Close(_) => {
								info!("Connection {} received close message", conn_id);
								break;
							}
							_ => {} // 忽略其他消息类型（包括 Ping/Pong/Binary）
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
				// 未鉴权状态下，超过心跳检查间隔还没鉴权就断开
				if !authenticated {
					warn!("Connection {} auth timeout, disconnecting", conn_id);
					break;
				}
				// 已鉴权状态下，检查心跳超时
				if last_heartbeat.elapsed() > Duration::from_secs(HEARTBEAT_TIMEOUT_SECS) {
					warn!("Connection {} heartbeat timeout, disconnecting", conn_id);
					break;
				}
			}
		}
	}

	// 清理连接
	if let Some(user_id) = privy_id {
		state.storage.write().await.unregister_connection(&user_id, &conn_id);
	}
	info!("Connection {} closed", conn_id);
}
