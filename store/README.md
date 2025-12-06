# Store Service

Store 服务用于存储撮合引擎中的订单数据，提供高性能的内存存储和持久化快照功能。

## 功能

1. **消费订单变化事件**：从 Redis Stream (`store_stream`) 中消费订单变化事件
2. **内存存储**：将订单变化直接存储在进程内存中，提高性能
3. **消息 ID 管理**：跟踪已处理的最新消息 ID，确保消息处理的连续性
4. **定期快照**：定期将所有活跃订单和市场信息保存为 JSON 文件
5. **数据持久化**：保存的 JSON 文件可用于服务启动时恢复订单和市场信息
6. **自动清理**：定期清理 Redis stream 中已处理的消息，避免 stream 无限增长
7. **优雅停机**：支持优雅停机，确保数据完整性

## 配置

配置文件位于 `deploy/store/dev.toml` 或 `deploy/store/prod.toml`：

```toml
[logging]
level = "info"
file = "./logs/store.log"
console = true
rotation_max_files = 3

[engine_output_mq]
db = 0
batch_size = 10

[storage]
snapshot_interval_seconds = 5
```

**配置说明**：

- `engine_output_mq.db`: Redis 数据库编号
- `engine_output_mq.batch_size`: 每次从 Redis stream 读取的消息数量
- `storage.snapshot_interval_seconds`: 快照保存间隔（秒）

**注意**：存储路径已固定为 `./data/store/orders_snapshot.json`，无需在配置中指定。

## 运行

```bash
# 设置运行模式（dev 或 prod）
export RUN_MODE=dev

# 运行 store 服务
cargo run --bin store
```

## 数据格式

快照文件保存在 `./data/store/orders_snapshot.json`，格式如下：

```json
{
  "snapshots": [
    {
      "symbol": {
        "event_id": 1,
        "market_id": 1,
        "token_id": "token_123"
      },
      "orders": [
        {
          "id": "order_123",
          "symbol": {
            "event_id": 1,
            "market_id": 1,
            "token_id": "token_123"
          },
          "side": "Buy",
          "order_type": "Limit",
          "quantity": 100,
          "price": 5000,
          "status": "New",
          "filled_quantity": 0,
          "remaining_quantity": 100,
          "timestamp": 1704110400000,
          "user_id": "user_456",
          "order_num": 1
        }
      ],
      "timestamp": 1704110400000
    }
  ],
  "events": {
    "1": {
      "event_id": 1,
      "markets": {
        "1": {
          "id": 1,
          "outcomes": ["Yes", "No"],
          "token_ids": ["token_yes", "token_no"]
        }
      },
      "end_date": "2024-01-01T12:00:00Z"
    }
  },
  "timestamp": 1704110400000,
  "last_message_id": "1704110400000-0"
}
```

**字段说明**：

- `snapshots`: 按 symbol 组织的订单快照数组，只包含活跃订单（状态为 `New` 或 `PartiallyFilled`）
- `events`: 市场信息映射（event_id -> EngineMQEventCreate），包含市场 ID、选项信息和结束时间
- `timestamp`: 快照生成时间戳（毫秒）
- `last_message_id`: 已处理的最新消息 ID，用于启动时恢复消费位置

## 架构设计

### 核心数据结构

所有数据（orders、events、last_message_id）都在同一把 `RwLock` 下，保证原子性：

```rust
struct OrderStorageData {
    orders: HashMap<String, HashMap<String, Order>>,
    events: HashMap<i64, EngineMQEventCreate>,
    last_message_id: Option<String>,
}

pub struct OrderStorage {
    data: Arc<RwLock<OrderStorageData>>,
}
```

**设计优势**：

- **原子性保证**：消息 ID 的更新与 orders/events 的变更在同一把锁下，确保数据一致性
- **线程安全**：使用 `Arc<RwLock<...>>` 实现多线程安全访问
- **内存高效**：多个 `OrderStorage` 实例共享同一份数据，通过 `Arc` 引用计数管理

### 消息处理流程

1. **消费消息**：从 Redis stream 读取消息（支持批量读取，数量由 `batch_size` 配置）
2. **处理事件**：调用 `handle_order_change` 处理订单变化事件
3. **原子更新**：在同一把写锁下：
   - 更新 orders 或 events
   - 更新 `last_message_id`
4. **更新读取位置**：更新 `last_id` 用于下次读取

### 快照机制

1. **定期触发**：根据 `snapshot_interval_seconds` 配置定期保存快照
2. **数据收集**：在同一把读锁下读取所有数据（orders、events、last_message_id）
3. **释放锁**：读取完成后立即释放锁，避免阻塞其他操作
4. **保存文件**：将数据序列化为 JSON 并写入文件，调用 `sync_all()` 确保刷盘
5. **清理旧消息**：保存快照后，清理 Redis stream 中小于等于 `last_message_id` 的所有消息

### 启动流程

1. **初始化**：加载配置、初始化日志、初始化 Redis 连接池
2. **加载快照**：从快照文件加载数据
   - 恢复 `last_message_id`
   - 加载市场信息（events）
   - 加载订单（orders），过滤过期市场
3. **清理旧消息**：如果有 `last_message_id`，清理 Redis stream 中小于该 ID 的消息
4. **启动消费者**：从 `last_message_id` 之后开始消费消息
5. **启动定期保存任务**：后台任务定期保存快照

### 优雅停机

1. **接收信号**：监听 SIGINT 或 SIGTERM 信号
2. **发送 shutdown 信号**：通过 `broadcast::channel` 发送 shutdown 信号给消费者
3. **等待处理完成**：等待消费者处理完当前消息（最多等待 10 秒）
4. **等待任务完成**：等待所有后台任务完成（最多等待 5 秒）
5. **退出**：服务正常退出

## 与撮合引擎的集成

1. **订单变化推送**：撮合引擎在订单变化时推送 `OrderChangeEvent` 到 Redis Stream
2. **Store 消费**：Store 服务消费这些事件并更新内存中的订单和市场信息
3. **定期保存**：Store 定期将内存中的订单和市场信息保存为 JSON 文件
4. **启动恢复**：Store 服务启动时会自动从快照文件加载订单和市场信息到内存
5. **消息清理**：定期清理 Redis stream 中已处理的消息，避免 stream 无限增长

## 消息类型

Store 服务处理以下订单变化事件：

- **OrderCreated**: 订单创建（新订单加入订单簿）
- **OrderUpdated**: 订单更新（部分成交）
- **OrderFilled**: 订单完全成交（从存储中删除）
- **OrderCancelled**: 订单取消（从存储中删除）
- **EventAdded**: 市场添加（添加市场信息和选项）
- **EventRemoved**: 市场移除（移除市场并清理该市场的所有订单）

## 注意事项

- **日志控制**：除了添加市场和移除市场操作外，其他订单操作不打印日志，避免日志过多
- **只保存活跃订单**：Store 只保存活跃订单（状态为 `New` 或 `PartiallyFilled`）
- **过期市场过滤**：已完全成交或取消的订单不会保存在快照中；启动加载时会自动过滤已过期市场的订单
- **消息 ID 管理**：消息 ID 与 orders/events 的更新在同一把锁下，保证原子性
- **Redis stream 清理**：
  - 启动时：如果有快照中的 `last_message_id`，清理小于该 ID 的消息
  - 快照保存后：清理小于等于 `last_message_id` 的消息
- **文件刷盘**：保存快照时使用 `sync_all()` 确保数据已写入磁盘
- **锁优化**：快照保存时，读取完数据后立即释放锁，避免阻塞其他操作
- **确保目录权限**：确保 `./data/store` 目录有写入权限
- **Redis 连接**：需要正确配置 Redis 连接信息
- **消费者模式**：消费者使用阻塞式读取（BLOCK 0），会一直阻塞等待新消息
- **批量处理**：支持批量读取消息，数量由 `batch_size` 配置控制

## 性能优化

1. **内存存储**：所有数据存储在内存中，提供高性能访问
2. **批量读取**：支持批量读取消息，减少 Redis 交互次数
3. **锁优化**：快照保存时尽快释放锁，避免长时间持有锁
4. **异步清理**：Redis stream 清理操作在快照保存后执行，不阻塞快照保存
5. **原子操作**：消息 ID 和数据的更新在同一把锁下，保证一致性

## 故障恢复

- **服务重启**：服务重启时会从快照文件恢复数据，包括订单、市场信息和消息 ID
- **消息恢复**：从快照中的 `last_message_id` 之后开始消费，不会丢失消息
- **连接重试**：Redis 连接失败时会自动重试，确保服务可用性
- **优雅停机**：支持优雅停机，确保数据完整性
