use {common::websocket_types::WebSocketDepth, std::collections::HashMap};

/// 深度存储的 Key: (event_id, market_id) - market级别
pub type DepthKey = (i64, i16);

/// 深度内存存储（market级别，每个market存储一个WebSocketDepth，包含两个token）
#[derive(Debug, Default)]
pub struct DepthStorage {
	depths: HashMap<DepthKey, WebSocketDepth>,
}

impl DepthStorage {
	pub fn new() -> Self {
		Self { depths: HashMap::new() }
	}

	/// 更新深度快照（market级别）
	pub fn update_depth(&mut self, key: DepthKey, depth: WebSocketDepth) {
		self.depths.insert(key, depth);
	}

	/// 获取所有深度快照
	pub fn get_all_depths(&self) -> Vec<WebSocketDepth> {
		self.depths.values().cloned().collect()
	}
}
