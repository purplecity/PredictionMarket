# 用户 WebSocket 接口文档

用于实时推送用户私有数据的 WebSocket 接口（需要鉴权）。

## 连接地址

```
ws://host:port/user
```

## 通信流程

1. 客户端连接 WebSocket
2. 服务端发送连接成功消息（包含连接ID）
3. 客户端发送鉴权消息（必须是第一条客户端消息）
4. 服务端返回鉴权结果
5. 鉴权成功后，服务端推送用户事件
6. 客户端定期发送心跳

---

## 客户端消息

### 鉴权

连接后必须立即发送鉴权消息：

```json
{"auth": "privy_jwt_token"}
```

### 心跳

鉴权成功后，客户端需定期发送文本消息 `ping`（纯文本，非 JSON）保持连接。

---

## 服务端消息

### 连接成功消息

WebSocket 连接建立后，服务端立即发送连接成功消息：

```json
{
  "event_type": "connected",
  "id": "550e8400-e29b-41d4-a716-446655440000"
}
```

**字段说明:**

| 字段 | 类型 | 说明 |
|------|------|------|
| event_type | string | 固定值 "connected" |
| id | string | 连接的唯一标识符（UUID） |

收到此消息后，客户端应立即发送鉴权消息。

---

### 鉴权响应

**鉴权成功:**
```json
{"event_type": "auth", "success": true}
```

**鉴权失败:**
```json
{"event_type": "auth", "success": false}
```

鉴权失败后连接将被关闭。

---

## 事件消息格式

鉴权成功后，服务端推送的消息遵循以下格式：

```json
{
  "event_id": 1,
  "event_type": "open_order_change" | "position_change",
  "data": {}
}
```

---

## 事件类型

### 1. open_order_change（订单变更）

用户订单状态变更时推送，包括创建、更新、取消、完全成交。

#### 1.1 订单创建

```json
{
  "event_id": 1,
  "event_type": "open_order_change",
  "data": {
    "types": "open_order_created",
    "privy_id": "cm...",
    "event_id": 1,
    "market_id": 1,
    "order_id": "uuid",
    "side": "buy",
    "outcome_name": "Yes",
    "price": "0.65",
    "quantity": "100",
    "volume": "65",
    "filled_quantity": "0",
    "created_at": 1700000000
  }
}
```

#### 1.2 订单更新（部分成交）

```json
{
  "event_id": 1,
  "event_type": "open_order_change",
  "data": {
    "types": "open_order_updated",
    "privy_id": "cm...",
    "event_id": 1,
    "market_id": 1,
    "order_id": "uuid",
    "side": "buy",
    "outcome_name": "Yes",
    "price": "0.65",
    "quantity": "100",
    "volume": "65",
    "filled_quantity": "50",
    "created_at": 1700000000
  }
}
```

#### 1.3 订单取消

```json
{
  "event_id": 1,
  "event_type": "open_order_change",
  "data": {
    "types": "open_order_cancelled",
    "event_id": 1,
    "market_id": 1,
    "order_id": "uuid",
    "privy_id": "cm..."
  }
}
```

#### 1.4 订单完全成交

```json
{
  "event_id": 1,
  "event_type": "open_order_change",
  "data": {
    "types": "open_order_filled",
    "event_id": 1,
    "market_id": 1,
    "order_id": "uuid",
    "privy_id": "cm..."
  }
}
```

**UserOpenOrders 字段说明:**

| 字段 | 类型 | 说明 |
|------|------|------|
| privy_id | string | 用户 Privy ID |
| event_id | i64 | 事件 ID |
| market_id | i16 | 市场 ID |
| order_id | string | 订单 UUID |
| side | string | "buy" 或 "sell" |
| outcome_name | string | 结果名称（如 "Yes"、"No"） |
| price | string | 订单价格 |
| quantity | string | 订单总数量 |
| volume | string | 订单金额（price * quantity） |
| filled_quantity | string | 已成交数量 |
| created_at | i64 | 创建时间戳 |

---

### 2. position_change（持仓变更）

用户持仓变更时推送，包括创建、更新、移除。

#### 2.1 持仓创建

```json
{
  "event_id": 1,
  "event_type": "position_change",
  "data": {
    "types": "position_created",
    "privy_id": "cm...",
    "event_id": 1,
    "market_id": 1,
    "outcome_name": "Yes",
    "token_id": "token_id_yes",
    "avg_price": "0.65",
    "quantity": "100"
  }
}
```

#### 2.2 持仓更新

```json
{
  "event_id": 1,
  "event_type": "position_change",
  "data": {
    "types": "position_updated",
    "privy_id": "cm...",
    "event_id": 1,
    "market_id": 1,
    "outcome_name": "Yes",
    "token_id": "token_id_yes",
    "avg_price": "0.60",
    "quantity": "150"
  }
}
```

#### 2.3 持仓移除

```json
{
  "event_id": 1,
  "event_type": "position_change",
  "data": {
    "types": "position_removed",
    "event_id": 1,
    "market_id": 1,
    "token_id": "token_id_yes",
    "privy_id": "cm..."
  }
}
```

**UserPositions 字段说明:**

| 字段 | 类型 | 说明 |
|------|------|------|
| privy_id | string | 用户 Privy ID |
| event_id | i64 | 事件 ID |
| market_id | i16 | 市场 ID |
| outcome_name | string | 结果名称 |
| token_id | string | Token ID |
| avg_price | string | 持仓均价 |
| quantity | string | 持仓数量 |

---

## 客户端处理

### 连接管理

```javascript
const ws = new WebSocket('ws://host:port/user');

let connectionId = null;

ws.onopen = () => {
  console.log('WebSocket 连接已建立，等待连接确认...');
};

ws.onmessage = (event) => {
  const msg = JSON.parse(event.data);

  // 处理连接成功消息
  if (msg.event_type === 'connected') {
    connectionId = msg.id;
    console.log('连接成功，连接ID:', connectionId);

    // 收到连接成功消息后立即发送鉴权消息
    const token = getPrivyJwtToken();
    ws.send(JSON.stringify({ auth: token }));
    return;
  }

  // 处理鉴权响应
  if (msg.event_type === 'auth') {
    if (msg.success) {
      console.log('鉴权成功');
      // 启动心跳
      setInterval(() => ws.send('ping'), 30000);
    } else {
      console.log('鉴权失败');
    }
    return;
  }

  // 处理业务事件
  handleMessage(msg);
};

ws.onclose = () => {
  console.log('连接断开');
  // 实现重连逻辑
  setTimeout(reconnect, 1000);
};

ws.onerror = (error) => {
  console.error('WebSocket 错误:', error);
};
```

### 消息处理

```javascript
function handleMessage(msg) {
  switch (msg.event_type) {
    case 'open_order_change':
      handleOrderChange(msg.data);
      break;
    case 'position_change':
      handlePositionChange(msg.data);
      break;
  }
}

function handleOrderChange(data) {
  switch (data.types) {
    case 'open_order_created':
      // 添加新订单到本地状态
      addOrder(data);
      break;
    case 'open_order_updated':
      // 更新已有订单
      updateOrder(data);
      break;
    case 'open_order_cancelled':
    case 'open_order_filled':
      // 从未完成订单中移除
      removeOrder(data.order_id);
      break;
  }
}

function handlePositionChange(data) {
  switch (data.types) {
    case 'position_created':
      // 添加新持仓到本地状态
      addPosition(data);
      break;
    case 'position_updated':
      // 更新已有持仓
      updatePosition(data);
      break;
    case 'position_removed':
      // 从本地状态中移除持仓
      removePosition(data.token_id);
      break;
  }
}
```

---

## 会话示例

```
客户端: 连接 ws://host:port/user

服务端: {"event_type": "connected", "id": "550e8400-e29b-41d4-a716-446655440000"}

客户端: {"auth": "eyJ..."}

服务端: {"event_type": "auth", "success": true}

客户端: ping

// 用户下单
服务端: {
  "event_id": 1,
  "event_type": "open_order_change",
  "data": {
    "types": "open_order_created",
    "privy_id": "cm...",
    "event_id": 1,
    "market_id": 1,
    "order_id": "abc-123",
    "side": "buy",
    "outcome_name": "Yes",
    "price": "0.65",
    "quantity": "100",
    "volume": "65",
    "filled_quantity": "0",
    "created_at": 1700000000
  }
}

// 订单部分成交
服务端: {
  "event_id": 1,
  "event_type": "open_order_change",
  "data": {
    "types": "open_order_updated",
    "privy_id": "cm...",
    "event_id": 1,
    "market_id": 1,
    "order_id": "abc-123",
    "side": "buy",
    "outcome_name": "Yes",
    "price": "0.65",
    "quantity": "100",
    "volume": "65",
    "filled_quantity": "50",
    "created_at": 1700000000
  }
}

// 成交后创建持仓
服务端: {
  "event_id": 1,
  "event_type": "position_change",
  "data": {
    "types": "position_created",
    "privy_id": "cm...",
    "event_id": 1,
    "market_id": 1,
    "outcome_name": "Yes",
    "token_id": "token_yes",
    "avg_price": "0.65",
    "quantity": "50"
  }
}

// 订单完全成交
服务端: {
  "event_id": 1,
  "event_type": "open_order_change",
  "data": {
    "types": "open_order_filled",
    "event_id": 1,
    "market_id": 1,
    "order_id": "abc-123",
    "privy_id": "cm..."
  }
}

// 持仓更新
服务端: {
  "event_id": 1,
  "event_type": "position_change",
  "data": {
    "types": "position_updated",
    "privy_id": "cm...",
    "event_id": 1,
    "market_id": 1,
    "outcome_name": "Yes",
    "token_id": "token_yes",
    "avg_price": "0.65",
    "quantity": "100"
  }
}
```

---

## 鉴权说明

- JWT 令牌必须是有效的 Privy 令牌
- 连接后必须立即发送鉴权消息（第一条消息）
- 令牌无效或过期将拒绝连接
- 断开连接后，客户端应使用新令牌重新连接

## 错误处理

- 未发送鉴权消息：连接超时后断开
- 第一条消息不是鉴权消息：立即断开
- 令牌无效：返回失败响应后断开
- 鉴权后发送非 ping 消息：立即断开
- 心跳超时：断开连接

---

## 最佳实践

1. **重连机制**: 实现指数退避重连策略
2. **令牌刷新**: 在令牌过期前刷新并重新连接
3. **状态同步**: 重连后通过 REST API 获取当前状态以确保一致性
4. **心跳检测**: 定期发送 ping 消息（建议 30 秒间隔）
