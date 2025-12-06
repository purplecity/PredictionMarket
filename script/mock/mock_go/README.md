# Mock Go 客户端

这个目录包含3个 Go 程序，用于模拟和测试 PredictionMarket 系统的各个组件。

**目录位置**: `script/mock/mock_go/`

## 目录结构

```
mock_go/
├── trade_responder/       # 交易响应服务
│   └── main.go
├── websocket_depth/       # 深度数据 WebSocket 客户端
│   └── main.go
├── websocket_user/        # 用户数据 WebSocket 客户端
│   └── main.go
├── go.mod                 # Go 模块定义
├── Makefile              # 快速命令工具
├── README.md             # 本文档
└── run_all.sh            # 一键启动脚本
```

## 程序说明

### 1. trade_responder
**功能**: 模拟链上交易响应服务

- 监听 `TRADE_SEND_STREAM` (Redis Stream)
- 接收 `TradeOnchainSendRequest` 消息
- 自动生成成功的 `TradeOnchainSendResponse` 响应
- 发送响应到 `TRADE_RESPONSE_STREAM`

**配置**:
- Redis: `127.0.0.1:8889` (DB 3)
- 消费者组: `mock_trade_responder`
- Stream: `deepsense:onchain:service:send_request` → `deepsense:onchain:service:send_reponse`

**运行**:
```bash
cd script/mock/mock_go
make trade
# 或直接运行
cd trade_responder && go run main.go
```

**响应内容**:
- 保留所有请求字段（除 `match_info`）
- 添加 `tx_hash`: 随机生成的交易哈希
- 添加 `success`: true（模拟成功）

---

### 2. websocket_depth
**功能**: WebSocket Depth 客户端

- 连接到 WebSocket Depth 服务器 (`ws://127.0.0.1:8084/ws`)
- 订阅市场深度数据
- 实时接收并打印深度快照和价格变化

**订阅示例**:
```json
{
  "action": "subscribe",
  "event_id": 1,
  "market_id": 1
}
```

**运行**:
```bash
cd script/mock/mock_go
make depth
# 或直接运行
cd websocket_depth && go run main.go
```

**功能**:
- 自动订阅 `event_id=1, market_id=1`
- 美化输出接收到的 JSON 数据
- 支持 Ctrl+C 优雅退出（自动取消订阅）

---

### 3. websocket_user
**功能**: WebSocket User 客户端

- 连接到 WebSocket User 服务器 (`ws://127.0.0.1:8083/ws`)
- 订阅用户活动数据
- 实时接收并打印用户相关事件

**订阅示例**:
```json
{
  "action": "subscribe",
  "user_id": 1
}
```

**运行**:
```bash
cd script/mock/mock_go
make user
# 或直接运行
cd websocket_user && go run main.go
```

**功能**:
- 自动订阅 `user_id=1`
- 美化输出接收到的 JSON 数据
- 支持 Ctrl+C 优雅退出（自动取消订阅）

---

## 安装依赖

首次使用前需要安装 Go 依赖：

```bash
cd script/mock/mock_go
go mod download
```

或者使用 tidy：

```bash
cd script/mock/mock_go
go mod tidy
```

## 依赖包

- `github.com/gorilla/websocket` - WebSocket 客户端库
- `github.com/redis/go-redis/v9` - Redis 客户端库

## 测试流程

### 完整测试流程：

1. **启动 trade_responder**:
   ```bash
   go run trade_responder.go
   ```

2. **启动 WebSocket 客户端**:
   ```bash
   # 终端1
   go run websocket_depth_client.go

   # 终端2
   go run websocket_user_client.go
   ```

3. **创建订单** (通过 API 或 mock 脚本):
   - 订单会被撮合
   - 生成交易请求到 `TRADE_SEND_STREAM`
   - `trade_responder` 自动响应成功
   - WebSocket 客户端会收到实时更新

4. **观察日志**:
   - `trade_responder`: 显示接收请求和发送响应
   - `websocket_depth_client`: 显示深度快照和价格变化
   - `websocket_user_client`: 显示用户订单和交易事件

## 配置说明

### Redis 配置
所有程序使用相同的 Redis 配置（与 `deploy/common.env` 一致）：
- Host: `127.0.0.1:8889`
- Password: `123456`
- DB: 根据用途不同（trade_responder 使用 DB 3）

### WebSocket 配置
- Depth Server: `ws://127.0.0.1:8084/ws`
- User Server: `ws://127.0.0.1:8083/ws`

## 自定义配置

你可以修改代码中的常量来适配不同的配置：

### trade_responder.go
```go
const (
    RedisAddr     = "127.0.0.1:8889"
    RedisPassword = "123456"
    RedisDB       = 3
)
```

### websocket_depth_client.go
```go
const (
    WSHost = "127.0.0.1:8084"
)

// 修改订阅参数
subscribe := DepthSubscribeMessage{
    Action:   "subscribe",
    EventID:  1,  // 修改这里
    MarketID: 1,  // 修改这里
}
```

### websocket_user_client.go
```go
const (
    WSHost = "127.0.0.1:8083"
)

// 修改订阅参数
subscribe := UserSubscribeMessage{
    Action: "subscribe",
    UserID: 1,  // 修改这里
}
```

## 故障排查

### 连接失败
- 检查 Redis 是否运行在 `127.0.0.1:8889`
- 检查密码是否正确
- 检查 WebSocket 服务是否已启动

### 无数据接收
- 确认已经有订单交易发生
- 检查订阅的 event_id/market_id/user_id 是否正确
- 查看服务端日志

### 编译错误
```bash
# 重新下载依赖
go mod download

# 清理缓存
go clean -modcache
go mod tidy
```

## 日志示例

### trade_responder 日志
```
✅ Connected to Redis
🚀 Trade Responder started, listening on stream: deepsense:onchain:service:send_request
📨 Received trade request: trade_id=xxx, event_id=1, market_id=1
✅ Sent trade response: trade_id=xxx, tx_hash=0x1234..., success=true
```

### websocket_depth_client 日志
```
✅ Connected to WebSocket Depth Server
📨 Subscribed to depth: event_id=1, market_id=1
📊 Received depth data:
{
  "event_id": 1,
  "market_id": 1,
  "update_id": 12345,
  "timestamp": 1700000000000,
  "depths": { ... }
}
```

### websocket_user_client 日志
```
✅ Connected to WebSocket User Server
📨 Subscribed to user: user_id=1
👤 Received user data:
{
  "type": "order_filled",
  "order_id": "xxx",
  "user_id": 1,
  ...
}
```

## 注意事项

1. **trade_responder** 必须在有交易发生前启动，否则可能错过消息
2. WebSocket 客户端会自动重连（需要自行实现）
3. 所有时间戳使用毫秒（Unix timestamp）
4. 修改订阅参数后需要重新编译运行
