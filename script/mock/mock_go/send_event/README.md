# Send Event Tool

## 功能说明

这个工具用于从 PostgreSQL 数据库中读取 `events` 表，并将事件消息发送到 Redis Stream (`event_input_stream`)，供 match_engine 消费。

### 两个核心功能

1. **sendEventCreate**: 发送未关闭的事件（`closed=false`）的创建消息（AddOneEvent）
2. **sendEventClose**: 发送指定事件的关闭消息（RemoveOneEvent），接受 event_id 作为参数

## 使用场景

- 重启 match_engine 后，需要重新加载所有未关闭的事件
- 测试环境初始化，需要批量加载事件到 match_engine
- 故障恢复时，需要重新同步事件状态
- 手动关闭指定的事件

## 工作流程

### sendEventCreate 函数

1. 查询 `events` 表中 `closed=false` 的所有事件
2. 对每个事件：
   - 解析 markets JSON 字段
   - 对每个 market 的 outcomes 和 token_ids 进行排序（Yes/No 时 Yes 在前，否则按字典序）
   - 构建 `EngineMQEventCreate` 消息
   - 构建 `EventInputMessage` 消息（types="AddOneEvent"）
   - 发送到 Redis Stream `event_input_stream`

### sendEventClose 函数

接受参数：`ctx context.Context, rdb *redis.Client, eventID int64`

1. 构建 `MQEventClose` 消息（包含指定的 event_id）
2. 构建 `EventInputMessage` 消息（types="RemoveOneEvent"）
3. 发送到 Redis Stream `event_input_stream`

### main 函数

1. 连接 PostgreSQL 数据库
2. 连接 Redis (DB 0)
3. 调用 sendEventCreate 发送所有未关闭事件

**注意**: sendEventClose 需要手动调用，传入具体的 event_id

## 配置

程序中的配置常量：

```go
// PostgreSQL
POSTGRES_HOST     = "127.0.0.1"
POSTGRES_PORT     = 5432
POSTGRES_USER     = "postgres"
POSTGRES_PASSWORD = "123456"
POSTGRES_DATABASE = "prediction_market"

// Redis
REDIS_HOST     = "127.0.0.1:8889"
REDIS_PASSWORD = "123456"
REDIS_DB       = 0  // engine_input_mq 使用 DB 0

// Redis Stream
EVENT_INPUT_STREAM  = "event_input_stream"
EVENT_INPUT_MSG_KEY = "msg"
```

如需修改，请直接编辑 `main.go` 中的常量。

## 编译和运行

### 1. 安装依赖

```bash
cd script/mock/mock_go/send_event
go mod download
```

### 2. 编译

```bash
go build -o send_event
```

### 3. 运行

#### 批量发送事件创建消息

```bash
./send_event
```

程序会自动发送所有 `closed=false` 的事件创建消息。

#### 手动关闭指定事件

如果需要关闭特定事件，可以修改 main 函数或创建独立脚本调用：

```go
// 在 main 函数中添加
if err := sendEventClose(ctx, rdb, 123); err != nil {
    log.Fatalf("Failed to close event: %v", err)
}
```

## 输出示例

### 批量发送事件创建

```
Connected to PostgreSQL
Connected to Redis

=== Sending Event Create Messages ===
Published AddOneEvent: event_id=1 (btc-100k)
Published AddOneEvent: event_id=2 (eth-5k)
✅ Successfully published 2 AddOneEvent messages to match_engine

✅ Event create messages sent successfully

💡 To close an event, call: sendEventClose(ctx, rdb, event_id)
```

### 手动关闭单个事件

```
✅ Published RemoveOneEvent: event_id=3
```

## 消息格式

### AddOneEvent 消息格式

Rust 端使用 `#[serde(tag = "types")]` 会将结构体字段展平到顶层：

```json
{
  "types": "AddOneEvent",
  "event_id": 1,
  "markets": {
    "1": {
      "market_id": 1,
      "outcomes": ["Yes", "No"],
      "token_ids": ["token_yes_1", "token_no_1"]
    }
  },
  "end_date": "2025-12-31T23:59:59Z"
}
```

### RemoveOneEvent 消息格式

同样展平结构体字段：

```json
{
  "types": "RemoveOneEvent",
  "event_id": 3
}
```

### 为什么是展平结构？

Rust 的 serde 使用 `#[serde(tag = "types")]` 时，对于 **newtype variant** (如 `RemoveOneEvent(MQEventClose)`)，会将内部结构体的字段展平到与 tag 同级，而不是嵌套。这是 serde 的 internally tagged enum 的标准行为。

## 注意事项

1. 运行前确保 PostgreSQL 和 Redis 服务已启动
2. 默认运行只会发送事件创建消息（AddOneEvent）
3. 如果 match_engine 正在运行，会自动消费这些消息
4. outcomes 和 token_ids 会自动排序（Yes/No 时 Yes 在前，否则按字典序）
5. sendEventCreate 自动查询并发送所有 `closed=false` 的事件
6. sendEventClose 需要手动调用，传入特定的 event_id 参数
7. sendEventClose 不查询数据库，直接发送指定 event_id 的关闭消息

## 故障排查

### 连接数据库失败

- 检查 PostgreSQL 是否运行：`pg_isready -h 127.0.0.1 -p 5432`
- 检查用户名密码是否正确
- 检查数据库名称是否正确

### 连接 Redis 失败

- 检查 Redis 是否运行：`redis-cli -h 127.0.0.1 -p 8889 ping`
- 检查 Redis 密码是否正确

### 没有事件发送

- 检查数据库中是否有未关闭的事件
- 查询未关闭事件：`SELECT id, event_identifier, closed FROM events WHERE closed = false;`

### 手动关闭事件失败

- 确保传入的 event_id 存在且有效
- 检查 Redis 连接是否正常
- 查看 match_engine 日志确认消息是否被正确消费
