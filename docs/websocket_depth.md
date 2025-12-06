# 深度 WebSocket 接口文档

用于实时市场深度数据推送的 WebSocket 接口（公开接口，无需鉴权）。

## 连接地址

```
ws://host:port/depth
```

## 通信流程

1. 客户端连接 WebSocket
2. 服务端发送连接成功消息（包含连接ID）
3. 客户端发送订阅消息
4. 服务端返回订阅结果
5. 服务端推送深度快照
6. 服务端持续推送价格变化
7. 客户端定期发送心跳

---

## 客户端消息

### 订阅

```json
{
  "action": "subscribe",
  "event_id": 1,
  "market_id": 1
}
```

### 取消订阅

```json
{
  "action": "unsubscribe",
  "event_id": 1,
  "market_id": 1
}
```

### 心跳

客户端需定期发送文本消息 `ping`（纯文本，非 JSON）保持连接。

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

---

### 订阅响应

**订阅成功:**
```json
{
  "event_type": "subscribed",
  "success": true
}
```

**订阅失败:**
```json
{
  "event_type": "subscribe_error",
  "success": false,
  "message": "错误原因"
}
```

### 取消订阅响应

**取消成功:**
```json
{
  "event_type": "unsubscribed",
  "success": true
}
```

**取消失败:**
```json
{
  "event_type": "unsubscribe_error",
  "success": false,
  "message": "错误原因"
}
```

### 错误响应

```json
{
  "event_type": "error",
  "success": false,
  "message": "Invalid message format"
}
```

---

### 深度快照

订阅成功后立即推送完整的订单簿快照。

```json
{
  "event_id": 1,
  "market_id": 1,
  "update_id": 12345,
  "timestamp": 1700000000000,
  "depths": {
    "token_id_yes": {
      "latest_trade_price": "0.65",
      "bids": [
        {"price": "0.64", "total_quantity": "1000", "total_size": "640.00"},
        {"price": "0.63", "total_quantity": "2000", "total_size": "1260.00"}
      ],
      "asks": [
        {"price": "0.66", "total_quantity": "800", "total_size": "528.00"},
        {"price": "0.67", "total_quantity": "1200", "total_size": "804.00"}
      ]
    },
    "token_id_no": {
      "latest_trade_price": "0.35",
      "bids": [
        {"price": "0.34", "total_quantity": "900", "total_size": "306.00"}
      ],
      "asks": [
        {"price": "0.36", "total_quantity": "700", "total_size": "252.00"}
      ]
    }
  }
}
```

**字段说明:**

| 字段 | 类型 | 说明 |
|------|------|------|
| event_id | i64 | 事件 ID |
| market_id | i16 | 市场 ID |
| update_id | u64 | 序列号 |
| timestamp | i64 | Unix 时间戳（毫秒） |
| depths | HashMap | token_id -> 深度信息 的映射 |

**深度信息:**

| 字段 | 类型 | 说明 |
|------|------|------|
| latest_trade_price | string | 最新成交价 |
| bids | array | 买单列表，按价格降序排列 |
| asks | array | 卖单列表，按价格升序排列 |

**价格档位:**

| 字段 | 类型 | 说明 |
|------|------|------|
| price | string | 价格 |
| total_quantity | string | 该价格的总数量 |
| total_size | string | 该价格档位的总价值（price × total_quantity） |

---

### 价格变化（增量更新）

订单簿增量变化，当有订单提交、取消或成交时推送。

```json
{
  "event_id": 1,
  "market_id": 1,
  "update_id": 12346,
  "timestamp": 1700000001000,
  "changes": {
    "token_id_yes": {
      "latest_trade_price": "0.65",
      "bids": [
        {"price": "0.64", "total_quantity": "1100", "total_size": "704.00"}
      ],
      "asks": []
    }
  }
}
```

**说明:**
- 若 `total_quantity` 为 "0"，表示该价格档位已被移除（此时 `total_size` 也为 "0"）
- 空的 `bids` 或 `asks` 数组表示该方向无变化
- 仅包含发生变化的价格档位
- `total_size` 字段为价格乘以数量的结果，用于前端显示

---

## 会话示例

```
客户端: 连接 ws://host:port/depth

服务端: {"event_type": "connected", "id": "550e8400-e29b-41d4-a716-446655440000"}

客户端: {"action": "subscribe", "event_id": 1, "market_id": 1}

服务端: {"event_type": "subscribed", "success": true}

服务端: {
  "event_id": 1,
  "market_id": 1,
  "update_id": 100,
  "timestamp": 1700000000000,
  "depths": {
    "token_yes": {
      "latest_trade_price": "0.50",
      "bids": [{"price": "0.49", "total_quantity": "1000", "total_size": "490.00"}],
      "asks": [{"price": "0.51", "total_quantity": "800", "total_size": "408.00"}]
    },
    "token_no": {
      "latest_trade_price": "0.50",
      "bids": [{"price": "0.49", "total_quantity": "800", "total_size": "392.00"}],
      "asks": [{"price": "0.51", "total_quantity": "1000", "total_size": "510.00"}]
    }
  }
}

客户端: ping

服务端: {
  "event_id": 1,
  "market_id": 1,
  "update_id": 101,
  "timestamp": 1700000001000,
  "changes": {
    "token_yes": {
      "latest_trade_price": "0.50",
      "bids": [{"price": "0.50", "total_quantity": "500", "total_size": "250.00"}],
      "asks": []
    }
  }
}

客户端: {"action": "unsubscribe", "event_id": 1, "market_id": 1}

服务端: {"event_type": "unsubscribed", "success": true}
```

---

## 客户端处理示例

```javascript
const ws = new WebSocket('ws://host:port/depth');

let orderBook = {};
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

    // 订阅市场
    ws.send(JSON.stringify({
      action: 'subscribe',
      event_id: 1,
      market_id: 1
    }));

    // 启动心跳
    setInterval(() => ws.send('ping'), 30000);
    return;
  }

  // 处理其他服务端响应
  if (msg.event_type) {
    console.log('Response:', msg);
    return;
  }

  // 处理深度数据
  if (msg.depths) {
    // 全量快照
    orderBook = msg.depths;
  } else if (msg.changes) {
    // 增量更新
    applyChanges(msg.changes);
  }
};

function applyChanges(changes) {
  for (const [tokenId, tokenChanges] of Object.entries(changes)) {
    if (!orderBook[tokenId]) {
      orderBook[tokenId] = { bids: [], asks: [], latest_trade_price: "0" };
    }

    // 更新最新成交价
    if (tokenChanges.latest_trade_price) {
      orderBook[tokenId].latest_trade_price = tokenChanges.latest_trade_price;
    }

    // 更新买单
    for (const level of tokenChanges.bids) {
      updatePriceLevel(orderBook[tokenId].bids, level);
    }

    // 更新卖单
    for (const level of tokenChanges.asks) {
      updatePriceLevel(orderBook[tokenId].asks, level);
    }
  }
}

function updatePriceLevel(levels, newLevel) {
  const index = levels.findIndex(l => l.price === newLevel.price);

  if (newLevel.total_quantity === "0") {
    if (index !== -1) levels.splice(index, 1);
  } else {
    if (index !== -1) {
      levels[index] = newLevel;
    } else {
      levels.push(newLevel);
    }
  }
}
```

---

## 限流说明

- 无需鉴权
- 单连接最大订阅数有限制
- 消息会批量聚合后定期推送，非逐笔推送
- 心跳超时将断开连接
