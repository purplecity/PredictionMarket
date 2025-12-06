/*
1. 任何订单的输入其实都是冻结了资产的 所以要有结果去进行资产更新,db更新,链上交互 撮合接受的order input,除了取消失败之外 其他的order input一定有一笔order output到processor
2. 吞吐量处理能力要求是跟数据流转方向是相反的 即output>engine>input
3. input/output的处理能取决于mq的能力 以及撮合机器性能(这就意味着可以多个消费者接受input 发送output) 当然output的处理能力还取决于后续的消费能力
*/

// 用于实时推送给客户端 市场的activity 如果是当前用户还可以弹窗

// 用于实时推送给客户端 市场各个选项的总交易量更新

// 用于实时推送给客户端 市场单个选项的完整订单簿/档位更新/买卖价格更新等
/* ws服务器应该跟bn一样构建一个oderbook副本 然后推给客户端的时候应该跟polyevent一样推送 */

// 用于传递给processor去处理资产/订单/链上交互

// 用于客户端缓存查询

// 用于启动时Load构建内存中的订单和订单簿
/*
这里应该用mmap 用来把文件映射到内存中 操作内存就是操作文件 这种是实时存储方式 是最好的方式
否则就得通过mq把订单簿变化推到mq 然后异步消费存储到redis或者文件中 如果是redis存储这里还不能跟用于客户端缓存查询的redis混用 因为这个用于重载构建内存中的订单和订单簿 这种的话mq和消费存储的数据要不能丢
这种有一个缺点就是因为市场太多 所以不太可能是一个市场个task去通过channel收消息再往mq中发 然后你还要保证输入到mq中的队列的顺序必须前后顺序 然后还要保证输出的能力比input好 所以要搞比input多的发送task才行 先这样写 如果还是不行那就单个市场1s后才推送所有变化的订单去存储 相当于合并1s内的交易变化推送
*/

/*  output综述

output的吞吐能力要比input强 所以我们搞配置文件中变量order_input_consumer_count*2个output tokio task
这4个task负责所有的消息输出 同时用hash的方法任何一个市场的output只会发送到一个task的channel中去 所以一个撮合引擎中的市场之间应该是热度均等 这样能均衡输出
尽量减轻撮合引擎的负担 不要让撮合引擎去做太多的事情 撮合引擎主要只负责撮合 所以output能输出一次就不要输出2次

1: store程序 -- rust
output mq需要一个topic,所有订单的实时变化推到这个topic.
store crate程序用于存储撮合内存中的订单. 它消费这个topic,为了增加消费速度,store消费的订单直接存在store进程的内存中,然后定时比如5s一次存json文件. 建议是存文件因为load的时候需要store全部消费完这个topic的内容,然后才能去load,所以干脆存储文件方式,避免直接启动撮合,撮合启动前需要load store存储的json文件

2: depth程序 -- go
output mq需要2个topic 一个cache topic用于深度快照 一个websocket topic用于推送定时的已归集的价格档位变化/深度快照等 以供后续websocket服务使用
单个市场/选项/结果的撮合引擎,需要同时监听一个每秒tick的信号 把这秒内深度快照以及价格档位的更新 深度快照推到cache topic 把归集价格档位变化推送到websocket topic
depth程序用于存储深度快照 以给客户端查询使用 从缓存topic中拿到之后 放到内存中定期推送到websocket topic
这是为了配合websocket服务器建立深度副本使用
撮合引擎中需要一个UpdateID u64. 每次每秒tick信号处理的时候加1 同时订单快照和归集价格档位变化的推送需要带上这个UpdateID.


3: processor程序 -- go
output mq需要3个topic 一个用于推送所有订单的实时变化 一个用于转发交易activity以供后续websocket服务使用 一个用于statistic topic推送统计交易量变化
processor应该是多副本的 每个副本作为一个消费者组程序去消费topic 处理资产/db订单/链上交互 同时转发交易activity变化到推送到websocket topic
注意订单的缓存是在processor中处理的

4: websocket程序 -- go
websocket首先构建对应撮合引擎的所有深度副本,监听websocket topic.
对于任意一个市场/选项/结果的深度快照副本,构建过程如下
	1.如果收到归集价格档位变化Event那么缓存起来,缓存第一个价格档位变化Event的UpdateID
	2.如果收到深度快照Event,深度快照Event中的UpdateID小于缓存中的第一个价格档位变化Event的UpdateID,那么丢弃等待接收下一次定时的深度快照Event
	3.如果收到深度快照Event,深度快照Event中的UpdateID大于缓存中的第一个价格档位变化Event的UpdateID,那么把缓存中大于深度快照Event的UpdateID的价格档位变化Event更新到深度快照Evnet中,此时构建好深度副本.
	丢弃价格档位变化缓存,对于新的深度快照Event缓存可以忽略,新的价格档位变化Event更新直接同步到深度快照副本中.

如果收到交易activity变化,直接推送给订阅的客户端.
*/

use {
	crate::types::OrderBookDepth,
	common::{
		consts::{DEPTH_MSG_KEY, DEPTH_STREAM, PROCESSOR_MSG_KEY, PROCESSOR_STREAM, STORE_MSG_KEY, STORE_STREAM, WEBSOCKET_MSG_KEY, WEBSOCKET_STREAM},
		processor_types::ProcessorMessage,
		store_types::OrderChangeEvent,
		websocket_types::{PriceLevelInfo, SingleTokenPriceInfo, WebSocketDepth, WebSocketPriceChanges},
	},
	redis::AsyncCommands,
	rust_decimal::Decimal,
	serde_json,
	std::{
		collections::{HashMap, hash_map::DefaultHasher},
		hash::{Hash, Hasher},
		str::FromStr,
		sync::Arc,
	},
	tokio::sync::{
		OnceCell,
		mpsc::{self, UnboundedReceiver, UnboundedSender},
	},
	tracing::{error, info},
};

/// Calculate total_size = price * total_quantity (both are decimal strings)
fn calculate_total_size(price: &str, total_quantity: &str) -> String {
	let price_decimal = Decimal::from_str(price).unwrap_or(Decimal::ZERO);
	let quantity_decimal = Decimal::from_str(total_quantity).unwrap_or(Decimal::ZERO);
	let total_size = price_decimal.checked_mul(quantity_decimal).unwrap_or(Decimal::ZERO);
	total_size.normalize().to_string()
}

/// 输出消息类型（通用输出消息，支持多种输出目标）
#[derive(Debug, Clone)]
pub enum OutputMessage {
	/// 推送到 store 的订单变化事件
	StoreOrderChange(OrderChangeEvent),
	/// 推送到 processor 的订单处理消息
	ProcessorOrderMessage(ProcessorMessage),
	/// 推送到 cache 的深度快照（market级别，包含两个token）
	CacheDepth(WebSocketDepth),
	/// 推送到 websocket 的价格档位变化
	WebSocketPriceChanges(WebSocketPriceChanges),
}

/// 输出任务消息（包含目标 stream 和消息内容）
#[derive(Debug, Clone)]
struct OutputTaskMessage {
	target_stream: String,
	message_key: String,
	message_json: String,
}

/// 全局输出发布器（负责所有类型的输出任务）
#[derive(Debug)]
pub struct OutputPublisher {
	senders: Arc<Vec<UnboundedSender<OutputTaskMessage>>>, // 只读，不需要 RwLock，使用 unbounded channel
}

impl OutputPublisher {
	pub fn new(output_task_count: usize) -> Self {
		let mut senders = Vec::new();
		for i in 0..output_task_count {
			let (sender, receiver) = mpsc::unbounded_channel();
			senders.push(sender);
			// 启动输出任务
			tokio::spawn(Self::output_task(i, receiver));
		}
		Self { senders: Arc::new(senders) }
	}

	/// 推送输出消息（通过 hash 分发到不同的 task）
	pub fn publish(&self, message: OutputMessage) -> anyhow::Result<()> {
		// 根据消息类型确定目标 stream 和 hash key
		// StoreOrderChange 统一使用 event_id，其他使用 event_id|market_id 组合
		let (target_stream, message_key, hash_key, message_json) = match &message {
			OutputMessage::StoreOrderChange(event) => {
				let hash_key = match event {
					OrderChangeEvent::OrderCreated(order) => order.symbol.event_id.to_string(),
					OrderChangeEvent::OrderUpdated(order) => order.symbol.event_id.to_string(),
					OrderChangeEvent::OrderFilled { symbol, .. } => symbol.event_id.to_string(),
					OrderChangeEvent::OrderCancelled { symbol, .. } => symbol.event_id.to_string(),
					OrderChangeEvent::EventAdded(event) => event.event_id.to_string(),
					OrderChangeEvent::EventRemoved(event_id) => event_id.to_string(),
					OrderChangeEvent::MarketUpdateId { event_id, .. } => event_id.to_string(),
				};
				(STORE_STREAM.to_string(), STORE_MSG_KEY.to_string(), hash_key, serde_json::to_string(event)?)
			}
			OutputMessage::ProcessorOrderMessage(msg) => {
				let hash_key = match msg {
					ProcessorMessage::OrderRejected(rejected) => format!("{}|{}", rejected.symbol.event_id, rejected.symbol.market_id),
					ProcessorMessage::OrderSubmitted(submitted) => format!("{}|{}", submitted.symbol.event_id, submitted.symbol.market_id),
					ProcessorMessage::OrderTraded(traded) => format!("{}|{}", traded.taker_symbol.event_id, traded.taker_symbol.market_id),
					ProcessorMessage::OrderCancelled(cancelled) => format!("{}|{}", cancelled.symbol.event_id, cancelled.symbol.market_id),
				};
				(PROCESSOR_STREAM.to_string(), PROCESSOR_MSG_KEY.to_string(), hash_key, serde_json::to_string(msg)?)
			}
			OutputMessage::CacheDepth(depth) => {
				let hash_key = format!("{}|{}", depth.event_id, depth.market_id);
				(DEPTH_STREAM.to_string(), DEPTH_MSG_KEY.to_string(), hash_key, serde_json::to_string(depth)?)
			}
			OutputMessage::WebSocketPriceChanges(msg) => {
				let hash_key = format!("{}|{}", msg.event_id, msg.market_id);
				(WEBSOCKET_STREAM.to_string(), WEBSOCKET_MSG_KEY.to_string(), hash_key, serde_json::to_string(msg)?)
			}
		};

		// 根据 hash_key 选择 task
		let mut hasher = DefaultHasher::new();
		hash_key.hash(&mut hasher);
		let hash_value = hasher.finish();

		let task_index = (hash_value as usize) % self.senders.len().max(1);

		if let Some(sender) = self.senders.get(task_index) {
			let task_msg = OutputTaskMessage { target_stream, message_key, message_json };
			sender.send(task_msg).map_err(|e| anyhow::anyhow!("Failed to send output message: {}", e))?;
		}

		Ok(())
	}

	/// 输出任务：将消息推送到对应的 Redis Stream
	async fn output_task(task_id: usize, mut receiver: UnboundedReceiver<OutputTaskMessage>) {
		info!("Output task {} started", task_id);

		loop {
			match receiver.recv().await {
				Some(task_msg) => {
					// TODO 这里发到mq失败应该自行停止撮合 因为这涉及到发送到存储和processor的关键数据
					if let Err(e) = Self::publish_to_redis(&task_msg).await {
						error!("Output task {}: Failed to publish message to Redis stream {}: {}", task_id, task_msg.target_stream, e);
					}
				}
				None => {
					info!("Output task {}: receiver closed, exiting", task_id);
					break;
				}
			}
		}
	}

	/// 将消息推送到 Redis Stream
	async fn publish_to_redis(task_msg: &OutputTaskMessage) -> anyhow::Result<()> {
		let items: Vec<(&str, &str)> = vec![(&task_msg.message_key, &task_msg.message_json)];

		// 根据 stream 类型选择不同的 Redis 连接和 MAXLEN 设置
		match task_msg.target_stream.as_str() {
			WEBSOCKET_STREAM => {
				// WEBSOCKET_STREAM 发送到 websocket_mq (DB 2) 并使用 MAXLEN
				let mut conn = common::redis_pool::get_websocket_mq_connection().await?;
				let _: Option<String> = conn.xadd_maxlen(&task_msg.target_stream, redis::streams::StreamMaxlen::Approx(common::consts::WEBSOCKET_STREAM_MAXLEN), "*", &items).await?;
			}
			DEPTH_STREAM => {
				// DEPTH_STREAM 发送到 websocket_mq (DB 2)，不使用 MAXLEN 因为 depth 服务会消费掉
				let mut conn = common::redis_pool::get_websocket_mq_connection().await?;
				let _: Option<String> = conn.xadd(&task_msg.target_stream, "*", &items).await?;
			}
			_ => {
				// 其他 stream (STORE_STREAM, PROCESSOR_STREAM) 发送到 engine_output_mq (DB 1)
				let mut conn = common::redis_pool::get_engine_output_mq_connection().await?;
				let _: Option<String> = conn.xadd(&task_msg.target_stream, "*", &items).await?;
			}
		}

		Ok(())
	}
}

pub static OUTPUT_PUBLISHER: OnceCell<OutputPublisher> = OnceCell::const_new();

/// 初始化输出发布器
pub fn init_output_publisher(output_task_count: usize) {
	let publisher = OutputPublisher::new(output_task_count);
	OUTPUT_PUBLISHER.set(publisher).expect("Output publisher already initialized");
	info!("Output publisher initialized with {} output tasks", output_task_count);
}

/// 推送订单变化事件到 store（便捷函数）
pub fn publish_order_change_to_store(event: OrderChangeEvent) -> anyhow::Result<()> {
	if let Some(publisher) = OUTPUT_PUBLISHER.get() {
		publisher.publish(OutputMessage::StoreOrderChange(event))?;
	} else {
		error!("Output publisher not initialized");
		return Err(anyhow::anyhow!("Output publisher not initialized"));
	}
	Ok(())
}

/// 推送 Processor 消息（便捷函数）
pub fn publish_to_processor(msg: ProcessorMessage) -> anyhow::Result<()> {
	if let Some(publisher) = OUTPUT_PUBLISHER.get() {
		publisher.publish(OutputMessage::ProcessorOrderMessage(msg))?;
	} else {
		error!("Output publisher not initialized");
		return Err(anyhow::anyhow!("Output publisher not initialized"));
	}
	Ok(())
}

/// 推送深度快照到 cache topic（market级别，包含两个token）
pub fn publish_cache_depth(
	event_id: i64,
	market_id: i16,
	update_id: u64,
	timestamp: i64,
	token_0_depth: &OrderBookDepth,
	token_1_depth: &OrderBookDepth,
	token_0_latest_trade_price: String,
	token_1_latest_trade_price: String,
) -> anyhow::Result<()> {
	if let Some(publisher) = OUTPUT_PUBLISHER.get() {
		let mut depths = HashMap::new();

		// token_0
		depths.insert(
			token_0_depth.symbol.token_id.clone(),
			SingleTokenPriceInfo {
				latest_trade_price: token_0_latest_trade_price,
				bids: token_0_depth
					.bids
					.iter()
					.map(|pl| PriceLevelInfo { price: pl.price.clone(), total_quantity: pl.total_quantity.clone(), total_size: calculate_total_size(&pl.price, &pl.total_quantity) })
					.collect(),
				asks: token_0_depth
					.asks
					.iter()
					.map(|pl| PriceLevelInfo { price: pl.price.clone(), total_quantity: pl.total_quantity.clone(), total_size: calculate_total_size(&pl.price, &pl.total_quantity) })
					.collect(),
			},
		);

		// token_1
		depths.insert(
			token_1_depth.symbol.token_id.clone(),
			SingleTokenPriceInfo {
				latest_trade_price: token_1_latest_trade_price,
				bids: token_1_depth
					.bids
					.iter()
					.map(|pl| PriceLevelInfo { price: pl.price.clone(), total_quantity: pl.total_quantity.clone(), total_size: calculate_total_size(&pl.price, &pl.total_quantity) })
					.collect(),
				asks: token_1_depth
					.asks
					.iter()
					.map(|pl| PriceLevelInfo { price: pl.price.clone(), total_quantity: pl.total_quantity.clone(), total_size: calculate_total_size(&pl.price, &pl.total_quantity) })
					.collect(),
			},
		);

		let ws_depth = WebSocketDepth { event_id, market_id, update_id, timestamp, depths };
		publisher.publish(OutputMessage::CacheDepth(ws_depth))?;
	} else {
		error!("Output publisher not initialized");
		return Err(anyhow::anyhow!("Output publisher not initialized"));
	}
	Ok(())
}

/// 推送价格档位变化到 websocket topic
/// 两个token的变化共享同一个update_id，一起推送
pub fn publish_websocket_price_changes(event_id: i64, market_id: i16, update_id: u64, timestamp: i64, changes: HashMap<String, SingleTokenPriceInfo>) -> anyhow::Result<()> {
	if let Some(publisher) = OUTPUT_PUBLISHER.get() {
		let msg = WebSocketPriceChanges { event_id, market_id, update_id, timestamp, changes };
		publisher.publish(OutputMessage::WebSocketPriceChanges(msg))?;
	} else {
		error!("Output publisher not initialized");
		return Err(anyhow::anyhow!("Output publisher not initialized"));
	}
	Ok(())
}
