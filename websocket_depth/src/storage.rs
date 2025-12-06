use {
	crate::{consts::MAX_SUBSCRIPTIONS_PER_CONNECTION, errors::SubscriptionError},
	common::websocket_types::{PriceLevelInfo, SingleTokenPriceInfo, WebSocketDepth, WebSocketPriceChanges},
	dashmap::DashMap,
	std::{
		collections::{HashMap, HashSet},
		sync::atomic::{AtomicBool, Ordering},
	},
	tokio::sync::mpsc,
	tracing::{error, info},
};

/// 连接发送器类型
/// 元组: (Option<(event_id, market_id, update_id)>, json_string)
/// - 价格变化消息: Some((event_id, market_id, update_id))
/// - 深度快照消息: None（handler自己发送深度快照，不通过此channel）
pub type ConnectionSender = mpsc::UnboundedSender<(Option<(i64, i16, u64)>, String)>;

/// 深度Key: (event_id, market_id) - market级别
pub type DepthKey = (i64, i16);

/// 订阅Key: (event_id, market_id)
pub type SubscriptionKey = (i64, i16);

/// 单个token的待处理价格变化
#[derive(Debug, Clone)]
struct PendingPriceChange {
	update_id: u64,
	timestamp: i64,
	latest_trade_price: String,
	bids: Vec<PriceLevelInfo>,
	asks: Vec<PriceLevelInfo>,
}

/// 对于启动时的深度构建:不管update_id了.我们设置20s的时间窗口在本地构建完所有订单簿副本 因为20s内足够推订单簿好几次了 不管event有没有都可以构建完整的副本了
/// market级别的深度副本状态（包含两个token）
#[derive(Debug)]
struct OptionDepthState {
	/// 当前深度快照（market级别，包含两个token的depths HashMap）
	/// Option的原因在于启动期过后的运行过程中 价格变化比深度快照先到是可能的 因为深度快照是在depth中会被延迟推送以减少推送压力 所以可能会晚于价格变化推送
	depth: Option<WebSocketDepth>,
	/// 构建中收集的价格变化（每个token独立存储） key是token_id
	pending_changes: HashMap<String, Vec<PendingPriceChange>>,
}

impl OptionDepthState {
	fn new_empty() -> Self {
		Self { depth: None, pending_changes: HashMap::new() }
	}

	fn new_with_depth(depth: WebSocketDepth) -> Self {
		Self { depth: Some(depth), pending_changes: HashMap::new() }
	}

	/// 用于价格变化比深度快照先到，第一次收到深度快照时的处理
	/// 应用pending中的价格变化到depth并清空pending队列
	fn apply_pending_and_clear(&mut self) {
		if let Some(depth) = &mut self.depth {
			// 按token分别应用pending changes
			for (token_id, pending_list) in std::mem::take(&mut self.pending_changes) {
				let mut pending_list = pending_list;
				pending_list.sort_by_key(|c| c.update_id);

				// 获取或创建该token的depths entry
				// 理论上来说token_id一定存在的只要快照的和价格变化都是带相同token的话
				let token_depth = depth.depths.entry(token_id.clone()).or_insert_with(|| SingleTokenPriceInfo { latest_trade_price: String::new(), bids: Vec::new(), asks: Vec::new() });
				for change in pending_list {
					if change.update_id > depth.update_id {
						// 应用变化
						for bid_change in &change.bids {
							Self::apply_change(&mut token_depth.bids, bid_change);
						}
						for ask_change in &change.asks {
							Self::apply_change(&mut token_depth.asks, ask_change);
						}
						// 更新最新成交价
						if !change.latest_trade_price.is_empty() {
							token_depth.latest_trade_price = change.latest_trade_price.clone();
						}
						depth.update_id = change.update_id;
						depth.timestamp = change.timestamp;
					}
				}
			}
		}
	}

	/// 应用价格变化到深度快照
	/// 启动窗口过后运行中使用
	fn apply_price_changes(&mut self, update_id: u64, timestamp: i64, token_id: &str, token_changes: &SingleTokenPriceInfo) {
		if let Some(depth) = &mut self.depth {
			// 获取或创建该token的depths entry
			// 理论上来说token_id一定存在的只要快照的和价格变化都是带相同token的话
			let token_depth = depth.depths.entry(token_id.to_string()).or_insert_with(|| SingleTokenPriceInfo { latest_trade_price: String::new(), bids: Vec::new(), asks: Vec::new() });
			// 应用bid变化
			for change in &token_changes.bids {
				Self::apply_change(&mut token_depth.bids, change);
			}
			// 应用ask变化
			for change in &token_changes.asks {
				Self::apply_change(&mut token_depth.asks, change);
			}
			// 更新最新成交价
			if !token_changes.latest_trade_price.is_empty() {
				token_depth.latest_trade_price = token_changes.latest_trade_price.clone();
			}
			// 更新update_id和timestamp
			depth.update_id = update_id;
			depth.timestamp = timestamp;
		}
	}

	/// 应用待处理的价格变化（启动窗口结束时使用）
	fn apply_pending_change(&mut self, token_id: &str, pending: &PendingPriceChange) {
		if let Some(depth) = &mut self.depth {
			let token_depth = depth.depths.entry(token_id.to_string()).or_insert_with(|| SingleTokenPriceInfo { latest_trade_price: String::new(), bids: Vec::new(), asks: Vec::new() });
			for change in &pending.bids {
				Self::apply_change(&mut token_depth.bids, change);
			}
			for change in &pending.asks {
				Self::apply_change(&mut token_depth.asks, change);
			}
			// 更新最新成交价
			if !pending.latest_trade_price.is_empty() {
				token_depth.latest_trade_price = pending.latest_trade_price.clone();
			}
			depth.update_id = pending.update_id;
			depth.timestamp = pending.timestamp;
		}
	}

	/// 应用单个价格变化到价格档位列表
	fn apply_change(levels: &mut Vec<PriceLevelInfo>, change: &PriceLevelInfo) {
		if let Some(pos) = levels.iter().position(|l| l.price == change.price) {
			if change.total_quantity == "0" {
				// 数量为0，移除该档位
				levels.remove(pos);
			} else {
				// 更新数量
				levels[pos].total_quantity = change.total_quantity.clone();
			}
		} else if change.total_quantity != "0" {
			// 新档位，插入
			levels.push(change.clone());
		}
	}
}

/// 深度存储 - 管理所有深度副本和连接订阅
/// 不需要跨字段加锁 因为每个字段都是原子操作 没有字段必须同时更改之类的原子性要求
pub struct DepthStorage {
	/// 深度副本: DepthKey -> OptionDepthState（market级别）
	/// depths插入的时机是深度快照和价格档位变化推送都会可能进行插入和更新
	depths: DashMap<DepthKey, OptionDepthState>,

	/// 订阅关系: SubscriptionKey -> 连接ID集合
	subscriptions: DashMap<SubscriptionKey, HashSet<String>>,

	/// 连接信息: conn_id -> (sender, 订阅的key集合)
	connections: DashMap<String, (ConnectionSender, HashSet<SubscriptionKey>)>,

	/// 是否已完成启动构建
	startup_ready: AtomicBool,
}

impl DepthStorage {
	pub fn new() -> Self {
		Self { depths: DashMap::new(), subscriptions: DashMap::new(), connections: DashMap::new(), startup_ready: AtomicBool::new(false) }
	}

	/// 检查是否已过启动窗口
	pub fn is_startup_ready(&self) -> bool {
		// Acquire: 确保能看到 mark_startup_ready 之前的所有写入（如 finalize_all_depths 的修改）
		self.startup_ready.load(Ordering::Acquire)
	}

	/// 标记启动完成
	pub fn mark_startup_ready(&self) {
		// Release: 确保之前的写入（如 finalize_all_depths）对其他线程可见
		self.startup_ready.store(true, Ordering::Release);
		info!("Depth storage startup ready, all replicas are now active");
	}

	/// 初始化深度（从Redis缓存加载）
	pub fn init_depth(&self, depth: WebSocketDepth) {
		let key = (depth.event_id, depth.market_id);
		self.depths.insert(key, OptionDepthState::new_with_depth(depth));
	}

	/*
	处理WebSocketDepth事件（market级别，包含两个token）
	执行逻辑:
	如果depths中不存在 那么直接插入 这是第一次插入depths OptionDepthState的depth有值
	如果depths中存在 那么要看depth是否是None 如果是,意味着这是因为价格变化而导致插入的depths. 直接把当前pending中的内容根据update_id应用到depth 并替换旧的depth,否则要看是否已经启动期了 启动期过了就不管这个 因为后续只管event的变化,在启动期中那要直接替换掉depth
	*/
	pub fn handle_depth_event(&self, depth: WebSocketDepth) {
		let key = (depth.event_id, depth.market_id);

		if let Some(mut state) = self.depths.get_mut(&key) {
			if let Some(old_depth) = &state.depth {
				// depth已存在，要看是否已经启动期了
				// 启动期过了就不管这个 因为后续只管event的变化
				// 在启动期中那要直接替换掉depth（如果update_id更大）
				if !self.is_startup_ready() && depth.update_id > old_depth.update_id {
					state.depth = Some(depth);
				}
			} else {
				// depth为None，说明是因为价格变化而插入的depths
				// 这是第一次插入深度，不管是否在启动期内都要先设置depth，再应用pending并清空
				state.depth = Some(depth);
				state.apply_pending_and_clear();
			}
		} else {
			// 新的event/market组合
			// 不管有没有过完启动窗口 都直接插入 这是第一次插入depths
			self.depths.insert(key, OptionDepthState::new_with_depth(depth));
		}
	}

	/*
	处理WebSocketPriceChanges事件
	WebSocketPriceChanges现在是market级别，changes是HashMap<String, SingleTokenPriceInfo>，key是token_id
	执行逻辑
	如果depths中不存在 那么直接插入 这是第一次插入depths OptionDepthState的depth没有值 pending中push相关changes
	如果depths中存在,那么要看对应的depth是否存在 不存在 那么是连续多次price_change导致的depths插入 直接插入到pending
	depth存在的情况下 要看是否过了启动期 如果在启动期中那么直接push到pending队列中相关changes 如果过了启动期那么根据update_id判断直接应用到depth 收集要广播的changes 最后一起广播 而不是遍历广播
	*/
	pub fn handle_price_changes_event(&self, msg: WebSocketPriceChanges) {
		let key = (msg.event_id, msg.market_id);
		let is_ready = self.is_startup_ready();

		if let Some(mut state) = self.depths.get_mut(&key) {
			// depths中存在，看depth是否存在
			if let Some(depth) = &state.depth {
				// depth存在，看是否过了启动期
				if is_ready {
					// 过了启动期，根据update_id判断直接应用到depth
					if msg.update_id > depth.update_id {
						for (token_id, token_changes) in &msg.changes {
							state.apply_price_changes(msg.update_id, msg.timestamp, token_id, token_changes);
							//只有过了启动期 而且确实能有应用变化才会广播
							self.broadcast_price_changes(&key, &msg);
						}
					}
				} else {
					// 在启动期中，直接push到pending队列
					for (token_id, token_changes) in &msg.changes {
						let pending = PendingPriceChange {
							update_id: msg.update_id,
							timestamp: msg.timestamp,
							latest_trade_price: token_changes.latest_trade_price.clone(),
							bids: token_changes.bids.clone(),
							asks: token_changes.asks.clone(),
						};
						state.pending_changes.entry(token_id.clone()).or_default().push(pending);
					}
				}
			} else {
				// depth不存在 直接插入到pending
				for (token_id, token_changes) in &msg.changes {
					let pending = PendingPriceChange {
						update_id: msg.update_id,
						timestamp: msg.timestamp,
						latest_trade_price: token_changes.latest_trade_price.clone(),
						bids: token_changes.bids.clone(),
						asks: token_changes.asks.clone(),
					};
					state.pending_changes.entry(token_id.clone()).or_default().push(pending);
				}
			}
		} else {
			// depths中不存在，这是第一次插入，depth没有值，pending中push相关changes
			let mut state = OptionDepthState::new_empty();
			for (token_id, token_changes) in &msg.changes {
				let pending = PendingPriceChange {
					update_id: msg.update_id,
					timestamp: msg.timestamp,
					latest_trade_price: token_changes.latest_trade_price.clone(),
					bids: token_changes.bids.clone(),
					asks: token_changes.asks.clone(),
				};
				state.pending_changes.entry(token_id.clone()).or_default().push(pending);
			}
			self.depths.insert(key, state);
		}
	}

	/// 完成所有深度副本的构建（在20s窗口结束时调用）保障当前所有pending的价格变化都被应用
	/// 应用所有pending的价格变化
	pub fn finalize_all_depths(&self) {
		for mut entry in self.depths.iter_mut() {
			// 只有depth存在时才应用pending的变化
			if let Some(depth) = &entry.depth {
				let current_update_id = depth.update_id;
				// 按token分别处理pending changes
				for (token_id, pending_list) in std::mem::take(&mut entry.pending_changes) {
					let mut pending_list = pending_list;
					pending_list.sort_by_key(|c| c.update_id);
					for change in pending_list {
						if change.update_id > current_update_id {
							entry.apply_pending_change(&token_id, &change);
						}
					}
				}
			}
			// 如果depth为None，说明只收到了价格变化但没收到深度快照
			// 保留pending，等待深度快照到来时处理
		}
	}

	/// 注册连接
	pub fn register_connection(&self, conn_id: String, sender: ConnectionSender) {
		self.connections.insert(conn_id, (sender, HashSet::new()));
	}

	/// 注销连接
	pub fn unregister_connection(&self, conn_id: &str) {
		if let Some((_, (_, subscribed_keys))) = self.connections.remove(conn_id) {
			// 从所有订阅中移除该连接
			for key in subscribed_keys {
				if let Some(mut subs) = self.subscriptions.get_mut(&key) {
					subs.remove(conn_id);
				}
			}
		}
	}

	/// 添加订阅
	/*
	连接处理select这两个
	rx.recv()
	ws_receiver.next()
	客户端订阅是在ws_receiver中处理的 所以百分百保证深度快照是第一个发送的
	但是由于下面这3行 因为这3行所涉及的map不是原子性的 是各自保障线程安全的 所以会有快照和价格变化的窗口问题
	比如前两行执行 来了3条价格档位变化 然后get_depth_for_subscription. 那么连接处理那里rx就会积压3条旧消息发送出去
	所以发送的时候要判断update_id是否大于当前深度快照的update_id 如果大于才发送 更准确的来说每一次update_id比上次大才发送
	这个update_id是分event_id/market_id 所以要分别判断 在match_engine中快照的update_id的级别是market_id这一层次.相同market_id下面的token_id共享update_id
	conn.1.insert(sub_key);
	self.subscriptions.entry(sub_key).or_default().insert(conn_id.to_string());
	let depth = self.get_depth_for_subscription(event_id, market_id);
	*/
	pub fn subscribe(&self, conn_id: &str, event_id: i64, market_id: i16) -> Result<WebSocketDepth, SubscriptionError> {
		let sub_key = (event_id, market_id);

		// 先检查是否有深度快照，如果没有则拒绝订阅
		if self.get_depth_for_subscription(event_id, market_id).is_none() {
			return Err(SubscriptionError::DepthNotAvailable);
		}

		// 检查连接是否存在和订阅数量限制
		if let Some(mut conn) = self.connections.get_mut(conn_id) {
			if conn.1.len() >= MAX_SUBSCRIPTIONS_PER_CONNECTION {
				return Err(SubscriptionError::MaxSubscriptionsExceeded);
			}

			// 添加到连接的订阅集合
			conn.1.insert(sub_key);

			// 添加到订阅映射
			self.subscriptions.entry(sub_key).or_default().insert(conn_id.to_string());

			// 再次获取深度快照，确保包含订阅期间可能发生的价格变化
			// 这样可以避免在第一次获取深度和插入订阅之间丢失价格变化事件
			let depth = self.get_depth_for_subscription(event_id, market_id).expect("depth should exist after initial check");

			Ok(depth)
		} else {
			Err(SubscriptionError::ConnectionNotFound)
		}
	}

	/// 取消订阅
	pub fn unsubscribe(&self, conn_id: &str, event_id: i64, market_id: i16) -> Result<(), SubscriptionError> {
		if let Some(mut conn) = self.connections.get_mut(conn_id) {
			let sub_key = (event_id, market_id);

			// 检查是否已订阅
			if !conn.1.remove(&sub_key) {
				return Err(SubscriptionError::SubscriptionNotFound);
			}

			// 从订阅映射中移除
			if let Some(mut subs) = self.subscriptions.get_mut(&sub_key) {
				subs.remove(conn_id);
			}

			Ok(())
		} else {
			Err(SubscriptionError::ConnectionNotFound)
		}
	}

	/// 获取订阅对应的深度快照（market级别，包含两个token）
	/// 只返回depth存在的快照，如果depth为None则返回None
	/// 因为深度快照是延迟推送的 所以为了避免用户订阅订阅不鸟. 那么应该延迟展示给前端市场信息. 对于做市商应该等拿到第一次深度快照才开始操作
	fn get_depth_for_subscription(&self, event_id: i64, market_id: i16) -> Option<WebSocketDepth> {
		let key = (event_id, market_id);
		if let Some(entry) = self.depths.get(&key) { entry.depth.clone() } else { None }
	}

	/// 广播价格变化给订阅者
	fn broadcast_price_changes(&self, sub_key: &SubscriptionKey, changes: &WebSocketPriceChanges) {
		if let Some(subscribers) = self.subscriptions.get(sub_key) {
			let msg = match serde_json::to_string(changes) {
				Ok(m) => m,
				Err(e) => {
					error!("Failed to serialize price changes: {}", e);
					return;
				}
			};

			// 发送带(event_id, market_id, update_id)的元组，handler端可以直接使用而无需反序列化
			let meta = (changes.event_id, changes.market_id, changes.update_id);
			for conn_id in subscribers.iter() {
				if let Some(conn) = self.connections.get(conn_id)
					&& conn.0.send((Some(meta), msg.clone())).is_err()
				{
					// 发送失败，连接可能已关闭
				}
			}
		}
	}
}

impl Default for DepthStorage {
	fn default() -> Self {
		Self::new()
	}
}
