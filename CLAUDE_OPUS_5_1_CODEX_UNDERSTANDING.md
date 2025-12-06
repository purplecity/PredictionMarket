# CLAUDE_OPUS_5_1_CODEX_UNDERSTANDING.md

> Claude Opus 4.5 对 PredictionMarket 预测市场撮合引擎系统的深度理解文档
> 生成时间: 2025-12-03

---

## 一、系统全景

### 1.1 项目定位

这是一个**生产级预测市场撮合引擎系统**，采用 Rust 语言开发，具备以下特点：

- **微服务架构**：11 个独立服务，职责分离
- **事件驱动**：基于 Redis Streams 的异步消息传递
- **高性能撮合**：纯内存订单簿，价格-时间优先匹配
- **交叉撮合**：预测市场特有的对立结果互补撮合机制
- **实时推送**：WebSocket 秒级深度和用户事件推送

### 1.2 技术栈

| 层次 | 技术选型 |
|------|----------|
| 语言 | Rust 2021 Edition |
| 异步运行时 | Tokio 1.48 (full features) |
| Web 框架 | Axum 0.8 |
| RPC | Tonic 0.14 (gRPC) |
| 数据库 | PostgreSQL + SQLx 0.8 |
| 消息队列 | Redis 0.32 (Streams) |
| 序列化 | Serde + Prost |
| 日志 | Tracing + tracing-subscriber |

---

## 二、Workspace 结构

```
PredictionMarket/
├── Cargo.toml                 # Workspace 定义，11 个 crate
├── common/                    # 共享类型库
├── proto/                     # gRPC Protobuf 定义
├── match_engine/              # 核心撮合引擎 ⭐
├── api/                       # REST API 服务
├── store/                     # 订单持久化服务
├── processor/                 # 交易处理服务
├── asset/                     # 资产管理 gRPC 服务
├── depth/                     # 深度缓存服务
├── event/                     # 市场生命周期管理
├── websocket_depth/           # 实时深度 WebSocket
├── websocket_user/            # 用户事件 WebSocket
├── onchain_msg/               # 链上消息处理
├── deploy/                    # 服务配置 (dev/prod TOML)
├── script/                    # 数据库脚本、Mock 客户端
├── data/                      # 运行时数据（快照）
└── logs/                      # 服务日志
```

---

## 三、核心服务详解

### 3.1 match_engine - 撮合引擎 (核心)

**文件**: `match_engine/src/`

这是整个系统的心脏，负责订单撮合。

#### 架构设计

```
                    ┌─────────────────────────────┐
                    │         Manager             │
                    │  (全局管理器, 维护stop状态)   │
                    └─────────────────────────────┘
                                  │
                    ┌─────────────┴─────────────┐
                    │      EventManager         │
                    │  (每个 Event 一个实例)     │
                    │  - order_senders          │
                    │  - exit_signal            │
                    │  - end_timestamp          │
                    └─────────────────────────────┘
                                  │
        ┌─────────────────────────┼─────────────────────────┐
        ▼                         ▼                         ▼
┌───────────────┐         ┌───────────────┐         ┌───────────────┐
│  MatchEngine  │         │  MatchEngine  │         │  MatchEngine  │
│  (Market 0)   │         │  (Market 1)   │         │  (Market N)   │
│ ┌───────────┐ │         │ ┌───────────┐ │         │ ┌───────────┐ │
│ │ token_0   │ │         │ │ token_0   │ │         │ │ token_0   │ │
│ │ OrderBook │ │         │ │ OrderBook │ │         │ │ OrderBook │ │
│ └───────────┘ │         │ └───────────┘ │         │ └───────────┘ │
│ ┌───────────┐ │         │ ┌───────────┐ │         │ ┌───────────┐ │
│ │ token_1   │ │         │ │ token_1   │ │         │ │ token_1   │ │
│ │ OrderBook │ │         │ │ OrderBook │ │         │ │ OrderBook │ │
│ └───────────┘ │         │ └───────────┘ │         │ └───────────┘ │
└───────────────┘         └───────────────┘         └───────────────┘
```

#### 订单簿数据结构

```rust
pub struct OrderBook {
    pub symbol: PredictionSymbol,
    // 买单: 使用负数 key 实现降序（高价优先）
    pub bids: BTreeMap<i32, Vec<Order>>,
    // 卖单: 正数 key 升序（低价优先）
    pub asks: BTreeMap<i32, Vec<Order>>,
    // 订单 ID -> (方向, 价格) 快速查找
    pub order_price_map: HashMap<String, (OrderSide, i32)>,
}
```

**关键设计**:
- `BTreeMap` 保证价格有序
- 买单用负数 key (`-price`) 实现降序遍历
- 同价格订单按 `order_num` FIFO 排序

#### 交叉撮合逻辑

预测市场的核心特性：**买入 Token A 等价于卖出 Token B**

```rust
// engine.rs - get_cross_matching_orders()
match taker.side {
    OrderSide::Buy => {
        // 1. 匹配同结果的卖单 (price)
        for order in same_result_orderbook.asks {
            if taker_price >= order.price { ... }
        }
        // 2. 匹配对立结果的买单 (opposite_result_price)
        for order in opposite_result_orderbook.bids {
            if taker_price >= order.opposite_result_price { ... }
        }
    }
    OrderSide::Sell => { /* 类似逻辑 */ }
}
```

#### 输出流

撮合引擎产出 4 个 Redis Stream:

| Stream | 用途 |
|--------|------|
| `store_stream` | 订单状态变更 → Store 服务 |
| `processor_stream` | 交易结果 → Processor 服务 |
| `depth_stream` | 深度快照 → Depth 服务 |
| `websocket_stream` | 价格变化 → WebSocket 服务 |

### 3.2 api - REST API 服务

**文件**: `api/src/`

用户交互入口，主要处理：

| 端点 | Handler | 功能 |
|------|---------|------|
| `POST /order` | `handle_place_order` | 提交订单 |
| `DELETE /order` | `handle_cancel_order` | 取消订单 |
| `DELETE /orders` | `handle_cancel_all_orders` | 取消所有订单 |
| `GET /user` | `handle_user_data` | 用户信息 |
| `GET /events` | `handle_events` | 市场列表 |

**下单流程**:

```
1. 验证 JWT (Privy)
2. 获取用户 ID (cache)
3. 验证市场存在
4. 调用 Asset gRPC (CreateOrder) 冻结资金
5. 构造 SubmitOrderMessage
6. XADD 到 order_input_stream
```

### 3.3 store - 订单持久化服务

**文件**: `store/src/`

维护订单簿的内存镜像，定期快照到磁盘。

```rust
struct OrderStorageData {
    orders: HashMap<String, HashMap<String, Order>>,  // symbol -> order_id -> Order
    events: HashMap<i64, EngineMQEventCreate>,        // event_id -> 市场信息
    market_update_ids: HashMap<(i64, i16), u64>,      // (event_id, market_id) -> update_id
    last_message_id: Option<String>,                   // Redis Stream 位置
}
```

**快照策略**:
- 每 5 秒保存到 `./data/store/orders_snapshot.json`
- 调用 `sync_all()` 确保落盘
- 保存后裁剪 Redis Stream 旧消息

### 3.4 processor - 交易处理服务

**文件**: `processor/src/`

消费 `processor_stream`，处理撮合结果：

| 消息类型 | 处理逻辑 |
|----------|----------|
| `OrderRejected` | 调用 Asset RPC 解冻资金 |
| `OrderCancelled` | 调用 Asset RPC 解冻剩余资金 |
| `OrderSubmitted` | 推送用户 WebSocket 事件 |
| `OrderTraded` | 调用 Asset RPC 执行交易，推送链上请求 |

### 3.5 asset - 资产管理服务 (gRPC)

**文件**: `asset/src/`

gRPC 服务，管理用户资产：

| RPC 方法 | 功能 |
|----------|------|
| `CreateOrder` | 冻结 USDC/Token |
| `CancelOrder` | 解冻资产 |
| `OrderRejected` | 订单拒绝解冻 |
| `Trade` | 执行资产转移 |
| `Deposit` / `Withdraw` | 充值提现 |
| `Split` / `Merge` / `Redeem` | 头寸操作 |

### 3.6 websocket_depth - 实时深度服务

**文件**: `websocket_depth/src/`

WebSocket 服务，推送订单簿深度。

**连接协议**:
```json
// 1. 连接确认
{"event_type": "connected", "id": "<uuid>"}

// 2. 订阅
{"action": "subscribe", "event_id": 1, "market_id": 1}

// 3. 深度更新
{"event_type": "depth", "data": {...}}

// 4. 心跳 (每30秒)
"ping"
```

### 3.7 websocket_user - 用户事件服务

**文件**: `websocket_user/src/`

推送用户特定事件（订单变化、仓位变化）。

**连接协议**:
```json
// 1. 连接确认
{"event_type": "connected", "id": "<uuid>"}

// 2. JWT 认证
{"auth": "<privy-jwt-token>"}

// 3. 认证响应
{"event_type": "auth", "success": true}

// 4. 事件推送 (订单/仓位变化)
{"event_type": "open_order_change", ...}
```

---

## 四、数据流架构

```
                              ┌─────────────────┐
                              │   用户请求       │
                              └────────┬────────┘
                                       │
                              ┌────────▼────────┐
                              │    API 服务      │
                              │   (Port 5002)    │
                              └────────┬────────┘
                                       │
                    ┌──────────────────┼──────────────────┐
                    │                  │                  │
           ┌────────▼────────┐ ┌───────▼───────┐ ┌───────▼───────┐
           │ Asset gRPC      │ │order_input    │ │ event_input   │
           │ CreateOrder     │ │_stream (DB6)  │ │ _stream (DB6) │
           └─────────────────┘ └───────┬───────┘ └───────┬───────┘
                                       │                  │
                              ┌────────▼──────────────────▼────┐
                              │         Match Engine           │
                              │   (内存订单簿 + 交叉撮合)       │
                              └────────┬───────────────────────┘
                                       │
         ┌─────────────────────────────┼─────────────────────────────┐
         │                             │                             │
┌────────▼────────┐          ┌────────▼────────┐          ┌────────▼────────┐
│  store_stream   │          │processor_stream │          │ websocket_stream│
│     (DB7)       │          │     (DB7)       │          │     (DB3)       │
└────────┬────────┘          └────────┬────────┘          └────────┬────────┘
         │                            │                            │
┌────────▼────────┐          ┌────────▼────────┐          ┌────────▼────────┐
│  Store 服务     │          │ Processor 服务  │          │WebSocket Depth  │
│ (快照持久化)    │          │                 │          │    服务         │
└─────────────────┘          └────────┬────────┘          └────────┬────────┘
                                      │                            │
                    ┌─────────────────┼─────────────────┐          │
                    │                 │                 │          │
           ┌────────▼────────┐ ┌──────▼──────┐ ┌───────▼───────┐   │
           │ Asset gRPC      │ │onchain_msg  │ │user_event     │   │
           │ Trade/Cancel    │ │ _stream     │ │ _stream (DB3) │   │
           └─────────────────┘ └──────┬──────┘ └───────┬───────┘   │
                                      │                │           │
                              ┌───────▼───────┐ ┌──────▼──────┐    │
                              │ OnChain Msg   │ │WebSocket    │    │
                              │    服务       │ │User 服务    │    │
                              └───────────────┘ └─────────────┘    │
                                                                   │
                              ┌─────────────────────────────────────┘
                              │
                      ┌───────▼───────┐
                      │   前端客户端   │
                      └───────────────┘
```

### Redis 数据库分配

| DB | 用途 | Streams |
|:--:|------|---------|
| 0 | 通用消息 | event-stream, api_mq_stream, onchain_event_stream |
| 3 | WebSocket 消息 | websocket_stream, user_event_stream |
| 4 | 缓存 | 深度快照、市场价格 |
| 5 | 分布式锁 | 各种锁 |
| 6 | 引擎输入 | order_input_stream, event_input_stream |
| 7 | 引擎输出 | store_stream, processor_stream, depth_stream |

---

## 五、核心数据结构

### 5.1 订单 (Order)

```rust
pub struct Order {
    pub order_id: String,
    pub symbol: PredictionSymbol,      // event_id + market_id + token_id
    pub side: OrderSide,               // Buy | Sell
    pub order_type: OrderType,         // Limit | Market
    pub quantity: u64,                 // × 100 (2位小数)
    pub price: i32,                    // × 10000, 范围 [100, 9900]
    pub opposite_result_price: i32,    // 10000 - price
    pub status: OrderStatus,
    pub filled_quantity: u64,
    pub remaining_quantity: u64,
    pub timestamp: i64,                // 毫秒
    pub user_id: i64,
    pub order_num: u64,                // FIFO 排序序号
    pub privy_id: String,
    pub outcome_name: String,
}
```

### 5.2 预测符号 (PredictionSymbol)

```rust
pub struct PredictionSymbol {
    pub event_id: i64,      // 预测事件 ID
    pub market_id: i16,     // 市场/选项 ID
    pub token_id: String,   // 结果 Token ID
}
// 格式: "event_id-*******-market_id-*******-token_id"
```

### 5.3 处理器消息类型

```rust
pub enum ProcessorMessage {
    OrderRejected(OrderRejected),     // 订单被拒
    OrderSubmitted(OrderSubmitted),   // 挂单成功
    OrderTraded(OrderTraded),         // 成交
    OrderCancelled(OrderCancelled),   // 取消
}

pub struct OrderTraded {
    pub taker_symbol: PredictionSymbol,
    pub taker_id: i64,
    pub taker_order_id: String,
    pub taker_order_side: OrderSide,
    pub trades: Vec<Trade>,           // 可能匹配多个 maker
}
```

### 5.4 WebSocket 消息类型

```rust
// 深度变化
pub struct WebSocketPriceChanges {
    pub event_id: i64,
    pub market_id: i16,
    pub update_id: u64,              // 严格递增
    pub timestamp: i64,
    pub changes: HashMap<String, SingleTokenPriceInfo>,
}

// 用户事件
pub enum UserEvent {
    OpenOrderChange(OpenOrderChangeEvent),
    PositionChange(PositionChangeEvent),
}
```

---

## 六、数据库设计

### 6.1 核心表

| 表名 | 主键 | 用途 |
|------|------|------|
| `users` | id (BIGSERIAL) | 用户账户，Privy 认证 |
| `events` | id (BIGSERIAL) | 预测事件/市场 |
| `orders` | id (UUID) | 用户订单 |
| `trades` | (batch_id, order_id, match_timestamp) | 成交记录 |
| `positions` | (user_id, token_id) | 用户持仓 |
| `asset_history` | id (BIGSERIAL) | 资产变更审计 |
| `operation_history` | id (BIGSERIAL) | 操作历史 |

### 6.2 自定义类型

```sql
CREATE TYPE order_side AS ENUM ('buy', 'sell');
CREATE TYPE order_type AS ENUM ('limit', 'market');
CREATE TYPE order_status AS ENUM ('new', 'partially_filled', 'filled', 'cancelled', 'rejected');
CREATE TYPE asset_history_type AS ENUM (
    'create_order', 'order_rejected', 'cancel_order',
    'deposit', 'withdraw',
    'on_chain_buy_success', 'on_chain_sell_success',
    'on_chain_buy_failed', 'on_chain_sell_failed',
    'redeem', 'split', 'merge'
);
```

### 6.3 关键索引

```sql
-- 订单查询优化
CREATE INDEX idx_orders_user_id_status ON orders(user_id, status);
CREATE INDEX idx_orders_user_id_event_id_status ON orders(user_id, event_id, status);

-- 持仓查询优化
CREATE INDEX idx_positions_user_id_redeemed ON positions(user_id, redeemed)
    WHERE redeemed IS NULL OR redeemed = FALSE;

-- 交易查询优化
CREATE INDEX idx_trades_user_id_taker ON trades(user_id) WHERE taker = TRUE;
```

---

## 七、撮合算法详解

### 7.1 价格表示

| 概念 | 公式 | 示例 |
|------|------|------|
| 原始价格 | [0.01, 0.99] | 0.65 |
| 存储价格 | × 10000 | 6500 |
| 对立价格 | 10000 - price | 3500 |

### 7.2 匹配规则

预测市场的独特之处：**买 YES 等于卖 NO**

| Taker | Maker 来源 | 匹配条件 |
|-------|------------|----------|
| Buy Token A | Sell Token A (asks) | `taker.price >= maker.price` |
| Buy Token A | Buy Token B (bids) | `taker.price >= maker.opposite_result_price` |
| Sell Token A | Buy Token A (bids) | `taker.price <= maker.price` |
| Sell Token A | Sell Token B (asks) | `taker.price <= maker.opposite_result_price` |

### 7.3 匹配优先级

1. **价格优先**: 买单高价先匹配，卖单低价先匹配
2. **时间优先**: 同价格按 `order_num` FIFO

### 7.4 自成交防护

```rust
// engine.rs - get_cross_matching_orders()
if user_id == taker.user_id {
    has_self_trade = true;
    break;  // 停止匹配，取消剩余订单
}
```

---

## 八、可靠性设计

### 8.1 消息可靠性

```
┌─────────────────────────────────────────────────────────────────┐
│                    Redis Streams 消费模式                        │
├─────────────────────────────────────────────────────────────────┤
│ 1. Consumer Group + 多消费者                                     │
│ 2. 启动时 XAUTOCLAIM 认领 pending 消息                           │
│ 3. 处理成功后 XACKDEL 确认并删除                                  │
│ 4. 优雅关闭: 完成当前消息后再退出                                 │
└─────────────────────────────────────────────────────────────────┘
```

### 8.2 快照恢复

```rust
// store/src/storage.rs
pub async fn load_snapshot(&self) -> anyhow::Result<Option<String>> {
    // 1. 读取 ./data/store/orders_snapshot.json
    // 2. 恢复 orders, events, market_update_ids
    // 3. 返回 last_message_id (用于续消费)
}
```

### 8.3 优雅关闭

所有服务实现信号处理：

```rust
// graceful.rs
pub async fn shutdown_signal() {
    tokio::signal::ctrl_c().await.unwrap();
    // 发送 broadcast 通知所有任务
    // 等待任务完成
}
```

### 8.4 订单去重

```rust
// match_engine/src/helper.rs
pub struct SlidingWindowDedup {
    // 滑动窗口记录已处理的订单 ID
    // 防止 Redis 重试导致重复处理
}
```

---

## 九、配置系统

### 9.1 环境变量

```bash
# deploy/common.env
RUST_LOG=info
REDIS_URL=redis://127.0.0.1:6379/0
DATABASE_URL=postgresql://user:password@localhost:5432/prediction_market
RUN_MODE=dev  # 或 prod
```

### 9.2 服务配置

```toml
# deploy/match_engine/dev.toml
[logging]
level = "info"
file = "./logs/match_engine.log"
console = true
rotation_max_files = 3

[engine_input_mq]
order_input_consumer_count = 2
order_input_batch_size = 100

[engine]
engine_max_order_count = 10000
```

### 9.3 常量定义

```rust
// common/src/consts.rs
pub const PRICE_MULTIPLIER: i32 = 10000;
pub const QUANTITY_MULTIPLIER: i32 = 100;
pub const SYMBOL_SEPARATOR: &str = "-*******-";
pub const SNAPSHOT_INTERVAL_SECONDS: u64 = 5;

// Redis DB 分配
pub const REDIS_DB_ENGINE_INPUT_MQ: u32 = 6;
pub const REDIS_DB_ENGINE_OUTPUT_MQ: u32 = 7;
```

---

## 十、开发与运维

### 10.1 构建命令

```bash
# 构建
cargo build
cargo build --release

# 测试
cargo test
cargo test -p match_engine

# 代码质量
cargo fmt      # 使用 rustfmt.toml (tab 缩进, 200 字符宽度)
cargo clippy   # 每个函数最多 8 个参数
```

### 10.2 运行服务

```bash
export RUN_MODE=dev

# 核心服务
cargo run --bin match_engine
cargo run --bin api
cargo run --bin store
cargo run --bin processor
cargo run --bin asset

# WebSocket 服务
cargo run --bin websocket_depth
cargo run --bin websocket_user

# 辅助服务
cargo run --bin event
cargo run --bin depth
cargo run --bin onchain_msg
```

### 10.3 服务端口

| 服务 | 默认端口 | 协议 |
|------|----------|------|
| api | 5002 | HTTP REST |
| asset | 5000 | gRPC |
| websocket_depth | 5003 | WebSocket |
| websocket_user | 5004 | WebSocket |

---

## 十一、设计亮点

### 11.1 高性能撮合

- **纯内存订单簿**: 无磁盘 I/O
- **BTreeMap 有序结构**: O(log n) 价格查找
- **每市场独立 tokio task**: 并行处理
- **批量消息处理**: 减少 Redis 往返

### 11.2 交叉撮合创新

- 自动转换对立结果价格
- 统一的订单簿深度视图（交叉插入）
- 防自成交保护

### 11.3 可观测性

- **update_id 机制**: 保证 WebSocket 消息顺序
- **结构化日志**: JSON 格式 + tracing
- **资产审计**: asset_history 完整记录

### 11.4 故障恢复

- 快照 + 续消费模式
- Pending 消息自动认领
- 优雅关闭保证数据完整

---

## 十二、系统边界与限制

| 项目 | 限制 | 说明 |
|------|------|------|
| 价格精度 | 4 位小数 | × 10000 存储 |
| 数量精度 | 2 位小数 | × 100 存储 |
| 价格范围 | [0.01, 0.99] | 预测市场约束 |
| Stream 最大长度 | 10000 | 自动裁剪 |
| 快照间隔 | 5 秒 | 可配置 |
| WebSocket 心跳 | 30 秒 | 文本 "ping" |

---

## 十三、总结

PredictionMarket 是一个**设计精良的预测市场撮合系统**：

1. **微服务解耦**: 11 个服务各司其职，通过 Redis Streams 松耦合
2. **高性能撮合**: 内存订单簿 + 交叉撮合，毫秒级延迟
3. **可靠性保障**: Consumer Group + 快照恢复 + 优雅关闭
4. **实时推送**: WebSocket 秒级更新，update_id 保序
5. **金融精度**: Decimal 运算，资产冻结机制，完整审计

系统体现了 Rust 在高性能金融系统中的优势：
- 零成本抽象
- 内存安全
- 并发友好
- 高效的异步运行时

---

*文档生成: Claude Opus 4.5*
*时间: 2025-12-03*
