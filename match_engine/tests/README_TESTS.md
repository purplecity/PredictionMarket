# MatchEngine 测试指南

## 📁 测试文件位置

**所有测试用例都在一个文件中**：
- `match_engine_tests.rs` (当前目录)

**测试文档**（同目录）：
- `TEST_DOCUMENTATION.md` - 详细的测试用例说明文档
- `README_TESTS.md` - 本文件（快速开始指南）

## 🚀 快速开始

### 运行所有测试
```bash
cargo test --test match_engine_tests
```

### 运行特定测试
```bash
# 运行所有 OrderBook 测试
cargo test --test match_engine_tests test_orderbook

# 运行所有 MatchEngine 测试
cargo test --test match_engine_tests test_match_engine

# 运行特定测试
cargo test --test match_engine_tests test_orderbook_add_order_buy
```

### 运行测试并查看输出
```bash
cargo test --test match_engine_tests -- --nocapture
```

## 📊 测试统计

### OrderBook 测试（28 个测试用例）
- ✅ 基础功能测试（7 个）
- ✅ 订单操作测试（6 个）
- ✅ 价格查询测试（7 个）
- ✅ 撮合测试（3 个）
- ✅ 深度和统计测试（5 个）

### MatchEngine 测试（22 个测试用例）
- ✅ 初始化测试（2 个）
- ✅ 订单提交测试（7 个）
  - 无匹配订单提交
  - 完全成交
  - 部分成交（限价单）
  - 市价单部分成交（不放在订单簿）
  - 市价单完全成交（不放在订单簿）
  - 市价单无法匹配（不放在订单簿）
  - 限价单无法匹配（放在订单簿）
- ✅ 订单取消测试（2 个）
- ✅ 撮合逻辑测试（2 个）
- ✅ 订单验证测试（1 个）
- ✅ 快照测试（1 个）
- ✅ 交叉撮合测试（9 个）
  - 买单Yes匹配相同结果
  - 买单Yes匹配相反结果
  - 买单Yes匹配两种结果
  - 卖单Yes匹配相同结果
  - 卖单Yes匹配相反结果
  - 卖单Yes匹配两种结果
  - 限价买单价格限制
  - 限价卖单价格限制
  - 卖单匹配相反结果卖单的价格限制（bug修复验证）

### 价格档位变化检测测试（8 个测试用例）
- ✅ 首次快照测试
- ✅ 数量变化测试
- ✅ 新增档位测试
- ✅ 档位消失测试
- ✅ 无变化测试
- ✅ 空订单簿测试
- ✅ 多档位变化测试
- ✅ 同一档位多订单测试

**总计：60 个测试用例**

## 📝 测试覆盖范围

### ✅ 已覆盖
- OrderBook 的所有核心方法
- MatchEngine 的核心撮合逻辑
- 订单验证逻辑
- 快照处理
- 价格档位变化检测
- 市价单处理（部分成交、完全成交、无法匹配时都不放在订单簿）
- 交叉撮合逻辑（get_cross_matching_orders）
  - 相同结果撮合
  - 相反结果撮合（使用opposite_result_price）
  - 价格排序规则
  - 限价单价格限制
- 边界情况处理

### ⚠️ 待补充（建议）
- 并发测试
- 性能测试
- 集成测试
- 优雅停机测试

## 📖 详细文档

查看同目录下的 `TEST_DOCUMENTATION.md` 获取每个测试用例的详细说明。
