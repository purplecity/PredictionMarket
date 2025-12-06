# MatchEngine 测试文档

## 概述

本文档详细说明了 MatchEngine 项目的所有测试用例，包括测试目的、测试场景和预期结果。

## 测试文件位置

所有测试用例都集中在以下文件中：
- **测试文件**: `match_engine/tests/match_engine_tests.rs`
- **运行方式**: `cargo test --test match_engine_tests`

## 测试结构

测试分为两个主要部分：
1. **OrderBook 测试** - 订单簿核心功能测试
2. **MatchEngine 测试** - 撮合引擎核心功能测试

---

## OrderBook 测试

### 1. 基础功能测试

#### test_orderbook_new
**目的**: 测试新建订单簿
- 验证新订单簿初始状态为空
- 验证 symbol 正确设置
- 验证 bids、asks、order_price_map 都为空

#### test_orderbook_add_order_buy
**目的**: 测试添加买单
- 验证买单添加到 bids
- 验证价格键为负数（用于排序）
- 验证 order_price_map 正确记录

#### test_orderbook_add_order_sell
**目的**: 测试添加卖单
- 验证卖单添加到 asks
- 验证价格键为正数
- 验证 order_price_map 正确记录

#### test_orderbook_add_order_same_price_level
**目的**: 测试同一价格档位添加多个订单
- 验证同一价格的订单在同一价格档位
- 验证订单按时间顺序排列

#### test_orderbook_add_order_different_price_levels
**目的**: 测试不同价格档位
- 验证不同价格的订单在不同价格档位
- 验证价格档位数量正确

#### test_orderbook_add_order_symbol_mismatch
**目的**: 测试 symbol 不匹配错误
- 验证添加不同 symbol 的订单会失败
- 验证错误信息正确

#### test_orderbook_add_order_zero_quantity
**目的**: 测试数量为 0 的错误
- 验证 remaining_quantity 为 0 的订单无法添加
- 验证错误处理正确

### 2. 订单操作测试

#### test_orderbook_remove_order
**目的**: 测试移除订单
- 验证订单从订单簿中移除
- 验证 order_price_map 中订单被删除
- 验证返回的订单信息正确

#### test_orderbook_remove_order_not_found
**目的**: 测试移除不存在的订单
- 验证移除不存在的订单返回错误
- 验证错误处理正确

#### test_orderbook_remove_order_empties_price_level
**目的**: 测试移除订单后清空价格档位
- 验证当价格档位最后一个订单被移除时，价格档位也被移除
- 验证空价格档位的清理逻辑

#### test_orderbook_update_order
**目的**: 测试更新订单数量
- 验证订单数量正确更新
- 验证 filled_quantity 正确计算
- 验证订单状态正确更新（PartiallyFilled）

#### test_orderbook_update_order_to_zero
**目的**: 测试更新订单数量为 0
- 验证订单数量更新为 0 时状态变为 Filled
- 验证 filled_quantity 等于原始 quantity

#### test_orderbook_update_order_not_found
**目的**: 测试更新不存在的订单
- 验证更新不存在的订单返回错误

### 3. 价格查询测试

#### test_orderbook_best_bid
**目的**: 测试获取最佳买价
- 验证返回最高买价
- 验证价格排序正确（使用负数键实现降序）

#### test_orderbook_best_ask
**目的**: 测试获取最佳卖价
- 验证返回最低卖价
- 验证价格排序正确

#### test_orderbook_best_bid_empty
**目的**: 测试空订单簿的最佳买价
- 验证没有买单时返回 None

#### test_orderbook_best_ask_empty
**目的**: 测试空订单簿的最佳卖价
- 验证没有卖单时返回 None

#### test_orderbook_get_spread
**目的**: 测试获取价差
- 验证价差 = 最佳卖价 - 最佳买价
- 验证计算正确

#### test_orderbook_get_spread_no_bid
**目的**: 测试没有买单时的价差
- 验证没有买单时返回 None

#### test_orderbook_get_spread_no_ask
**目的**: 测试没有卖单时的价差
- 验证没有卖单时返回 None

### 4. 撮合测试

#### test_orderbook_get_matching_orders_buy
**目的**: 测试买单撮合逻辑
- 验证买单匹配价格 <= 买价的卖单
- 验证按价格优先（价格越低优先）
- 验证匹配数量正确

#### test_orderbook_get_matching_orders_sell
**目的**: 测试卖单撮合逻辑
- 验证卖单匹配价格 >= 卖价的买单
- 验证按价格优先（价格越高优先）
- 验证匹配数量正确

#### test_orderbook_get_matching_orders_event_order
**目的**: 测试市价单撮合
- 验证市价单匹配所有可匹配的订单
- 验证价格限制不适用于市价单

### 5. 深度和统计测试

#### test_orderbook_get_depth
**目的**: 测试获取订单簿深度
- 验证返回所有价格档位
- 验证买单从高到低排序
- 验证卖单从低到高排序
- 验证 update_id 正确设置

#### test_orderbook_get_depth_with_limit
**目的**: 测试限制深度的获取
- 验证 max_depth 参数生效
- 验证只返回指定数量的价格档位

#### test_orderbook_get_depth_aggregates_quantity
**目的**: 测试同一价格档位数量聚合
- 验证同一价格档位的多个订单数量正确聚合
- 验证 order_count 正确统计

#### test_orderbook_build_from_orders
**目的**: 测试从订单列表构建订单簿
- 验证从订单列表正确构建订单簿
- 验证所有订单正确添加到订单簿

#### test_orderbook_get_orderbook_stats
**目的**: 测试获取订单簿统计信息
- 验证统计信息正确（价格档位数量、订单数量、总数量等）

---

## MatchEngine 测试

### 1. 初始化测试

#### test_match_engine_new
**目的**: 测试新建 MatchEngine
- 验证初始状态（order_num, update_id 为 0）
- 验证 orders、price_level_changes 为空
- 验证 last_depth_snapshot 为 None

#### test_match_engine_new_with_orders
**目的**: 测试从订单列表创建 MatchEngine
- 验证订单正确加载
- 验证 order_num 正确设置（max + 1）
- 验证订单簿正确构建

### 2. 订单提交测试

#### test_match_engine_submit_order_no_match
**目的**: 测试无匹配订单的提交
- 验证订单添加到订单簿
- 验证订单状态为 New
- 验证订单数量正确

#### test_match_engine_submit_order_full_match
**目的**: 测试完全成交
- 验证 taker 订单完全成交
- 验证 maker 订单被移除
- 验证订单状态正确（Filled）

#### test_match_engine_submit_order_partial_match
**目的**: 测试部分成交
- 验证 taker 订单部分成交
- 验证 maker 订单完全成交并被移除
- 验证剩余订单正确添加到订单簿
- 验证订单状态正确（PartiallyFilled）

#### test_match_engine_submit_order_event_order_partial_match
**目的**: 测试市价单部分成交
- **验证市价单部分成交后不放在订单簿中**（这是关键测试点）
- 验证市价单部分成交时，剩余数量正确
- 验证 maker 订单完全成交并被移除
- 验证市价单不会添加到订单簿（即使部分成交）

#### test_match_engine_submit_order_event_order_full_match
**目的**: 测试市价单完全成交
- 验证市价单完全成交
- 验证市价单完全成交后不放在订单簿中
- 验证 maker 订单被移除
- 验证订单状态正确（Filled）

#### test_match_engine_submit_order_event_order_no_match
**目的**: 测试市价单无法匹配
- **验证市价单无法匹配时不放在订单簿中**（即使没有成交）
- 验证市价单没有成交时，remaining_quantity 和 filled_quantity 正确

#### test_match_engine_submit_order_limit_order_no_match_stays_in_orderbook
**目的**: 测试限价单无法匹配时放在订单簿中（与市价单形成对比）
- 验证限价单无法匹配时会放在订单簿中
- 验证限价单与市价单的行为差异

### 3. 订单取消测试

#### test_match_engine_cancel_order
**目的**: 测试取消订单
- 验证订单从订单簿中移除
- 验证 orders HashMap 中订单被删除

#### test_match_engine_cancel_order_not_found
**目的**: 测试取消不存在的订单
- 验证返回错误
- 验证错误处理正确

### 4. 撮合逻辑测试

#### test_match_engine_match_order_buy
**目的**: 测试买单撮合逻辑
- 验证匹配多个卖单
- 验证成交数量正确计算
- 验证 Trade 记录正确创建
- **验证成交价格**：成交价格是 maker 的价格（价格优先原则）
  - 第一个 trade：maker1 价格 400 → "40"（400/10 = 40.0）
  - 第二个 trade：maker2 价格 500 → "50"（500/10 = 50.0）
- 验证完全成交的 maker 订单被移除

#### test_match_engine_match_order_sell
**目的**: 测试卖单撮合逻辑
- 验证匹配多个买单
- 验证成交数量正确计算
- 验证 Trade 记录正确创建
- **验证成交价格**：成交价格是 maker 的价格，最高买价优先匹配
  - 第一个 trade：maker1 价格 600 → "60"（600/10 = 60.0）- 最高买价优先
  - 第二个 trade：maker2 价格 500 → "50"（500/10 = 50.0）
- 验证完全成交的 maker 订单被移除

### 5. 交叉撮合测试（get_cross_matching_orders）

交叉撮合规则是预测市场的核心特性：
- 买单 Yes：匹配相同结果的卖单（Yes的卖单，使用price）+ 相反结果的买单（No的买单，使用opposite_result_price），按价格从小到大排序
- 卖单 Yes：匹配相同结果的买单（Yes的买单，使用price）+ 相反结果的卖单（No的卖单，使用opposite_result_price），按价格从大到小排序
- opposite_result_price = 10000 - price（例如：No价格0.3，opposite_result_price = 0.7）

#### test_match_engine_cross_matching_buy_yes_same_result
**目的**: 测试买单Yes匹配相同结果的卖单
- 验证买单Yes匹配Yes结果的卖单（使用price）
- 验证按价格从小到大排序
- 验证成交价格正确

#### test_match_engine_cross_matching_buy_yes_opposite_result
**目的**: 测试买单Yes匹配相反结果的买单
- 验证买单Yes匹配No结果的买单（使用opposite_result_price）
- 验证按价格从小到大排序（使用opposite_result_price）
- 验证交叉撮合逻辑正确

#### test_match_engine_cross_matching_buy_yes_both_results
**目的**: 测试买单Yes同时匹配相同和相反结果的订单
- 验证买单Yes同时匹配Yes结果的卖单和No结果的买单
- 验证按价格从小到大排序（统一排序）
- 验证价格优先原则正确（0.05 < 0.7，先匹配Yes卖单）

#### test_match_engine_cross_matching_sell_yes_same_result
**目的**: 测试卖单Yes匹配相同结果的买单
- 验证卖单Yes匹配Yes结果的买单（使用price）
- 验证按价格从大到小排序
- 验证最高买价优先匹配

#### test_match_engine_cross_matching_sell_yes_opposite_result
**目的**: 测试卖单Yes匹配相反结果的卖单
- 验证卖单Yes匹配No结果的卖单（使用opposite_result_price）
- 验证按价格从大到小排序（使用opposite_result_price）
- 验证交叉撮合逻辑正确

#### test_match_engine_cross_matching_sell_yes_both_results
**目的**: 测试卖单Yes同时匹配相同和相反结果的订单
- 验证卖单Yes同时匹配Yes结果的买单和No结果的卖单
- 验证按价格从大到小排序（统一排序）
- 验证价格优先原则正确（0.7 > 0.06，先匹配No卖单）

#### test_match_engine_cross_matching_price_limit_buy
**目的**: 测试限价买单的价格限制
- 验证限价买单只匹配价格 <= taker_price 的订单
- 验证价格高于限价的订单不会被匹配
- 验证价格限制逻辑正确

#### test_match_engine_cross_matching_price_limit_sell
**目的**: 测试限价卖单的价格限制
- 验证限价卖单只匹配价格 >= taker_price 的订单
- 验证价格低于限价的订单不会被匹配
- 验证价格限制逻辑正确

#### test_match_engine_cross_matching_sell_opposite_result_price_limit
**目的**: 测试卖单taker匹配相反结果卖单时的价格限制（bug修复验证）
- **验证关键修复**：taker_price >= opposite_price 才能匹配
- 验证当taker价格不够高时，不会匹配opposite_price更高的订单
- 验证只有满足条件的订单被匹配，未匹配的订单仍然保留在订单簿中
- 场景：taker卖单Yes价格0.65，应该只匹配maker2（opposite_result_price = 0.6），不匹配maker1（opposite_result_price = 0.7）

### 6. 订单验证测试

#### test_match_engine_validate_order
**目的**: 测试订单验证逻辑
测试以下场景：
- ✅ 有效订单通过验证
- ❌ 数量为 0 的订单被拒绝
- ❌ 限价单但没有价格的订单被拒绝
- ❌ 用户ID为空的订单被拒绝

### 6. 快照测试

#### test_match_engine_handle_snapshot_tick
**目的**: 测试快照处理
- 验证 update_id 增加
- 验证深度快照正确保存
- **验证 price_level_changes 包含所有价格档位**（首次快照时记录所有档位）
- 验证深度快照包含正确的订单信息

---

## 运行测试

### 运行所有测试
```bash
cargo test --test match_engine_tests
```

### 运行特定测试
```bash
# 运行 OrderBook 测试
cargo test --test match_engine_tests test_orderbook

# 运行 MatchEngine 测试
cargo test --test match_engine_tests test_match_engine

# 运行特定测试
cargo test --test match_engine_tests test_orderbook_add_order_buy
```

### 运行测试并显示输出
```bash
cargo test --test match_engine_tests -- --nocapture
```

### 运行测试并显示详细输出
```bash
cargo test --test match_engine_tests -- --nocapture --test-threads=1
```

---

## 测试覆盖范围

### ✅ 已覆盖的功能

#### OrderBook
- ✅ 添加订单（买单/卖单）
- ✅ 移除订单
- ✅ 更新订单
- ✅ 获取最佳买价/卖价
- ✅ 获取价差
- ✅ 撮合逻辑
- ✅ 订单簿深度
- ✅ 统计信息

#### MatchEngine
- ✅ 订单提交
- ✅ 订单取消
- ✅ 订单撮合
- ✅ 订单验证
- ✅ 快照处理
- ✅ 价格档位变化检测
- ✅ 市价单处理（部分成交、完全成交、无法匹配）
- ✅ 交叉撮合（get_cross_matching_orders）

### 7. 价格档位变化检测测试

**注意**: 所有价格档位变化检测测试都会验证 `price_level_changes`。`handle_snapshot_tick` 会在开头清空上一次的变化，然后在快照后保留最新的变化供测试使用。

#### test_match_engine_update_price_level_changes_first_snapshot
**目的**: 测试首次快照（没有上次快照）
- **验证 price_level_changes 中记录了所有价格档位**（首次快照时记录所有档位）
- 验证买盘和卖盘价格档位的 total_quantity、price、side 等字段正确
- 验证 last_depth_snapshot 正确保存

#### test_match_engine_update_price_level_changes_quantity_change
**目的**: 测试价格档位数量变化
- **验证 price_level_changes 中记录了数量变化**
- 验证订单数量变化时，价格档位的 total_quantity 正确更新（从 100 变为 50）
- 验证变化被正确检测并记录，包括 price、side 等字段

#### test_match_engine_update_price_level_changes_new_level
**目的**: 测试新增价格档位
- **验证 price_level_changes 中记录了新价格档位**
- 验证新增价格档位时，新档位被正确记录（total_quantity、price 等）
- 验证只有变化的档位被记录（未变化的档位不记录）
- 验证快照包含所有价格档位

#### test_match_engine_update_price_level_changes_level_removed
**目的**: 测试价格档位消失（订单完全成交或被取消）
- **验证 price_level_changes 中记录了消失的价格档位**
- **验证消失的价格档位 quantity 为 0**
- 验证价格、side、symbol 等字段正确
- 验证只有消失的价格档位被记录（数量未变化的档位不记录）
- 验证最终快照状态正确

#### test_match_engine_update_price_level_changes_no_change
**目的**: 测试价格档位无变化
- **验证 price_level_changes 为空**（无变化时不记录）
- 验证 last_depth_snapshot 仍然存在

#### test_match_engine_update_price_level_changes_empty_orderbook
**目的**: 测试空订单簿
- **验证空订单簿时 price_level_changes 为空**
- 验证从空订单簿到有订单的状态变化被正确检测
- **验证 price_level_changes 中记录了新价格档位**（添加订单后）
- 验证 price、total_quantity 等字段正确

#### test_match_engine_update_price_level_changes_multiple_levels
**目的**: 测试多个价格档位同时变化
- **验证 price_level_changes 中记录了所有变化**（数量变化、新增档位、移除档位）
- 验证 buy1 数量变化（从 100 变为 50）
- 验证新增价格档位（550 -> 55，total_quantity = 300）
- 验证 sell1 价格档位消失（quantity = 0）
- 验证未变化的档位不记录在 price_level_changes 中
- 验证快照状态正确

#### test_match_engine_update_price_level_changes_same_price_level_multiple_orders
**目的**: 测试同一价格档位的多个订单变化
- **验证 price_level_changes 中记录了总数量变化**
- 验证同一价格档位的多个订单数量正确聚合（100 + 200 = 300）
- 验证移除订单后，price_level_changes 中记录了数量变化（从 300 变为 200）
- 验证快照的总数量和 order_count 正确

---

### ⚠️ 需要补充的测试（建议）

1. **并发测试**
   - 多线程环境下的订单处理
   - 并发订单提交
   - 并发订单取消

2. **性能测试**
   - 大量订单下的性能表现
   - 撮合性能基准测试

3. **集成测试**
   - 完整的订单流程（从提交到成交）
   - 多市场场景
   - 市场过期场景

4. **优雅停机测试**
   - graceful_shutdown 功能
   - 停机过程中的订单处理

5. **市场过期检查测试**
   - start_event_expiry_check_task 功能
   - 市场过期后的清理

---

## 测试辅助函数

测试文件包含以下辅助函数：

### create_test_symbol()
创建测试用的 PredictionSymbol

### create_test_order(id, side, price, quantity)
创建测试用的 Order

### create_match_engine()
创建测试用的 MatchEngine 实例

---

## 注意事项

1. **测试隔离**: 每个测试都是独立的，不会相互影响
2. **异步测试**: MatchEngine 测试使用 `#[tokio::test]`，因为 MatchEngine 是异步的
3. **同步测试**: OrderBook 测试使用 `#[test]`，因为 OrderBook 是同步的
4. **测试数据**: 所有测试使用模拟数据，不需要真实的 Redis 连接

---

## 维护说明

- 添加新功能时，请同时添加相应的测试用例
- 修改现有功能时，请更新相关测试用例
- 测试失败时，请检查：
  1. 测试数据是否正确
  2. 断言条件是否合理
  3. 代码逻辑是否改变
