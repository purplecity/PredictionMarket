# CURSOR_GPT_5_1_CODEX_UNDERSTANDING

## 1. 项目概览
- Rust 2024 工作区由 13 个 crate 组成（API、撮合、资产、事件、WebSocket 等），通过 Cargo workspace 共享依赖，覆盖从用户下单到链上结算的完整预测市场后端链路。
- 微服务之间主要靠 Redis Stream、gRPC（tonic）与 HTTP（axum）通信，持久化依赖 PostgreSQL、Redis（多库）、以及本地 JSON 快照。
- `common` crate 提供跨服务的配置、连接池、领域模型与 Proto 绑定，是所有二进制的编译依赖；`proto` crate 生成 Asset gRPC 客户端/服务端。
- 仓库提供脚本（`script/*.sh`）与 Docker 规范（`deploy/`, `docker-compose.yml`）帮助一次性启停全部组件、重置数据库或清理 Redis。

## 2. 工作区结构速览
| 路径 | 角色 |
| --- | --- |
| `Cargo.toml` / `Cargo.lock` | 工作区定义与共享依赖 |
| `common/` | 共享库：配置、连接池、常量、MQ/WS/WebSocket 类型等 |
| `proto/` | gRPC 定义与生成代码（主要是资产服务） |
| `api/` | HTTP 入口，负责鉴权、行情查询、下单撤单等 |
| `asset/` | 资产账本 & gRPC 服务，处理充值、扣款、订单冻结等 |
| `match_engine/` | 撮合核心，订阅订单/事件 Stream，驱动订单簿与行情输出 |
| `store/` | 维护订单簿快照到本地 `data/store` 并回放 |
| `processor/` | 承接撮合输出，驱动资产 RPC、WebSocket、链上请求 |
| `depth/` | 读取撮合深度 Stream，维护市场内存深度快照 |
| `websocket_depth/` & `websocket_user/` | Axum WebSocket 服务，公开深度推送 & 用户私有事件 |
| `event/` | 事件创建/关闭管道，扇出到 API、撮合、链上 |
| `onchain_msg/` | 链上交互、事件监听、交易回执消费者 |
| `processor/`, `depth/`, `event/`, `store/`, `onchain_msg/` | 其余 Rust 服务，分别处理订单生命周期不同阶段 |
| `deploy/` | 每个服务的 TOML 配置 + 公共环境变量 |
| `script/` | 启停、状态、迁移、清理等自动化脚本 |
| `docs/` | REST/WebSocket 文档及思考笔记 |
| `data/` | 快照持久化（`store` 使用） |
| `logs/` | 运行日志与 PID 文件 |
| `docker-compose.yml` | 本地多容器编排 |

## 3. 核心服务与模块

### API（HTTP 入口）
- 基于 Axum，初始化通用依赖后同时启动事件消费者与 HTTP Server；依赖 `server::app()` 组装路由，`graceful::shutdown_signal()` 统一优雅停机。

```20:38:api/src/main.rs
#[tokio::main]
async fn main() -> anyhow::Result<()> {
	init::init_all().await?;
	tokio::spawn(consumer::consumer_task());
	let config = config::get_config();
	let addr = config.server.get_addr();
	let listener = tokio::net::TcpListener::bind(&addr).await?;
	let app = server::app()?;
	axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
		.with_graceful_shutdown(graceful::shutdown_signal())
		.await?;
	Ok(())
}
```

- 订单入口 `handle_place_order` 将用户请求转给 Asset RPC，随后把撮合消息写入 Redis `ORDER_INPUT_STREAM`，确保资产与撮合双轨一致。

```217:317:api/src/handlers.rs
let submit_order_msg = SubmitOrderMessage { … };
let order_input_msg = OrderInputMessage::SubmitOrder(submit_order_msg);
let msg_json = serde_json::to_string(&order_input_msg)?;
let mut conn = redis_pool::get_engine_input_mq_connection().await?;
let _: String = conn
	.xadd(ORDER_INPUT_STREAM, "*", &[(ORDER_INPUT_MSG_KEY, msg_json.as_str())])
	.await?;
```

### Asset（gRPC 账本）
- 负责资产冻结/解冻、订单创建校验、拆分合并等业务，暴露 tonic gRPC 接口，和 PostgreSQL 写库相连。

```17:49:asset/src/main.rs
#[tokio::main]
async fn main() -> anyhow::Result<()> {
	init::init_all().await?;
	let config = get_config();
	let addr: SocketAddr = config.grpc.get_addr().parse()?;
	let service = asset_service::create_asset_service();
	let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel::<()>();
	let server_handle = tokio::spawn(async move {
		Server::builder()
			.add_service(service)
			.serve_with_shutdown(addr, async { shutdown_rx.await.ok(); })
			.await
	});
	graceful::graceful_shutdown(shutdown_tx).await?;
	if let Err(e) = server_handle.await { … }
	Ok(())
}
```

### Match Engine（撮合核心）
- 启动时从快照恢复订单簿，随后消费订单与事件 Stream，并将撮合结果扇出到 store/processor/depth。

```15:28:match_engine/src/main.rs
#[tokio::main]
async fn main() -> Result<()> {
	init::init_all().await?;
	load::load().await?;
	crate::input::start_input_consumer().await?;
	info!("Match engine started");
	graceful::wait_for_shutdown().await?;
	Ok(())
}
```

- 输入消费者根据消息类型调用 `submit_order` / `cancel_order`，把请求送入每个市场的 channel。

```282:333:match_engine/src/input.rs
pub async fn process_order_input_message(msg: OrderInputMessage) {
	match msg {
		OrderInputMessage::SubmitOrder(submit_msg) => {
			if let Err(e) = submit_order(submit_msg.clone()).await {
				let rejected = OrderRejected { … };
				if let Err(send_err) = publish_to_processor(ProcessorMessage::OrderRejected(rejected)) {
					error!("Failed to publish order rejected message: {}", send_err);
				}
			}
		}
		OrderInputMessage::CancelOrder(msg) => {
			if let Err(e) = cancel_order(msg).await {
				error!("Failed to cancel order: {}", e);
			}
		}
	}
}
```

### Store（撮合状态持久化）
- `OrderStorage` 监听 `STORE_STREAM`，将事件写入内存并周期性刷写 `data/store/orders_snapshot.json`，同时记录 `last_message_id` 以支持精确回放。

```64:146:store/src/storage.rs
pub async fn handle_order_change(&self, event: OrderChangeEvent, message_id: String) {
	let mut data = self.data.write().await;
	match event {
		OrderChangeEvent::OrderCreated(order) => { … }
		OrderChangeEvent::OrderUpdated(order) => { … }
		OrderChangeEvent::OrderCancelled { … } | OrderChangeEvent::OrderFilled { … } => { … }
		OrderChangeEvent::EventAdded(event_create) => { … }
		OrderChangeEvent::EventRemoved(event_id) => { … }
		OrderChangeEvent::MarketUpdateId { event_id, market_id, update_id } => {
			data.market_update_ids.insert((event_id, market_id), update_id);
		}
	}
	data.last_message_id = Some(message_id);
}
```

### Processor（撮合结果编排）
- 订阅 `PROCESSOR_STREAM`，调用 Asset RPC 结算冻结量，并把 Open Order / Position 变化推到 WebSocket。

```264:335:processor/src/consumer.rs
async fn process_message_inner(message: ProcessorMessage) -> anyhow::Result<()> {
	match message {
		ProcessorMessage::OrderRejected(order_rejected) => handle_order_rejected(order_rejected).await?,
		ProcessorMessage::OrderCancelled(order_cancelled) => handle_order_cancelled(order_cancelled).await?,
		ProcessorMessage::OrderSubmitted(order_submitted) => handle_order_submitted(order_submitted).await?,
		ProcessorMessage::OrderTraded(order_traded) => handle_order_traded(order_traded).await?,
	}
	Ok(())
}
async fn handle_order_traded(msg: OrderTraded) -> anyhow::Result<()> {
	let batches: Vec<&[Trade]> = msg.trades.chunks(consts::TRADE_BATCH_SIZE).collect();
	let mut all_order_update_ids = HashMap::new();
	for batch in batches.into_iter() {
		let order_update_ids = process_trade_batch(batch, &msg, event_id, market_id).await?;
		all_order_update_ids.extend(order_update_ids);
	}
	send_maker_order_updated_events(&msg.trades, event_id, market_id, &all_order_update_ids).await;
	Ok(())
}
```

### 深度与 WebSocket
- `depth` 服务读取 websocket MQ，生成市场级内存快照，并向 `websocket_depth` 推送初始数据。

```38:87:depth/src/consumer.rs
pub async fn start_consumer_task(storage: Arc<RwLock<DepthStorage>>) {
	let mut conn = redis_pool::get_websocket_mq_connection().await?;
	let mut last_id = "0".to_string();
	loop {
		tokio::select! {
			result = read_messages(&mut conn, &storage, &mut last_id, batch_size) => {
				if let Err(e) = result {
					error!("Error reading messages: {}, reconnecting...", e);
					conn = redis_pool::get_websocket_mq_connection().await?;
				}
			}
			_ = shutdown_receiver.recv() => break,
		}
	}
}
```

- `websocket_depth`/`websocket_user` 使用 axum 在 `/depth`、`/user` 暴露 WebSocket，前者无需鉴权、后者首条消息必须携带 Privy JWT（详见 `docs/websocket_depth.md`, `docs/websocket_user.md`）。

### Event（市场路由）
- 事件消费者将 `EventCreate` 校验后写库，并扇出到 API/MatchEngine/Onchain Stream，确保所有组件共享一致的市场定义。

```229:385:event/src/consumer.rs
async fn handle_event_create(event_create: EventCreate) -> anyhow::Result<()> {
	if event_create.markets.is_empty() { … }
	// 构造市场、事件模型并写入数据库
	let event_id = db::insert_event(&event).await?;
	// 扇出到不同 Stream
	let api_msg = ApiEventMqMessage::EventCreate(Box::new(api_create));
	publish_to_stream(API_MQ_STREAM, API_MQ_MSG_KEY, &api_msg).await?;
	let engine_msg = EventInputMessage::AddOneEvent(engine_create);
	publish_to_engine_stream(EVENT_INPUT_STREAM, EVENT_INPUT_MSG_KEY, &engine_msg).await?;
	let onchain_msg = OnchainEventMessage::Create(onchain_create_msg);
	publish_to_stream(ONCHAIN_EVENT_STREAM, ONCHAIN_EVENT_MSG_KEY, &onchain_msg).await?;
	Ok(())
}
```

### Onchain Msg
- `onchain_msg` 同时启动多个消费者，处理撮合回执、链上执行指令以及事件广播。

```15:38:onchain_msg/src/main.rs
#[tokio::main]
async fn main() -> anyhow::Result<()> {
	init::init_all().await?;
	if let Err(e) = trade_response_consumer::start_consumers().await { … }
	if let Err(e) = onchain_action_consumer::start_consumers().await { … }
	if let Err(e) = event_consumer::start_consumer().await { … }
	graceful::shutdown_signal().await;
	Ok(())
}
```

### 共享基础：配置、常量、Proto
- Common 环境变量统一本地/线上配置，字段覆盖 Redis、PostgreSQL、内部 HTTP 与 Privy 鉴权。

```8:57:common/src/common_env.rs
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CommonEnv {
	pub internal_service_host: String,
	pub run_mode: String,
	pub engine_input_mq_redis_host: String,
	pub engine_output_mq_redis_host: String,
	pub websocket_mq_redis_host: String,
	pub common_mq_redis_host: String,
	pub cache_redis_host: String,
	pub lock_redis_host: String,
	pub postgres_read_host: String,
	pub postgres_write_host: String,
	pub asset_rpc_url: String,
	pub privy_pem_key: String,
	pub privy_secret_key: String,
	pub privy_app_id: String,
}
```

- 常量定义了所有 Redis Stream 名称与 DB 编号，保证跨服务一致性。

```14:118:common/src/consts.rs
pub const ORDER_INPUT_STREAM: &str = "order_input_stream";
pub const PROCESSOR_STREAM: &str = "processor_stream";
pub const DEPTH_STREAM: &str = "depth_stream";
pub const USER_EVENT_STREAM: &str = "user_event_stream";
pub const REDIS_DB_COMMON_MQ: u32 = 0;
pub const REDIS_DB_WEBSOCKET_MQ: u32 = 3;
pub const REDIS_DB_CACHE: u32 = 4;
pub const REDIS_DB_LOCK: u32 = 5;
pub const REDIS_DB_ENGINE_INPUT_MQ: u32 = 6;
pub const REDIS_DB_ENGINE_OUTPUT_MQ: u32 = 7;
```

- Asset gRPC 接口在 `proto/asset.proto` 中统一描述，覆盖充值、下单、成交、拆分/合并等动作。

```218:230:proto/asset.proto
service AssetService {
  rpc Deposit(DepositRequest) returns (Response);
  rpc Withdraw(WithdrawRequest) returns (Response);
  rpc CreateOrder(CreateOrderRequest) returns (CreateOrderResponse);
  rpc OrderRejected(OrderRejectedRequest) returns (Response);
  rpc CancelOrder(CancelOrderRequest) returns (ResponseWithUpdateId);
  rpc Trade(TradeRequest) returns (TradeResponse);
  rpc TradeOnchainSendResult(TradeOnchainSendResultRequest) returns (Response);
  rpc Split(SplitRequest) returns (Response);
  rpc Merge(MergeRequest) returns (Response);
  rpc Redeem(RedeemRequest) returns (Response);
}
```

## 4. 核心业务流

### 4.1 订单生命周期（用户下单 → 撮合 → 推送）
1. **HTTP 下单**：`api` 鉴权后解析请求，调用 Asset gRPC 写入资金冻结，再把 `SubmitOrder` 消息写入 `ORDER_INPUT_STREAM`（见 `api/src/handlers.rs` 片段）。
2. **撮合接入**：`match_engine` 消费 `ORDER_INPUT_STREAM`，按 event/market channel 派发订单；校验失败会向 processor 发送拒单。
3. **撮合输出**：撮合成功后写入 `STORE_STREAM`、`PROCESSOR_STREAM`、`DEPTH_STREAM`，更新订单、仓位与深度。
4. **资产/用户编排**：`processor` 逐条处理撮合事件，调用 Asset RPC 更新余额，并通过 WebSocket Stream 推送用户订单/仓位变化。
5. **持久化 & 重放**：`store` 记录增量并周期保存快照，`match_engine` 和 `store` 都可在重启时从 `data/store/orders_snapshot.json` + Redis pending 恢复。
6. **实时前台**：`depth` 与 `websocket_depth` 提供盘口快照与增量，`websocket_user` 提供订单、仓位事件，二者都依赖 Redis MQ 与 Processor 产出的消息。

### 4.2 市场创建/关闭
- 外部管理者通过 `event` 服务推送 `EventCreate/EventClose`；服务会校验 topic、outcome、时间戳并持久化，再一次性广播给 API（缓存/查询）、撮合（加载 market）与链上（合约映射），确保所有组件在同一事件集上运行（参考 `event/src/consumer.rs`）。

### 4.3 深度与广播
- 撮合在 `DEPTH_STREAM`、`WEBSOCKET_STREAM` 输出盘口快照/增量，`depth` 服务将其写入内存结构后推送给 `websocket_depth`；前端需要订阅 `/depth` 并定期 ping（详见 `docs/websocket_depth.md`）。

### 4.4 用户事件与链上交互
- `processor` 将订单/仓位变化复制到 `USER_EVENT_STREAM`，被 `websocket_user` 消费后推送给鉴权连接。
- `onchain_msg` 同步链上动作：接收撮合生成的链上交易（`TRADE_SEND_STREAM`）、监听链上结果并回写 Asset/Processor，保持链上与账本的一致。

## 5. 配置、部署与运维

### 环境与服务配置
- 基础环境位于 `deploy/common.env`，包含 Redis 多库、PostgreSQL 主从、Asset RPC 与 Privy 密钥；所有服务在 `init_all` 中读取。

```1:27:deploy/common.env
run_mode="dev"
engine_input_mq_redis_host="127.0.0.1:8889"
engine_output_mq_redis_host="127.0.0.1:8889"
websocket_mq_redis_host="127.0.0.1:8889"
common_mq_redis_host="127.0.0.1:8889"
cache_redis_host="127.0.0.1:8889"
lock_redis_host="127.0.0.1:8889"
postgres_read_host="127.0.0.1"
postgres_write_host="127.0.0.1"
asset_rpc_url="http://127.0.0.1:5003"
internal_service_host="http://127.0.0.1:5001"
privy_pem_key="-----BEGIN PUBLIC KEY-----\n…"
privy_secret_key="…"
privy_app_id="cm7vmiyfv00itghujg076vjmi"
```

- 每个服务在 `deploy/<service>/dev.toml` 定义日志级别、端口等轻量配置，便于通过环境变量切换。

### Docker & 本地编排
- `docker-compose.yml` 提供 Postgres、Redis 及所有 Rust 服务定义，容器共享 `./logs` 与数据目录，便于本地一键启动。

```4:41:docker-compose.yml
services:
  postgres:
    image: postgres:16-alpine
    environment:
      POSTGRES_DB: prediction_market
  redis:
    image: redis:7-alpine
    command: redis-server --appendonly yes
  asset:
    build:
      dockerfile: deploy/asset/Dockerfile
    depends_on:
      postgres:
        condition: service_healthy
      redis:
        condition: service_healthy
```

### 运维脚本
- `script/start_core.sh` / `start_others.sh` / `stop_all.sh` 管理 10 个 Rust 进程，使用 PID 文件 + `nohup cargo run` 方式。

```3:52:script/start_core.sh
CORE_SERVICES=( "match_engine" "store" "asset" )
for SERVICE in "${CORE_SERVICES[@]}"; do
	PID_FILE="./logs/${SERVICE}.pid"
	if [ -f "$PID_FILE" ] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
		echo "$SERVICE 已在运行"
		continue
	fi
	nohup cargo run --bin "$SERVICE" > "$LOG_FILE" 2>&1 &
	echo $! > "$PID_FILE"
	sleep 1
done
```

- 其他脚本：`reset_database.sh`（重建 Postgres 并执行 `script/database.sql`）、`clear_redis.sh`（清空 6 个 Redis DB）、`start_core.sh`/`start_others.sh`/`status.sh`/`stop_all.sh`，以及 `mock/` 目录下的链路模拟工具。

## 6. 开发与测试实践
- **构建**：任一二进制可通过 `cargo run --bin <service>` 启动；`script/start_core.sh` + `start_others.sh` 封装了启动顺序与日志。
- **格式 & Lint**：仓库附带 `rustfmt.toml` 与 `clippy.toml`，建议在提交前运行 `cargo fmt` / `cargo clippy --all-targets`。
- **数据库迁移**：`script/database.sql` + `script/migrations/` 提供初始化脚本，`reset_database.sh` 可快速重置。
- **测试**：
  - `asset/tests/` 提供端到端账本测试，涵盖下单、撤单、余额校验等。

```33:78:asset/tests/test_create_order.rs
#[tokio::test]
async fn test_create_buy_order() {
	let env = TestEnv::setup().await.unwrap();
	let user_id = generate_test_user_id();
	env.create_test_user(user_id, &privy_id).await.unwrap();
	asset::handlers::handle_deposit(user_id, USDC_TOKEN_ID, usdc_amount, …).await.unwrap();
	let result = asset::handlers::handle_create_order(...).await;
	assert!(result.is_ok());
	assert_eq!(usdc_frozen, volume);
}
```

  - `match_engine/tests/match_engine_tests.rs`（详见 `match_engine/tests/README_TESTS.md`）覆盖 60+ 个订单簿/撮合用例，涵盖限价/市价、交叉撮合、价位变化检测。

## 7. 文档与资料
- `docs/api.md`：REST API 详细规格、鉴权、错误码。
- `docs/websocket_depth.md`、`docs/websocket_user.md`：实时深度与用户事件协议。
- `PROJECT_UNDERSTANDING.md` / `CURSOR_PROJECT_UNDERSTANDING.md` / `CLAUDE*.md`：历史 AI 总结，可与当前文档交叉验证。
- `prompt.md` / `docs/think.txt`：补充实现笔记与思考。

## 8. 风险、注意事项与建议
- **Redis 可靠性**：当前依赖 Redis Stream + 手动 pending 认领，需关注 Redis 单点与消息清理策略，定期确认 `xack_del` 成功率。
- **快照一致性**：`store` 与 `match_engine` 仍依赖本地 JSON；在容器环境需确保 `data/*` 挂载并定期备份，避免单点磁盘故障。
- **链路可观测性**：日志集中在 `logs/*.log`，建议补充指标（Prometheus）与集中化日志收集，方便排查撮合/资产/链上的跨服务问题。
- **测试覆盖**：资产与撮合层已有较完整测试，但 API、processor、WebSocket、onchain 仍缺乏自动化验证，可新增集成测试或契约测试。
- **配置安全**：`deploy/common.env` 当前包含明文密钥，提交仓库前应加密或使用模板避免泄露。

以上内容即本次对整个 workspace 的系统理解，后续若发现新的模块或流程，可在本文件追加对应章节。
