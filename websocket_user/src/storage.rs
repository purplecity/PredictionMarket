use {
	common::websocket_types::{ServerMsg, UserEvent},
	std::collections::HashMap,
	tokio::sync::mpsc,
	tracing::error,
};

/// 用户连接句柄，用于向客户端发送消息
/// 元组: (tracking_key, update_id, json_string)
pub type UserSender = mpsc::UnboundedSender<(String, i64, String)>;

/// 单个用户的所有连接
#[derive(Default)]
pub struct UserConnections {
	/// 连接ID -> 发送器
	connections: HashMap<String, UserSender>,
}

impl UserConnections {
	// pub fn new() -> Self {
	// 	Self { connections: HashMap::new() }
	// }

	/// 添加一个新连接
	pub fn add_connection(&mut self, conn_id: String, sender: UserSender) {
		self.connections.insert(conn_id, sender);
	}

	/// 移除一个连接
	pub fn remove_connection(&mut self, conn_id: &str) -> bool {
		self.connections.remove(conn_id).is_some()
	}

	/// 获取连接数量
	pub fn connection_count(&self) -> usize {
		self.connections.len()
	}

	/// 向所有连接广播消息
	pub fn broadcast(&self, tracking_key: String, update_id: i64, json_string: String) {
		for sender in self.connections.values() {
			// 忽略发送失败，连接会在 handle_socket 中被清理
			let _ = sender.send((tracking_key.clone(), update_id, json_string.clone()));
		}
	}
}

/// 用户连接存储
#[derive(Default)]
pub struct UserStorage {
	/// privy_id -> 用户的所有连接
	users: HashMap<String, UserConnections>,
}

impl UserStorage {
	pub fn new() -> Self {
		Self { users: HashMap::new() }
	}

	/// 注册一个新的用户连接
	pub fn register_connection(&mut self, privy_id: String, conn_id: String, sender: UserSender) {
		let user_conns = self.users.entry(privy_id.clone()).or_default();
		user_conns.add_connection(conn_id.clone(), sender);
		//info!("User {} registered connection {}, total: {}", privy_id, conn_id, user_conns.connection_count());
	}

	/// 移除用户连接
	pub fn unregister_connection(&mut self, privy_id: &str, conn_id: &str) {
		if let Some(user_conns) = self.users.get_mut(privy_id)
			&& user_conns.remove_connection(conn_id)
		{
			//info!("User {} unregistered connection {}, remaining: {}", privy_id, conn_id, user_conns.connection_count());
			// 如果用户没有更多连接，移除用户条目
			if user_conns.connection_count() == 0 {
				self.users.remove(privy_id);
				//info!("User {} has no more connections, removed from storage", privy_id);
			}
		}
	}

	/// 处理用户事件，转发到对应用户的所有连接
	pub fn handle_user_event(&self, event: UserEvent) {
		let privy_id = event.get_privy_user_id();

		if let Some(user_conns) = self.users.get(privy_id) {
			// 提取 meta 信息
			let tracking_key = event.get_tracking_key();
			let update_id = event.get_update_id();

			// 构造 ServerMsg 并序列化
			let server_msg = ServerMsg { event_id: event.get_event_id(), event_type: event.get_event_type_str().to_string(), data: serde_json::to_value(&event).unwrap_or_default() };

			match serde_json::to_string(&server_msg) {
				Ok(json_string) => {
					user_conns.broadcast(tracking_key, update_id, json_string);
				}
				Err(e) => {
					error!("Failed to serialize server message: {}, event: {:?}", e, event);
				}
			}
		}
	}

	// 获取当前连接的用户数量
	// pub fn user_count(&self) -> usize {
	// 	self.users.len()
	// }

	// 获取总连接数
	// pub fn total_connections(&self) -> usize {
	// 	self.users.values().map(|u| u.connection_count()).sum()
	// }
}
