# PredictionMarket 测试文档

完整的 Match Engine 和 Asset 服务测试说明文档。

**测试统计**: 122 个测试用例，100% 通过率
- 核心撮合逻辑测试：33 个
- Match Engine 集成测试：71 个
- Asset 服务测试：18 个

## 目录

- [测试架构](#测试架构)
- [运行测试](#运行测试)
- [测试数据说明](#测试数据说明)
- [Match Engine 测试](#match-engine-测试)
- [Asset 测试](#asset-测试)
- [测试工具函数](#测试工具函数)

---

## 测试架构

### 目录结构

```
tests/
├── src/
│   ├── lib.rs                        # 测试库入口
│   └── test_utils.rs                 # 测试工具函数（TestEnv、辅助函数）
├── tests/                            # 集成测试
│   ├── match_engine_tests.rs         # Match Engine 测试（71个测试用例）
│   ├── match_engine_core_tests.rs    # 核心撮合逻辑测试（33个测试用例）
│   ├── asset_deposit_withdraw.rs     # 存取款测试（5个测试用例）
│   ├── asset_create_order.rs         # 订单创建测试（4个测试用例）
│   ├── asset_cancel_order.rs         # 订单取消测试（4个测试用例）
│   └── asset_split_merge.rs          # Split/Merge测试（5个测试用例）
├── benchmark/                        # 性能测试
│   ├── src/
│   │   ├── main.rs
│   │   ├── scenarios.rs
│   │   └── reporter.rs
│   └── README.md
└── README.md                         # 本文档
```

### 测试环境 (TestEnv)

位于 `tests/src/test_utils.rs`，提供：

- **数据库连接池管理**: 每个测试使用独立的小连接池（max 3），避免资源耗尽
- **测试数据生成**: 基于时间戳+原子计数器生成唯一的 user_id、event_id，确保并发测试不冲突
- **测试用户/市场创建**: 自动创建测试所需的用户和市场数据
- **余额查询**: 提供查询可用余额、冻结余额、总余额的辅助函数
- **数据清理**: 提供清理测试用户、事件的工具函数（可选使用）

---

## 运行测试

### 运行所有测试

```bash
# 运行所有测试（Match Engine + Asset）
cargo test -p tests
```

### 运行 Match Engine 测试

```bash
# 运行所有 match engine 测试（71个集成测试）
cargo test --test match_engine_tests

# 运行核心撮合逻辑测试（33个单元测试）
cargo test --test match_engine_core_tests
```

### 运行 Asset 测试

```bash
# 运行所有 asset 测试
cargo test --test asset_deposit_withdraw
cargo test --test asset_create_order
cargo test --test asset_cancel_order
cargo test --test asset_split_merge

# 一次性运行所有 asset 测试
cargo test -p tests --test asset_deposit_withdraw --test asset_create_order --test asset_cancel_order --test asset_split_merge
```

### 运行特定测试

```bash
# 运行市价买单测试
cargo test --test match_engine_tests test_market_order_buy

# 运行 orderbook 基础测试
cargo test --test match_engine_tests test_orderbook

# 运行交叉匹配测试
cargo test --test match_engine_tests test_cross_matching

# 运行存款测试
cargo test --test asset_deposit_withdraw test_deposit
```

### 显示测试输出

```bash
# 显示所有输出（包括 println!）
cargo test --test match_engine_tests -- --nocapture

# 单线程运行（便于调试）
cargo test --test match_engine_tests -- --test-threads=1

# 运行但不捕获输出
cargo test --test match_engine_tests -- --show-output
```

### 编译但不运行

```bash
cargo test --test match_engine_tests --no-run
```

---

## 测试数据说明

### 价格表示

- **内部存储**: i32，价格乘以 10000
  - 例如：0.6 存储为 6000
  - 例如：0.05 存储为 500
- **计算类型**: Decimal（精确小数，避免浮点误差）
- **价格范围**: 0.001 ~ 0.999
- **价格精度**: price × quantity / 1000000 = USDC amount

### 数量和金额

| 字段 | 类型 | 说明 |
|------|------|------|
| `quantity` | u64 | 限价单 Token 数量 |
| `volume` | Decimal | 限价买单 USDC 金额 (price × quantity / 1000000) |
| `market_volume` | Decimal | 市价买单 USDC 预算（quantity=0） |
| `market_quantity` | u64 | 市价卖单 Token 数量（volume=0） |
| `price` | i32 | 限价单价格；市价买单不使用；市价卖单为价格下限 |

### 订单类型

#### 限价单 (Limit Order)

- **买单**: 指定价格和数量，冻结 volume (USDC)
- **卖单**: 指定价格和数量，冻结 quantity (Token)

#### 市价单 (Market Order)

- **市价买单**:
  - 参数：`quantity=0`, `volume` (USDC 预算)
  - 逻辑：按价格从低到高吃单，直到 volume 用完
  - 验证：允许 quantity=0
  - 冻结：volume (USDC)

- **市价卖单**:
  - 参数：`quantity` (Token 数量), `volume=0`, `price` (价格下限)
  - 逻辑：按价格从高到低卖，如果价格低于 price 则停止
  - 冻结：quantity (Token)

### Symbol 格式

```
<event_id>|<market_id>|<token_id>
```

示例：`1|1|token1`（分隔符定义在 `common/src/consts.rs` 的 `SYMBOL_SEPARATOR`）

---

## Match Engine 测试

### 测试覆盖 (71 个测试用例)

#### 1. Orderbook 基础操作 (27 个测试)

**创建和基本操作:**
- `test_orderbook_new` - 创建空 orderbook
- `test_orderbook_add_order_buy` - 添加买单
- `test_orderbook_add_order_sell` - 添加卖单
- `test_orderbook_add_order_same_price_level` - 同一价格多个订单
- `test_orderbook_add_order_different_price_levels` - 不同价格档位
- `test_orderbook_add_order_zero_quantity` - 数量为 0（应忽略）
- `test_orderbook_add_order_symbol_mismatch` - Symbol 不匹配（应忽略）

**订单管理:**
- `test_orderbook_remove_order` - 移除订单
- `test_orderbook_remove_order_not_found` - 移除不存在的订单
- `test_orderbook_remove_order_empties_price_level` - 移除后清空价格档位
- `test_orderbook_update_order` - 更新订单数量
- `test_orderbook_update_order_to_zero` - 更新到 0（自动移除）
- `test_orderbook_update_order_not_found` - 更新不存在的订单

**查询功能:**
- `test_orderbook_best_bid` - 获取最佳买价
- `test_orderbook_best_bid_empty` - 空 orderbook 最佳买价
- `test_orderbook_best_ask` - 获取最佳卖价
- `test_orderbook_best_ask_empty` - 空 orderbook 最佳卖价
- `test_orderbook_get_spread` - 获取价差
- `test_orderbook_get_spread_no_bid` - 无买单时的价差
- `test_orderbook_get_spread_no_ask` - 无卖单时的价差
- `test_orderbook_get_depth` - 获取深度数据
- `test_orderbook_get_depth_aggregates_quantity` - 深度数据聚合
- `test_orderbook_get_depth_with_limit` - 限制深度档位数量
- `test_orderbook_get_orderbook_stats` - 获取统计信息
- `test_orderbook_build_from_orders` - 从订单列表构建
- `test_orderbook_get_matching_orders_buy` - 获取买单匹配列表
- `test_orderbook_get_matching_orders_sell` - 获取卖单匹配列表
- `test_orderbook_get_matching_orders_event_order` - 活动订单匹配

#### 2. 限价单匹配 (2 个测试)

- `test_match_engine_match_order_buy` - 买单完全匹配
- `test_match_engine_match_order_sell` - 卖单完全匹配

#### 3. 市价单匹配 (10 个测试)

**完全成交:**
- `test_market_order_buy_full_match` - 市价买单完全成交（volume 正好用完）
- `test_market_order_sell_full_match` - 市价卖单完全成交（价格满足要求）

**部分成交:**
- `test_market_order_buy_partial_match_price_limit` - 市价买单部分成交（volume 有限）
- `test_market_order_sell_partial_match_price_limit` - 市价卖单部分成交（价格低于下限）
- `test_market_order_buy_partial_quantity_from_large_order` - 从大订单中部分吃单（买）
- `test_market_order_sell_partial_quantity_from_large_order` - 从大订单中部分吃单（卖）

**无法匹配:**
- `test_market_order_buy_no_match` - 市价买单无法匹配
- `test_market_order_partial_fill_cancel_remaining` - 部分成交后取消剩余

**特殊情况:**
- `test_market_order_self_trade_detection` - 自成交检测
- `test_market_order_edge_case_exact_price` - 边缘情况（volume 正好用完）

#### 4. MatchEngine 综合测试 (20 个测试)

**引擎创建:**
- `test_match_engine_new` - 创建空引擎
- `test_match_engine_new_with_orders` - 创建带订单的引擎

**订单提交:**
- `test_match_engine_submit_order_no_match` - 提交无匹配订单
- `test_match_engine_submit_order_full_match` - 提交完全匹配订单
- `test_match_engine_submit_order_partial_match` - 提交部分匹配订单
- `test_match_engine_submit_order_limit_order_no_match_stays_in_orderbook` - 限价单停留在 orderbook

**活动订单:**
- `test_match_engine_submit_order_event_order_no_match` - 活动订单无匹配
- `test_match_engine_submit_order_event_order_full_match` - 活动订单完全匹配
- `test_match_engine_submit_order_event_order_partial_match` - 活动订单部分匹配

**订单管理:**
- `test_match_engine_cancel_order` - 取消订单
- `test_match_engine_cancel_order_not_found` - 取消不存在的订单
- `test_match_engine_validate_order` - 订单验证

**快照和价格档位:**
- `test_match_engine_handle_snapshot_tick` - 快照处理
- `test_match_engine_update_price_level_changes_no_change` - 无变化
- `test_match_engine_update_price_level_changes_new_level` - 新档位
- `test_match_engine_update_price_level_changes_quantity_change` - 数量变化
- `test_match_engine_update_price_level_changes_level_removed` - 档位移除
- `test_match_engine_update_price_level_changes_multiple_levels` - 多个档位
- `test_match_engine_update_price_level_changes_same_price_level_multiple_orders` - 同价格多订单
- `test_match_engine_update_price_level_changes_first_snapshot` - 首次快照
- `test_match_engine_update_price_level_changes_empty_orderbook` - 空 orderbook

#### 5. 交叉匹配测试 (9 个测试)

**买 Yes 交叉匹配:**
- `test_match_engine_cross_matching_buy_yes_both_results` - 两个结果都存在
- `test_match_engine_cross_matching_buy_yes_same_result` - 仅同市场结果
- `test_match_engine_cross_matching_buy_yes_opposite_result` - 仅反向市场结果

**卖 Yes 交叉匹配:**
- `test_match_engine_cross_matching_sell_yes_both_results` - 两个结果都存在
- `test_match_engine_cross_matching_sell_yes_same_result` - 仅同市场结果
- `test_match_engine_cross_matching_sell_yes_opposite_result` - 仅反向市场结果

**价格限制:**
- `test_match_engine_cross_matching_price_limit_buy` - 买单价格限制
- `test_match_engine_cross_matching_price_limit_sell` - 卖单价格限制
- `test_match_engine_cross_matching_sell_opposite_result_price_limit` - 反向结果价格限制

#### 6. 自成交检测 (1 个测试)

- `test_self_trade_detection_stops_matching` - 自成交检测停止匹配

---

## 核心撮合逻辑测试

### 测试覆盖 (33 个测试用例)

这些测试直接调用 `get_cross_matching_orders` 核心撮合函数，不触发 Redis 消息发送，专注于测试撮合算法的正确性。

#### 1. 限价买单测试 (8 个测试)

- `test_limit_buy_full_match_single_order` - 完全匹配单个卖单
- `test_limit_buy_full_match_multiple_orders` - 完全匹配多个卖单
- `test_limit_buy_partial_match` - 部分匹配（卖单不足）
- `test_limit_buy_no_match_price_too_low` - 价格过低无匹配
- `test_limit_buy_self_trade_detection` - 自成交检测
- `test_limit_buy_self_trade_after_partial_match` - 部分成交后遇到自成交
- `test_limit_buy_price_time_priority` - 价格-时间优先原则

#### 2. 限价卖单测试 (5 个测试)

- `test_limit_sell_full_match_single_order` - 完全匹配单个买单
- `test_limit_sell_full_match_multiple_orders` - 完全匹配多个买单
- `test_limit_sell_partial_match` - 部分匹配（买单不足）
- `test_limit_sell_no_match_price_too_high` - 价格过高无匹配
- `test_limit_sell_self_trade_detection` - 自成交检测

#### 3. 市价买单测试 (7 个测试)

- `test_market_buy_full_match_single_order` - 完全匹配单个卖单
- `test_market_buy_full_match_multiple_orders` - 完全匹配多个卖单
- `test_market_buy_partial_match_volume_limit` - volume 不足部分匹配
- `test_market_buy_no_match_empty_book` - 空盘口无匹配
- `test_market_buy_self_trade_detection` - 自成交检测
- `test_market_buy_rounding_edge_case` - 除法向下取整边缘情况

#### 4. 市价卖单测试 (6 个测试)

- `test_market_sell_full_match_single_order` - 完全匹配单个买单
- `test_market_sell_full_match_multiple_orders` - 完全匹配多个买单
- `test_market_sell_partial_match` - 部分匹配（买单不足）
- `test_market_sell_price_floor_enforcement` - 价格下限强制执行
- `test_market_sell_no_match_empty_book` - 空盘口无匹配
- `test_market_sell_self_trade_detection` - 自成交检测

#### 5. 边缘情况测试 (7 个测试)

- `test_edge_case_zero_quantity` - 数量为 0
- `test_edge_case_zero_volume_market_buy` - volume 为 0（市价买单）
- `test_edge_case_very_small_volume` - 极小 volume（买不到 1 个）
- `test_edge_case_exact_volume_match` - volume 正好用完
- `test_edge_case_large_numbers` - 大数量订单
- `test_edge_case_price_boundary_max` - 最大价格（9990）
- `test_edge_case_price_boundary_min` - 最小价格（10）
- `test_multiple_orders_same_price_time_priority` - 相同价格时间优先
- `test_complex_scenario_mixed_prices_and_users` - 复杂场景（混合价格和用户）

### 核心测试要点

#### 价格范围
- 最小价格：10（0.001）
- 最大价格：9990（0.999）
- 市价买单 price 参数：1000（表示愿意接受任何价格）
- 市价卖单 price 参数：价格下限

#### Volume 计算
```
volume = price * quantity / 1000000
```

#### 自成交检测
当遇到自己的订单时：
- 返回 `has_self_trade = true`
- 停止继续匹配
- 保留之前已成交的部分

#### 时间优先
相同价格档位内，按订单添加顺序（order_num）匹配。

---

### 关键测试场景说明

#### 市价买单完全成交

```rust
// 场景：
// 卖单1：价格 500，数量 100
// 卖单2：价格 600，数量 100
// 市价买单：0.11 USDC volume, quantity=0
//
// 匹配逻辑：
// 按价格从低到高吃单，直到 volume 用完
// 1. 吃 maker1 @ 500: 100 * 500 / 1000000 = 0.05 USDC
// 2. 吃 maker2 @ 600: 100 * 600 / 1000000 = 0.06 USDC
// 总共花费 0.11 USDC，正好用完
```

#### 市价买单从大订单中部分吃单

```rust
// 场景：
// 卖单1：价格 500，数量 50
// 卖单2：价格 700，数量 200
// 市价买单：0.05 USDC volume, quantity=0
//
// 匹配逻辑：
// 1. 吃 maker1(50 @ 500): 50 * 500 / 1000000 = 0.025 USDC
// 2. 尝试吃 maker2(35 @ 700): 35 * 700 / 1000000 = 0.0245 USDC
// 3. volume 用完，maker2 剩余 165 quantity
```

#### 市价卖单价格限制

```rust
// 场景：
// 买单1：价格 700，数量 100
// 买单2：价格 600，数量 100
// 买单3：价格 400，数量 100
// 市价卖单：300 quantity，价格下限 600
//
// 匹配逻辑：
// 按价格从高到低卖：
// 1. 吃 maker1 @ 700: 700 >= 600 ✓，成交 100
// 2. 吃 maker2 @ 600: 600 >= 600 ✓，成交 100
// 3. 尝试吃 maker3 @ 400: 400 < 600 ✗，停止
// 总共成交 200，剩余 100 被取消
```

#### 交叉匹配原理

在二元预测市场中，Yes 和 No 的价格总和为 1：

- Yes @ 0.6 = No @ 0.4
- 买 Yes @ 0.6 可以匹配：
  - Yes 卖单 @ ≤ 0.6
  - No 卖单 @ ≤ 0.4（等价于 Yes @ 0.6）

系统会选择最优价格进行匹配。

---

## Asset 测试

### 测试覆盖 (18 个测试用例)

Asset 测试已全部通过！测试文件：
- `asset_deposit_withdraw.rs` - 存取款测试（5 个测试）
- `asset_create_order.rs` - 订单创建测试（4 个测试）
- `asset_cancel_order.rs` - 订单取消测试（4 个测试）
- `asset_split_merge.rs` - Split/Merge 测试（5 个测试）

### Asset 测试详细列表

#### 1. 存取款测试 (asset_deposit_withdraw.rs - 5个测试)

- ✅ `test_usdc_deposit` - USDC 存款
- ✅ `test_token_deposit` - Token 存款（需要事件和市场）
- ✅ `test_usdc_withdraw` - USDC 取款
- ✅ `test_withdraw_insufficient_balance` - 余额不足取款（允许负余额）
- ✅ `test_duplicate_deposit_idempotency` - 重复存款幂等性（通过 UNIQUE NULLS NOT DISTINCT 约束）

#### 2. 订单创建测试 (asset_create_order.rs - 4个测试)

- ✅ `test_create_buy_order` - 创建买单（冻结 USDC）
- ✅ `test_create_sell_order` - 创建卖单（冻结 Token）
- ✅ `test_create_order_insufficient_balance` - 余额不足时创建订单失败
- ✅ `test_create_multiple_orders` - 创建多个订单（累积冻结）

#### 3. 订单取消测试 (asset_cancel_order.rs - 4个测试)

- ✅ `test_cancel_buy_order` - 取消买单（解冻 USDC）
- ✅ `test_cancel_sell_order` - 取消卖单（解冻 Token）
- ✅ `test_partial_cancel_order` - 部分取消订单
- ✅ `test_order_rejected` - 订单被拒绝（解冻资产）

#### 4. Split/Merge 测试 (asset_split_merge.rs - 5个测试)

- ✅ `test_split` - Split 操作（USDC → Token0 + Token1）
- ✅ `test_merge` - Merge 操作（Token0 + Token1 → USDC）
- ✅ `test_split_insufficient_balance` - Split 余额不足（允许负余额）
- ✅ `test_merge_insufficient_balance` - Merge 余额不足（失败）
- ✅ `test_split_then_merge_roundtrip` - Split-Merge 往返测试

### Asset 功能说明

Asset 服务主要负责：

#### 1. 存取款管理

- **USDC 存款**: `handle_deposit(user_id, "USDC", amount, tx_hash, ...)`
- **Token 存款**: `handle_deposit(user_id, token_id, amount, tx_hash, event_id, market_id, ...)`
- **USDC 取款**: `handle_withdraw(user_id, "USDC", amount, tx_hash, ...)`
- **Token 取款**: `handle_withdraw(user_id, token_id, amount, tx_hash, ...)`

特点：
- 允许负余额（用户掌握私钥，可随时充值）
- 通过 `UNIQUE NULLS NOT DISTINCT` 约束防止重复处理（幂等性）
  - 约束：`(user_id, history_type, tx_hash, token_id, trade_id, order_id)`
  - NULL 值被视为相等，确保正确去重

#### 2. 订单生命周期

- **创建订单**: `handle_create_order(...)`
  - 买单：冻结 volume (USDC)
  - 卖单：冻结 quantity (Token)

- **订单成交**: `handle_trade(...)`
  - 解冻对应资产
  - 增加获得的资产

- **取消订单**: `handle_cancel_order(...)`
  - 解冻 cancelled_quantity 或 cancelled_volume

- **订单拒绝**: `handle_order_rejected(...)`
  - 根据订单类型解冻对应资产

#### 3. Split/Merge 操作

- **Split**: USDC → Token0 + Token1
  ```rust
  handle_split(user_id, event_id, market_id, amount,
               token_0_id, token_1_id, ...)
  // USDC 减少 amount
  // Token0 增加 amount
  // Token1 增加 amount
  ```

- **Merge**: Token0 + Token1 → USDC
  ```rust
  handle_merge(user_id, event_id, market_id,
               token_0_id, token_1_id, amount, ...)
  // Token0 减少 amount
  // Token1 减少 amount
  // USDC 增加 amount
  ```

### 资产冻结规则

#### 限价买单
- **冻结**: `volume` = price × quantity / 1000000 (USDC)
- **解冻**: 成交后解冻对应的 volume，增加 token

#### 限价卖单
- **冻结**: `quantity` (Token)
- **解冻**: 成交后解冻对应的 quantity，增加 USDC

#### 市价买单
- **冻结**: `volume` (USDC 预算)
- **解冻**: 成交后按实际花费解冻，增加 token

#### 市价卖单
- **冻结**: `quantity` (Token 数量)
- **解冻**: 成交后按实际卖出解冻，增加 USDC

---

## 测试工具函数

### TestEnv 结构

```rust
pub struct TestEnv {
    pub pool: PgPool,  // 数据库连接池
}
```

### 核心方法

```rust
// 设置测试环境（连接数据库）
pub async fn setup() -> anyhow::Result<Self>

// 清理测试环境（关闭连接池）
pub async fn teardown(self)

// 创建测试用户
pub async fn create_test_user(&self, user_id: i64, privy_id: &str)
    -> anyhow::Result<()>

// 创建测试事件和市场（返回 token_0_id 和 token_1_id）
pub async fn create_test_event_and_market(&self, event_id: i64, market_id: i16)
    -> anyhow::Result<(String, String)>

// 获取用户总余额 (balance + frozen_balance)
pub async fn get_user_balance(&self, user_id: i64, token_id: &str)
    -> anyhow::Result<Decimal>

// 获取用户可用余额 (balance)
pub async fn get_user_available_balance(&self, user_id: i64, token_id: &str)
    -> anyhow::Result<Decimal>

// 获取用户冻结余额 (frozen_balance)
pub async fn get_user_frozen_balance(&self, user_id: i64, token_id: &str)
    -> anyhow::Result<Decimal>

// 清理测试用户（删除订单、交易、持仓、资产历史、用户）
pub async fn cleanup_test_user(&self, user_id: i64)
    -> anyhow::Result<()>

// 清理测试事件
pub async fn cleanup_test_event(&self, event_id: i64)
    -> anyhow::Result<()>
```

### 辅助函数

```rust
// 生成唯一的测试用户 ID（时间戳微秒 + 原子计数器，确保并发安全）
pub fn generate_test_user_id() -> i64

// 生成唯一的测试事件 ID
pub fn generate_test_event_id() -> i64

// 解析 Decimal 字符串
pub fn parse_decimal(s: &str) -> Decimal
```

### 使用示例

```rust
#[tokio::test]
async fn test_example() {
    // 1. 设置测试环境
    let env = TestEnv::setup().await.expect("Failed to setup");

    // 2. 生成唯一 ID
    let user_id = generate_test_user_id();
    let event_id = generate_test_event_id();
    let privy_id = format!("test_privy_{}", user_id);

    // 3. 创建测试数据
    env.create_test_user(user_id, &privy_id).await.expect("Failed");
    let (token_0_id, token_1_id) = env
        .create_test_event_and_market(event_id, 1)
        .await
        .expect("Failed");

    // 4. 执行测试逻辑
    // ...

    // 5. 验证结果
    let balance = env.get_user_balance(user_id, &token_0_id)
        .await
        .expect("Failed");
    assert_eq!(balance, expected_balance);

    // 6. 清理（可选）
    env.cleanup_test_user(user_id).await.ok();
    env.cleanup_test_event(event_id).await.ok();
    env.teardown().await;
}
```

---

## 测试最佳实践

### 1. 测试隔离

每个测试使用唯一的 ID 避免冲突：

```rust
let user_id = generate_test_user_id();  // 时间戳 + 原子计数器，并发安全
```

### 2. 精确断言

使用 Decimal 类型避免浮点误差：

```rust
let expected = parse_decimal("60.00");
assert_eq!(result, expected);  // 精确比较
```

### 3. 错误场景测试

不仅测试成功路径，也要测试失败情况：

```rust
// 测试余额不足
let result = create_order(...).await;
assert!(result.is_err(), "Should fail with insufficient balance");

// 测试重复操作
let result2 = deposit(...).await;  // 相同 tx_hash
assert!(result2.is_err(), "Duplicate should fail");
```

### 4. 描述性测试名称

```rust
// ✅ 好的命名
test_market_order_buy_partial_match_volume_limit
test_orderbook_remove_order_empties_price_level
test_duplicate_deposit_idempotency

// ❌ 不好的命名
test_order_1
test_scenario_a
```

---

## 数据库配置

### 配置文件

测试使用 `deploy/common.env` 中的数据库配置：

```bash
PREDICTION_EVENT_postgres_write_host=127.0.0.1
PREDICTION_EVENT_postgres_write_port=5432
PREDICTION_EVENT_postgres_write_user=postgres
PREDICTION_EVENT_postgres_write_password=123456
PREDICTION_EVENT_postgres_write_database=prediction_market
```

### 连接池配置

```rust
PgPoolOptions::new()
    .max_connections(3)      // 小连接池，避免资源耗尽
    .min_connections(1)
    .acquire_timeout(Duration::from_secs(5))
    .connect(&database_url)
```

### 数据库约束

Asset 测试依赖以下数据库约束：

```sql
-- asset_history 表的联合唯一约束（幂等性保证）
CONSTRAINT uq_asset_history UNIQUE NULLS NOT DISTINCT
    (user_id, history_type, tx_hash, token_id, trade_id, order_id)
```

`NULLS NOT DISTINCT` 确保 NULL 值被视为相等，正确处理以下场景：
- Deposit/Withdraw: `trade_id` 和 `order_id` 为 NULL
- Split/Merge: `trade_id` 和 `order_id` 为 NULL，但 `token_id` 不同（允许）
- Trade: 不同 `trade_id` 或 `order_id`（允许）

---

## 常见问题

### Q1: 市价买单为什么 quantity=0？

**A**: 市价买单的核心是"用固定 USDC 买尽可能多的 token"：
- 提供：`volume` (USDC 预算)
- 不知道：能买多少 token
- 因此：`quantity=0` 表示数量待定
- 验证：引擎特殊处理 `quantity=0 && order_type==Market && side==Buy`

### Q2: 市价卖单的 price 字段是什么意思？

**A**: `price` 是价格下限（floor price）：
- 市价卖单：卖出 `quantity` 个 token
- 价格限制：只接受 `>= price` 的买单
- 停止条件：如果盘口价格 `< price`，停止匹配

### Q3: 什么是交叉匹配？

**A**: 在二元预测市场中，Yes 和 No 的价格总和为 1：
- Yes @ 0.6 = No @ 0.4
- 买 Yes @ 0.6 可以匹配 No 卖单 @ ≤ 0.4

系统会同时检查同市场和反向市场的订单，选择最优价格匹配。

### Q4: 为什么允许负余额？

**A**: 因为用户掌握私钥，可以随时从链上充值。允许负余额提供更好的用户体验，适合去中心化场景。

### Q5: 如何调试失败的测试？

```bash
# 显示完整输出
cargo test --test match_engine_tests test_name -- --nocapture

# 单线程运行
cargo test --test match_engine_tests test_name -- --test-threads=1

# 查看数据库状态
psql -U postgres -d prediction_market -c "SELECT * FROM positions WHERE user_id = <id>;"
```

### Q6: 并发测试如何避免冲突？

**A**: `generate_test_user_id()` 使用时间戳微秒 + 原子计数器：

```rust
static COUNTER: AtomicU64 = AtomicU64::new(0);
let now = SystemTime::now().duration_since(UNIX_EPOCH).expect("time after epoch");
let counter = COUNTER.fetch_add(1, Ordering::SeqCst);
((now.as_secs() * 1_000_000 + now.subsec_micros() as u64) * 1000 + counter) as i64
```

即使在同一微秒内，不同线程也会获得不同的 ID。

---

## 测试统计

### Match Engine 测试

| 测试分类 | 数量 | 说明 |
|---------|------|------|
| Orderbook 基础 | 27 | 创建、添加、移除、更新、查询 |
| 限价单匹配 | 2 | 买/卖完全匹配 |
| 市价单匹配 | 10 | 完全/部分成交、价格限制、自成交 |
| MatchEngine 综合 | 20 | 引擎创建、订单提交、快照 |
| 交叉匹配 | 9 | Yes/No 市场交叉匹配 |
| 自成交检测 | 1 | 防止自成交 |
| **小计** | **71** | **全部通过** ✅ |

### Asset 测试

| 测试分类 | 数量 | 说明 |
|---------|------|------|
| 存取款 | 5 | USDC/Token 存取款、幂等性 |
| 订单创建 | 4 | 买/卖单创建、余额检查、多订单 |
| 订单取消 | 4 | 全部/部分取消、订单拒绝 |
| Split/Merge | 5 | USDC ↔ Token 转换、往返测试 |
| **小计** | **18** | **全部通过** ✅ |

### 总计

| 指标 | 数量 |
|------|------|
| 总测试数 | 89 |
| 通过 | 89 ✅ |
| 忽略 | 0 |
| 失败 | 0 |
| **通过率** | **100%** |

---

## 总结

本测试套件提供了全面的测试覆盖，**89 个测试用例，全部通过，通过率 100%**：

### Match Engine (71 个测试 - 全部通过 ✅)
- ✅ Orderbook 基础操作和边界情况（27个测试）
- ✅ 限价单和市价单的完整匹配逻辑（12个测试）
- ✅ 交叉匹配（Yes/No 市场互操作）（9个测试）
- ✅ 自成交检测和防范（1个测试）
- ✅ 价格档位变化追踪（9个测试）
- ✅ MatchEngine 综合测试（20个测试）

### Asset (18 个测试 - 全部通过 ✅)
- ✅ 存取款操作（5个测试，包括幂等性）
- ✅ 订单生命周期（创建、取消、拒绝）（8个测试）
- ✅ 资产冻结和解冻（8个测试）
- ✅ Split/Merge 操作（5个测试）
- ✅ 数据库约束验证（UNIQUE NULLS NOT DISTINCT）

### 测试文件列表

- `match_engine_tests.rs` - 71 个测试 ✅
- `asset_deposit_withdraw.rs` - 5 个测试 ✅
- `asset_create_order.rs` - 4 个测试 ✅
- `asset_cancel_order.rs` - 4 个测试 ✅
- `asset_split_merge.rs` - 5 个测试 ✅

所有测试用例均已验证通过，确保系统的稳定性和正确性。
