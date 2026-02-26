# REST API 接口文档

基础路径: `/api`

## 响应格式

所有 API 响应遵循统一格式:

```json
{
  "code": 0,
  "msg": "success",
  "data": {}
}
```

- `code`: 0 表示成功，非 0 表示错误
- `msg`: 结果描述信息
- `data`: 响应数据（错误时为 null）

## 鉴权说明

需要鉴权的接口支持两种认证方式：

### 方式一：Privy JWT Token（推荐）

在请求头中包含 `Authorization` 字段：

```
Authorization: Bearer <privy_jwt_token>
```

### 方式二：API Key

在请求头中包含 `x-api-key` 字段：

```
x-api-key: <api_key>
```

**认证优先级：** 如果同时提供了 JWT 和 API Key，系统优先验证 JWT。

**API Key 说明：**
- **前端应用请使用 Privy JWT Token 认证**
- API Key 仅用于特殊场景：自动化脚本、做市商程序、第三方集成等
- API Key 与用户账户关联
- API Key 用户对延迟敏感，部分接口（如 `/positions`）会跳过动态计算以提升性能
- `/user_data` 接口不支持 API Key 认证（仅用于新用户注册场景）

---

## 公开接口

### GET /topics

获取可用的主题列表。

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": ["crypto", "sports", "politics"]
}
```

---

### GET /events

获取事件列表（分页）。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| ending_soon | bool | 否 | 按结束时间升序排序 |
| newest | bool | 否 | 按创建时间降序排序 |
| topic | string | 否 | 按主题筛选 |
| title | string | 否 | 按标题模糊匹配（大小写敏感），例如 `title=BTC` 将匹配所有包含 "BTC" 的事件 |
| volume | bool | 否 | 按交易量降序排序 |
| page | i16 | 是 | 页码（从 1 开始） |
| page_size | i16 | 是 | 每页数量 |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "events": [
      {
        "event_id": 1,
        "slug": "btc-100k",
        "image": "https://...",
        "title": "BTC 能否突破 10 万?",
        "volume": "1000000",
        "topic": "crypto",
        "has_streamer": true,
        "markets": [
          {
            "market_id": 1,
            "title": "Yes/No",
            "question": "Will BTC reach 100K?",
            "outcome_0_name": "Yes",
            "outcome_1_name": "No",
            "outcome_0_token_id": "token_id_yes",
            "outcome_1_token_id": "token_id_no",
            "outcome_0_chance": "0.65",
            "outcome_1_chance": "0.35",
            "outcome_0_best_bid": "0.64",
            "outcome_0_best_ask": "0.66",
            "outcome_1_best_bid": "0.34",
            "outcome_1_best_ask": "0.36"
          }
        ]
      }
    ],
    "total": 100,
    "has_more": true
  }
}
```

**字段说明:**
- `has_streamer`: 该事件是否有正在直播的主播（status = 'LIVE'）
- `outcome_0_best_bid` / `outcome_0_best_ask`: outcome_0 的买一价和卖一价
- `outcome_1_best_bid` / `outcome_1_best_ask`: outcome_1 的买一价和卖一价
- `total`: 符合条件的总记录数
- `has_more`: 是否还有更多数据（`page * page_size >= total` 时为 `false`）

---

### GET /event_detail

获取事件详情。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| event_id | i64 | 是 | 事件 ID |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "event_id": 1,
    "slug": "btc-100k",
    "image": "https://...",
    "title": "BTC 能否突破 10 万?",
    "rules": "事件规则...",
    "volume": "1000000",
    "starttime": 1735603200,
    "endtime": 1735689600,
    "closed": false,
    "resolved": false,
    "markets": [
      {
        "title": "Yes/No",
        "question": "Will BTC reach 100K?",
        "image": "https://...",
        "market_id": 1,
        "volume": "500000",
        "condition_id": "0x...",
        "parent_collection_id": "0x...",
        "outcome_0_name": "Yes",
        "outcome_1_name": "No",
        "outcome_0_token_id": "token_id_yes",
        "outcome_1_token_id": "token_id_no",
        "outcome_0_chance": "0.65",
        "outcome_1_chance": "0.35",
        "outcome_0_best_bid": "0.64",
        "outcome_0_best_ask": "0.66",
        "outcome_1_best_bid": "0.34",
        "outcome_1_best_ask": "0.36",
        "closed": false,
        "winner_outcome_name": "",
        "winner_outcome_token_id": ""
      }
    ]
  }
}
```

**字段说明:**
- `volume` (Event 级别): 事件总交易量（所有市场的交易量之和）
- `closed` (Event 级别): 整个事件是否已关闭
- `resolved` (Event 级别): 整个事件是否已结算
- `markets[].volume` (Market 级别): **单个市场的交易量** - 该市场内所有交易的累计金额
- `markets[].closed` (Market 级别): **单个市场是否已关闭** - 每个市场可以独立关闭，无需等待整个事件关闭
- `markets[].winner_outcome_name`: 获胜结果名称（市场关闭后填充）
- `markets[].winner_outcome_token_id`: 获胜 token ID（市场关闭后填充）

---

### GET /depth

获取市场深度数据（公开接口，无需鉴权）。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| event_id | i64 | 是 | 事件 ID |
| market_id | i16 | 是 | 市场 ID |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "update_id": 12345,
    "timestamp": 1700000000000,
    "depths": {
      "token_id_yes": {
        "latest_trade_price": "0.65",
        "bids": [
          {"price": "0.65", "total_quantity": "1000", "total_size": "650.00"}
        ],
        "asks": [
          {"price": "0.66", "total_quantity": "500", "total_size": "330.00"}
        ]
      },
      "token_id_no": {
        "latest_trade_price": "0.35",
        "bids": [
          {"price": "0.34", "total_quantity": "800", "total_size": "272.00"}
        ],
        "asks": [
          {"price": "0.35", "total_quantity": "600", "total_size": "210.00"}
        ]
      }
    }
  }
}
```

**字段说明:**
- `latest_trade_price`: 最新成交价
- `bids` / `asks`: 价格档位数组，每个档位包含：
  - `price`: 价格
  - `total_quantity`: 该价格的总数量
  - `total_size`: 该价格档位的总价值（price × total_quantity）

---

### GET /lives

获取直播事件列表（以 streamer 为单位）。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| ending_soon | bool | 否 | 按事件结束时间升序排序 |
| newest | bool | 否 | 按事件创建时间降序排序 |
| topic | string | 否 | 按主题筛选 |
| volume | bool | 否 | 按交易量降序排序（默认） |
| page | i16 | 是 | 页码（从 1 开始） |
| page_size | i16 | 是 | 每页数量 |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "streamers": [
      {
        "streamer_uid": "privy_id_xxx",
        "live_room": {
          "id": "room_uuid",
          "room_id": "privy_id_xxx",
          "room_name": "直播间名称",
          "room_desc": "直播间描述",
          "status": "LIVE",
          "event_id": "1",
          "create_date": 1700000000000,
          "picture_url": "https://...",
          "created_at": 1700000000000,
          "updated_at": 1700000000000
        },
        "user": {
          "privy_id": "privy_id_xxx",
          "name": "主播名称",
          "profile_image": "https://..."
        },
        "event": {
          "event_id": 1,
          "slug": "btc-100k",
          "image": "https://...",
          "title": "BTC 能否突破 10 万?",
          "volume": "1000000",
          "topic": "crypto",
          "has_streamer": true,
          "markets": [...]
        }
      }
    ],
    "total": 10,
    "has_more": true
  }
}
```

**字段说明:**
- `streamer_uid`: 主播 ID（即 privy_id）
- `live_room`: 直播间信息（来自 live_rooms 表）
  - `status`: 直播状态（"LIVE" 表示正在直播）
- `user`: 主播用户信息（可能为 null）
- `event`: 关联的事件信息（包含完整的市场数据）
- `total`: 符合条件的总记录数
- `has_more`: 是否还有更多数据

---

### GET /event_streamers

获取指定事件对应的正在直播的主播列表，包含直播间人数。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| event_id | i64 | 是 | 事件 ID |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "streamers": [
      {
        "streamer_uid": "privy_id_xxx",
        "viewer_count": 1234,
        "live_room": {
          "id": "room_uuid",
          "room_id": "privy_id_xxx",
          "room_name": "直播间名称",
          "room_desc": "直播间描述",
          "status": "LIVE",
          "event_id": "1",
          "create_date": 1700000000000,
          "picture_url": "https://...",
          "created_at": 1700000000000,
          "updated_at": 1700000000000
        },
        "user": {
          "privy_id": "privy_id_xxx",
          "name": "主播名称",
          "profile_image": "https://..."
        }
      }
    ]
  }
}
```

**字段说明:**
- `streamers`: 主播列表（按 viewer_count 从高到低排序）
- `streamer_uid`: 主播 ID（即 privy_id，也是 room_id）
- `viewer_count`: 直播间当前观看人数
- `live_room`: 直播间信息
- `user`: 主播用户信息（可能为 null）

**注意:** 只返回 status = 'LIVE' 的主播。

---

### GET /streamer_detail

获取主播详情（包含用户信息、直播间信息和关联事件信息）。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| streamer_uid | string | 是 | 主播 ID（privy_id） |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "streamer_uid": "privy_id_xxx",
    "live_room": {
      "id": "room_uuid",
      "room_id": "privy_id_xxx",
      "room_name": "直播间名称",
      "room_desc": "直播间描述",
      "status": "LIVE",
      "event_id": "1",
      "create_date": 1700000000000,
      "picture_url": "https://...",
      "created_at": 1700000000000,
      "updated_at": 1700000000000
    },
    "user": {
      "privy_id": "privy_id_xxx",
      "name": "主播名称",
      "profile_image": "https://..."
    },
    "event": {
      "event_id": 1,
      "slug": "btc-100k",
      "image": "https://...",
      "title": "BTC 能否突破 10 万?",
      "volume": "1000000",
      "topic": "crypto",
      "has_streamer": true,
      "markets": [
        {
          "market_id": 1,
          "title": "Yes/No",
          "question": "Will BTC reach 100K?",
          "outcome_0_name": "Yes",
          "outcome_1_name": "No",
          "outcome_0_token_id": "token_id_yes",
          "outcome_1_token_id": "token_id_no",
          "outcome_0_chance": "0.65",
          "outcome_1_chance": "0.35",
          "outcome_0_best_bid": "0.64",
          "outcome_0_best_ask": "0.66",
          "outcome_1_best_bid": "0.34",
          "outcome_1_best_ask": "0.36"
        }
      ]
    }
  }
}
```

**字段说明:**
- `streamer_uid`: 主播 ID
- `live_room`: 直播间信息（可能为 null，如果该主播没有直播间记录）
- `user`: 主播用户信息（可能为 null）
- `event`: 关联的事件信息（可能为 null，如果直播间没有关联事件或事件不存在）

---

### GET /streamer_viewer_count

获取主播直播间在线观看人数。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| streamer_id | string | 是 | 主播 ID（privy_id / room_id） |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "viewer_count": 1234
  }
}
```

**字段说明:**
- `viewer_count`: 当前直播间在线观看人数

---

### GET /simple_info

获取用户简单信息（头像、用户名）。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| uid | i64 | 是 | 用户 ID |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "name": "用户名",
    "profile_image": "https://..."
  }
}
```

**字段说明:**
- `name`: 用户名称
- `profile_image`: 用户头像 URL

---

### GET /points

获取用户积分信息。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| uid | i64 | 是 | 用户 ID |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "boost_points": 1000,
    "invite_points": 500,
    "total_points": 1500,
    "rank": "15"
  }
}
```

**字段说明:**
- `boost_points`: boost 后的积分
- `invite_points`: 邀请获得的积分
- `total_points`: 总积分（boost_points + invite_points）
- `rank`: 排名，前 200 名显示具体名次（如 "15"），超过 200 名显示 ">200"

---

### GET /balance

获取用户 USDC 余额。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| uid | i64 | 是 | 用户 ID |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "balance": "10000.50"
  }
}
```

**字段说明:**
- `balance`: 用户 USDC 可用余额

---

### GET /portfolio_value

获取用户投资组合价值。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| uid | i64 | 是 | 用户 ID |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "value": "10000.50"
  }
}
```

**字段说明:**
- `value`: 投资组合总价值（基于当前市场价格计算的持仓价值）

---

### GET /traded_volume

获取用户总交易量。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| uid | i64 | 是 | 用户 ID |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "volume": "50000.00"
  }
}
```

**字段说明:**
- `volume`: 用户总交易量（作为 taker 和 maker 的交易量之和）

---

### GET /positions

获取用户持仓列表。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| uid | i64 | 是 | 用户 ID |
| event_id | i64 | 否 | 按事件 ID 筛选 |
| market_id | i16 | 否 | 按市场 ID 筛选 |
| question | string | 否 | 按问题文本筛选（模糊匹配，大小写敏感） |
| value | bool | 否 | 按持仓价值排序（true=降序，false=升序） |
| quantity | bool | 否 | 按数量排序 |
| avg_price | bool | 否 | 按均价排序 |
| profit_value | bool | 否 | 按盈亏金额排序 |
| profit_percentage | bool | 否 | 按盈亏百分比排序 |
| page | i16 | 是 | 页码 |

**特别说明:**
> ⚠️ **重要提示:**
> 1. 每页固定 1000 条，仅支持单一排序参数，默认按 profit_value 降序
> 2. **当数据超过 1000 条时，将不支持盈亏相关的动态排序**（profit_value、profit_percentage、value），仅支持 avg_price 排序
> 3. **请勿依赖此接口实时更新盈亏数据**，因为后端计算较重。建议客户端接入 WebSocket 深度推送后，根据盘口价格自行计算盈亏
> 4. `question` 参数为模糊匹配（大小写敏感），例如 `question=BTC` 将匹配所有包含 "BTC" 的问题

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "positions": [
      {
        "event_id": 1,
        "market_id": 1,
        "event_title": "BTC 能否突破 10 万?",
        "market_title": "Yes/No",
        "market_question": "Will BTC reach 100K?",
        "event_image": "https://...",
        "market_image": "https://...",
        "outcome_name": "Yes",
        "condition_id": "0x...",
        "token_id": "token_id_yes",
        "avg_price": "0.50",
        "quantity": "100",
        "value": "65.00",
        "profit_value": "15.00",
        "profit_percentage": "30.00",
        "current_price": "0.65"
      }
    ],
    "total": 10,
    "has_more": false
  }
}
```

**字段说明:**
- `condition_id`: 市场条件 ID（同一市场的两个 token 共享相同的 condition_id）
- `total`: 符合条件的总记录数
- `has_more`: 是否还有更多数据（每页固定 1000 条，`page * 1000 >= total` 时为 `false`）
- `current_price`: 当前市场价格
  - 若事件已关闭：赢家为 "1"，输家为 "0"
  - 若事件未关闭：从价格缓存获取（基于买一卖一和最新成交价计算），无数据时为空字符串
  - 若数据量超过 1000 条：返回空字符串（性能优化）

---

### GET /closed_positions

获取用户已平仓（已赎回）持仓列表。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| uid | i64 | 是 | 用户 ID |
| event_id | i64 | 否 | 按事件 ID 筛选 |
| market_id | i16 | 否 | 按市场 ID 筛选 |
| question | string | 否 | 按问题文本筛选（模糊匹配，大小写敏感） |
| avg_price | bool | 否 | 按均价排序 |
| profit_value | bool | 否 | 按盈亏金额排序 |
| profit_percentage | bool | 否 | 按盈亏百分比排序 |
| redeem_timestamp | bool | 否 | 按赎回时间排序 |
| page | i16 | 是 | 页码 |

**特别说明:**
> ⚠️ **重要提示:**
> 1. 每页固定 1000 条，默认按 redeem_timestamp 降序。**当数据超过 1000 条时，将不支持盈亏相关的动态排序**（profit_value、profit_percentage），仅支持 avg_price 和 redeem_timestamp 排序
> 2. `question` 参数为模糊匹配（大小写敏感），例如 `question=BTC` 将匹配所有包含 "BTC" 的问题

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "positions": [
      {
        "event_id": 1,
        "market_id": 1,
        "event_title": "BTC 能否突破 10 万?",
        "market_title": "Yes/No",
        "market_question": "Will BTC reach 100K?",
        "event_image": "https://...",
        "market_image": "https://...",
        "outcome_name": "Yes",
        "token_id": "token_id_yes",
        "avg_price": "0.50",
        "quantity": "100",
        "value": "50.00",
        "payout": "100.00",
        "profit_value": "50.00",
        "profit_percentage": "100.00",
        "redeem_timestamp": 1700000000,
        "win": true
      }
    ],
    "total": 5,
    "has_more": false
  }
}
```

**字段说明:**
- `total`: 符合条件的总记录数
- `has_more`: 是否还有更多数据（每页固定 1000 条，`page * 1000 >= total` 时为 `false`）

---

### GET /activity

获取用户活动历史（拆分/合并/赎回/买入/卖出）。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| uid | i64 | 是 | 用户 ID |
| event_id | i64 | 否 | 按事件 ID 筛选 |
| market_id | i16 | 否 | 按市场 ID 筛选 |
| page | i16 | 是 | 页码 |
| page_size | i16 | 是 | 每页数量（1-100） |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "activities": [
      {
        "event_id": 1,
        "market_id": 1,
        "event_title": "BTC 能否突破 10 万?",
        "market_title": "Yes/No",
        "market_question": "Will BTC reach 100K?",
        "event_image": "https://...",
        "market_image": "https://...",
        "types": "buy",
        "timestamp": 1700000000,
        "outcome_name": "Yes",
        "price": "0.65",
        "quantity": "100",
        "value": "65.00",
        "tx_hash": "0x..."
      }
    ],
    "total": 20,
    "has_more": true
  }
}
```

**字段说明:**
- `total`: 符合条件的总记录数
- `has_more`: 是否还有更多数据（`page * page_size >= total` 时为 `false`）
- `value`: 交易价值（USDC）。对于 buy/sell 等于 `price * quantity`；对于 split/merge/redeem 等于 USDC 数量

**活动类型:**
- `split`: 拆分 USDC 为代币
- `merge`: 合并代币为 USDC
- `redeem`: 赎回获胜代币
- `buy`: 买入代币
- `sell`: 卖出代币

---

### GET /open_orders

获取用户未完成订单（新建或部分成交）。

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| uid | i64 | 是 | 用户 ID |
| event_id | i64 | 否 | 按事件 ID 筛选 |
| market_id | i16 | 否 | 按市场 ID 筛选 |
| page | i16 | 是 | 页码 |
| page_size | i16 | 是 | 每页数量（1-100） |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "orders": [
      {
        "event_id": 1,
        "market_id": 1,
        "event_title": "BTC 能否突破 10 万?",
        "market_title": "Yes/No",
        "market_question": "Will BTC reach 100K?",
        "event_image": "https://...",
        "market_image": "https://...",
        "order_id": "uuid",
        "side": "buy",
        "outcome_name": "Yes",
        "price": "0.65",
        "quantity": "100",
        "filled_quantity": "50",
        "volume": "65",
        "created_at": 1700000000
      }
    ],
    "total": 3,
    "has_more": false
  }
}
```

**字段说明:**
- `total`: 符合条件的总记录数
- `has_more`: 是否还有更多数据（`page * page_size >= total` 时为 `false`）

---

## 需要鉴权的接口

### GET /user_data

获取当前用户信息。若用户不存在则自动创建。

**请求头:** `Authorization: Bearer <privy_jwt>`

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| invite_code | string | 否 | 邀请码（仅新用户注册时有效） |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "id": 1,
    "privy_id": "cm...",
    "privy_evm_address": "0x...",
    "privy_email": "user@example.com",
    "privy_x": "@username",
    "privy_x_image": "https://...",
    "name": "用户名",
    "bio": "个人简介",
    "profile_image": "https://...",
    "invite_code": "cm..."
  }
}
```

**邀请码说明:**
- 邀请码即用户的 `privy_id`（天然唯一，响应中 `invite_code` 与 `privy_id` 值相同）
- 新用户注册时可传入他人的邀请码，若邀请码有效：
  - 被邀请人获得 100 积分
  - 邀请人获得 50 积分
- 若邀请码无效或未提供，用户正常注册，积分为 0

---

### POST /user_image

更新用户头像。

**请求头:** `Authorization: Bearer <privy_jwt>`

**请求体:**
```json
{
  "image_url": "https://storage.googleapis.com/...",
  "image_type": "header"
}
```

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": null
}
```

---

### POST /user_profile

更新用户资料。

**请求头:** `Authorization: Bearer <privy_jwt>`

**请求体:**
```json
{
  "name": "新用户名",
  "bio": "新简介"
}
```

**参数校验:**
- `name`: 1-32 个字符
- `bio`: 1-128 个字符

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": null
}
```

---

### GET /image_sign

获取图片上传签名 URL。

**请求头:** `Authorization: Bearer <privy_jwt>`

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| bucket_type | string | 是 | 存储桶类型（目前仅支持 "header"） |
| image_type | string | 是 | 图片格式（jpg, jpeg, png, gif, webp） |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "signed_url": "https://storage.googleapis.com/...",
    "public_url": "https://storage.googleapis.com/..."
  }
}
```

---

### POST /place_order

提交订单。

**请求头:** `Authorization: Bearer <privy_jwt>`

**请求体:**
```json
{
  "expiration": "1735689600",
  "feeRateBps": "100",
  "maker": "0x...",
  "makerAmount": "1000000000000000000",
  "nonce": "1",
  "salt": 12345,
  "side": "buy",
  "signature": "0x...",
  "signatureType": 2,
  "signer": "0x...",
  "taker": "0x0000000000000000000000000000000000000000",
  "takerAmount": "1500000000000000000",
  "tokenId": "token_id_yes",
  "event_id": 1,
  "market_id": 1,
  "price": "0.65",
  "order_type": "limit"
}
```

**参数校验:**
- `side`: "buy" 或 "sell"
- `price`: 0.0010 - 0.9990，最多 4 位小数
- `makerAmount` / `takerAmount`: 必须能被 10^16 整除
- `order_type`: "limit"（限价单）或 "market"（市价单）

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": "order_uuid"
}
```

**错误码:**
- `2005`: 事件不存在或已关闭
- `2006`: 市场不存在或已关闭
- `2007`: Token ID 不存在
- `2008`: 签名验证失败
- `2010`: 事件已过期

---

### POST /cancel_order

取消指定订单。

**请求头:** `Authorization: Bearer <privy_jwt>`

**请求体:**
```json
{
  "order_id": "order_uuid"
}
```

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": null
}
```

---

### POST /cancel_all_orders

取消当前用户的所有未完成订单。

**请求头:** `Authorization: Bearer <privy_jwt>`

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": null
}
```

---

### GET /order_history

获取用户的委托历史（所有订单状态）。

**请求头:** `Authorization: Bearer <privy_jwt>`

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| event_id | i64 | 否 | 按事件 ID 筛选 |
| market_id | i16 | 否 | 按市场 ID 筛选 |
| page | i16 | 是 | 页码（从 1 开始） |
| page_size | i16 | 是 | 每页数量（1-100） |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "order_history": [
      {
        "order_id": "uuid",
        "event_title": "BTC 能否突破 10 万?",
        "event_image": "https://...",
        "market_title": "Yes/No",
        "market_question": "Will BTC reach 100K?",
        "market_image": "https://...",
        "token_id": "token_yes",
        "outcome": "Yes",
        "order_side": "buy",
        "order_type": "limit",
        "price": "0.6500",
        "quantity": "100.00",
        "volume": "65.00",
        "filled_quantity": "50.00",
        "cancelled_quantity": "0.00",
        "status": "partially_filled",
        "created_at": 1700000000,
        "updated_at": 1700000100
      }
    ],
    "total": 42,
    "has_more": true
  }
}
```

**字段说明:**
- `order_history`: 订单历史列表
  - `order_id`: 订单 ID
  - `event_title`: 事件标题
  - `event_image`: 事件图片 URL
  - `market_title`: 市场标题
  - `market_question`: 市场问题
  - `market_image`: 市场图片 URL
  - `token_id`: token ID
  - `outcome`: 结果名称
  - `order_side`: 订单方向（"buy" 或 "sell"）
  - `order_type`: 订单类型（"limit" 或 "market"）
  - `price`: 订单价格（字符串格式，4位小数）
  - `quantity`: 订单数量（字符串格式，2位小数）
  - `volume`: 订单金额（price * quantity，字符串格式）
  - `filled_quantity`: 已成交数量（字符串格式，2位小数）
  - `cancelled_quantity`: 已取消数量（字符串格式，2位小数）
  - `status`: 订单状态（"new"、"partially_filled"、"filled"、"cancelled"、"rejected"）
  - `created_at`: 创建时间戳（秒）
  - `updated_at`: 更新时间戳（秒）
- `total`: 符合条件的总记录数
- `has_more`: 是否还有更多数据

**订单状态说明:**
- `new`: 新订单（未成交）
- `partially_filled`: 部分成交
- `filled`: 完全成交
- `cancelled`: 已取消
- `rejected`: 被拒绝

---

### GET /event_balance

获取用户在指定事件的余额信息（USDC 余额和各 token 余额）。可选指定 market_id 来只获取特定市场的余额。

**请求头:** `Authorization: Bearer <privy_jwt>`

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| event_id | i64 | 是 | 事件 ID |
| market_id | i16 | 否 | 市场 ID（不提供则返回整个事件所有市场的 token 余额） |

**响应示例 1（指定 market_id）:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "token_available": {
      "token_id_yes": "100.0000",
      "token_id_no": "50.0000"
    },
    "cash_available": "5000.2500"
  }
}
```

**响应示例 2（不指定 market_id，返回整个事件）:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "token_available": {
      "token_id_yes_market1": "100.0000",
      "token_id_no_market1": "50.0000",
      "token_id_yes_market2": "200.0000",
      "token_id_no_market2": "150.0000"
    },
    "cash_available": "5000.2500"
  }
}
```

**字段说明:**
- `token_available`: 用户的 token 可用余额（HashMap<token_id, balance>）
  - 如果指定了 market_id，只返回该市场的 token
  - 如果未指定 market_id，返回该事件所有市场的所有 token（token_id 唯一，不会重复）
- `cash_available`: 用户的 USDC 可用余额

---

### GET /invite_stats

获取用户邀请统计信息（积分比例、boost倍数、邀请获得积分、被邀请人总交易量、邀请人数）。

**请求头:** `Authorization: Bearer <privy_jwt>` 或 `x-api-key: <api_key>`

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "invite_points_ratio": 5,
    "invitee_boost": "1.00",
    "invite_earned_points": 500,
    "invitees_total_volume": "10000.00",
    "invite_count": 10
  }
}
```

**字段说明:**
- `invite_points_ratio`: 邀请获得积分比例（除以100，如5表示5%）
- `invitee_boost`: 被邀请人boost倍数（Decimal字符串，如"1.50"表示1.5倍）
- `invite_earned_points`: 本赛季邀请获得的积分
- `invitees_total_volume`: 被邀请人总交易量
- `invite_count`: 用户成功邀请的人数

---

### GET /invite_records

获取用户的邀请记录列表（前 10 条，按时间倒序）。

**请求头:** `Authorization: Bearer <privy_jwt>`

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "records": [
      {
        "user_id": 123,
        "name": "被邀请用户名",
        "profile_image": "https://...",
        "created_at": "2024-01-15T10:30:00Z"
      }
    ]
  }
}
```

**字段说明:**
- `records`: 邀请记录列表（最多 10 条）
  - `user_id`: 被邀请用户的 ID
  - `name`: 被邀请用户的名称
  - `profile_image`: 被邀请用户的头像
  - `created_at`: 邀请时间（ISO 8601 格式）

---

### GET /season_current

获取当前活跃赛季信息。

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "season_id": 1,
    "name": "Season 1",
    "start_time": 1704067200,
    "end_time": 1706745600
  }
}
```

**字段说明:**
- `season_id`: 赛季ID
- `name`: 赛季名称
- `start_time`: 开始时间戳（秒）
- `end_time`: 结束时间戳（秒）

**错误码:**
- `2009`: 赛季不存在（没有活跃赛季）

---

### GET /seasons

获取所有赛季列表。

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "seasons": [
      {
        "season_id": 1,
        "start_time": 1704067200,
        "end_time": 1706745600
      },
      {
        "season_id": 2,
        "start_time": 1706745600,
        "end_time": 1709424000
      }
    ]
  }
}
```

**字段说明:**
- `seasons`: 赛季列表（按 season_id 降序排列）
  - `season_id`: 赛季ID
  - `start_time`: 开始时间戳（秒）
  - `end_time`: 结束时间戳（秒）

---

### GET /leaderboard_points

获取积分排行榜（前200名，支持历史赛季查询）。

**请求头:** 可选 `Authorization: Bearer <privy_jwt>` 或 `x-api-key: <api_key>`（用于获取当前用户排名）

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| season_id | i32 | 否 | 赛季ID，不传则查询当前赛季 |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "leaderboard": [
      {
        "rank": 1,
        "user_id": 123,
        "name": "用户名",
        "profile_image": "https://...",
        "total_points": 10000
      }
    ],
    "my_rank": "15",
    "my_total_points": 500
  }
}
```

**字段说明:**
- `leaderboard`: 排行榜列表（最多200条，仅包含积分大于0的用户）
  - `rank`: 排名
  - `user_id`: 用户ID
  - `name`: 用户名称
  - `profile_image`: 用户头像
  - `total_points`: 总积分
- `my_rank`: 当前用户排名（如果超过200名则为">200"，未登录则为null）
- `my_total_points`: 当前用户总积分（未登录则为null）

**错误码:**
- `2009`: 赛季不存在

---

### GET /leaderboard_volume

获取交易量排行榜（前200名，支持历史赛季查询）。

**请求头:** 可选 `Authorization: Bearer <privy_jwt>` 或 `x-api-key: <api_key>`（用于获取当前用户排名）

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| season_id | i32 | 否 | 赛季ID，不传则查询当前赛季 |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "leaderboard": [
      {
        "rank": 1,
        "user_id": 123,
        "name": "用户名",
        "profile_image": "https://...",
        "accumulated_volume": "100000.00"
      }
    ],
    "my_rank": "15",
    "my_accumulated_volume": "5000.00"
  }
}
```

**字段说明:**
- `leaderboard`: 排行榜列表（最多200条，仅包含交易量大于0的用户）
  - `rank`: 排名
  - `user_id`: 用户ID
  - `name`: 用户名称
  - `profile_image`: 用户头像
  - `accumulated_volume`: 累积交易量
- `my_rank`: 当前用户排名（如果超过200名则为">200"，未登录则为null）
- `my_accumulated_volume`: 当前用户累积交易量（未登录则为null）

**错误码:**
- `2009`: 赛季不存在

---

### GET /user_points_volumes

获取当前用户的赛季积分和交易量（需要认证）。

**请求头:** `Authorization: Bearer <privy_jwt>` 或 `x-api-key: <api_key>`

**请求参数:**
| 参数 | 类型 | 必填 | 说明 |
|------|------|------|------|
| season_id | i32 | 否 | 赛季ID，不传则查询当前赛季 |

**响应:**
```json
{
  "code": 0,
  "msg": "success",
  "data": {
    "total_points": 1500,
    "accumulated_volume": "50000.00"
  }
}
```

**字段说明:**
- `total_points`: 该赛季的总积分
- `accumulated_volume`: 该赛季的累积交易量

**数据来源:**
- 当前赛季：从 `points` 表获取
- 历史赛季：从 `season_history` 表获取

**错误码:**
- `2009`: 赛季不存在

---

## 错误码

| 错误码 | 说明 |
|--------|------|
| 0 | 成功 |
| 2001 | 鉴权失败 |
| 2002 | Privy 地址未设置 |
| 2003 | 参数无效 |
| 2004 | 用户不存在 |
| 2005 | 事件不存在 |
| 2006 | 市场不存在 |
| 2007 | Token ID 不存在 |
| 2008 | 签名验证失败 |
| 2009 | 赛季不存在 |
| 2010 | 事件已过期 |
| 2997 | 自定义错误 |
| 2998 | 内部错误 |
| 2999 | 未知错误 |
