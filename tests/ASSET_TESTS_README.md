# Asset Crate 测试文档

## 概述

本目录包含 asset crate 的集成测试。测试涵盖以下功能：

- **存款和取款** (`test_deposit_withdraw.rs`) - USDC 和 Token 的存取款操作
- **创建订单** (`test_create_order.rs`) - 买单和卖单的创建，余额冻结验证
- **取消订单** (`test_cancel_order.rs`) - 订单取消和拒绝，余额解冻验证
- **Split 和 Merge** (`test_split_merge.rs`) - USDC 与 Token 之间的分割和合并操作

## 测试特点

- **独立运行**：每个测试函数独立运行，不相互依赖
- **数据库隔离**：每个测试使用唯一的用户ID和事件ID，避免数据冲突
- **完整生命周期**：每个测试包含 setup（建立连接池）→ 执行 → cleanup（清理数据）→ teardown（关闭连接池）
- **从环境变量读取配置**：数据库连接信息从 `deploy/common.env` 读取
- **使用正确的表结构**：余额查询从 `positions` 表（而非 user_assets），使用 `balance` 和 `frozen_balance` 字段

## 前置要求

1. **数据库准备**：确保 PostgreSQL 数据库已创建并运行
2. **环境变量配置**：确保 `deploy/common.env` 文件存在且配置正确
3. **数据库Schema**：确保数据库schema已创建（运行 `script/database.sql`）

## 运行测试

### 运行所有测试

```bash
cargo test --package asset
```

### 运行单个测试文件

```bash
# 测试存款和取款
cargo test --package asset --test test_deposit_withdraw

# 测试创建订单
cargo test --package asset --test test_create_order

# 测试取消订单
cargo test --package asset --test test_cancel_order

# 测试 Split 和 Merge
cargo test --package asset --test test_split_merge
```

### 运行单个测试函数

```bash
# 测试 USDC 存款
cargo test --package asset --test test_deposit_withdraw test_usdc_deposit

# 测试创建买单
cargo test --package asset --test test_create_order test_create_buy_order

# 测试 Split 操作
cargo test --package asset --test test_split_merge test_split
```

### 显示测试输出

```bash
# 显示 println! 和 tracing 日志
cargo test --package asset -- --nocapture
```

## 测试列表

### test_deposit_withdraw.rs

- `test_usdc_deposit` - 测试 USDC 存款
- `test_token_deposit` - 测试 Token 存款
- `test_usdc_withdraw` - 测试 USDC 取款
- `test_withdraw_insufficient_balance` - 测试余额不足时取款失败
- `test_duplicate_deposit_idempotency` - 测试重复存款的幂等性

### test_create_order.rs

- `test_create_buy_order` - 测试创建买单
- `test_create_sell_order` - 测试创建卖单
- `test_create_order_insufficient_balance` - 测试余额不足时创建订单失败
- `test_create_multiple_orders` - 测试创建多个订单

### test_cancel_order.rs

- `test_cancel_buy_order` - 测试取消买单
- `test_cancel_sell_order` - 测试取消卖单
- `test_partial_cancel_order` - 测试部分取消订单
- `test_order_rejected` - 测试订单被拒绝

### test_split_merge.rs

- `test_split` - 测试 Split 操作（USDC → token_0 + token_1）
- `test_merge` - 测试 Merge 操作（token_0 + token_1 → USDC）
- `test_split_insufficient_balance` - 测试 Split 余额不足
- `test_merge_insufficient_balance` - 测试 Merge 余额不足
- `test_split_then_merge_roundtrip` - 测试 Split 然后 Merge 的往返操作

## 测试工具模块

`test_utils/mod.rs` 提供了以下辅助功能：

### TestEnv 结构体

- `setup()` - 初始化测试环境（创建数据库连接池）
- `teardown()` - 清理测试环境（关闭数据库连接池）
- `create_test_user()` - 创建测试用户
- `create_test_event_and_market()` - 创建测试事件和市场
- `get_user_balance()` - 获取用户总余额（balance + frozen_balance）
- `get_user_available_balance()` - 获取用户可用余额（balance）
- `get_user_frozen_balance()` - 获取用户冻结余额（frozen_balance）
- `cleanup_test_user()` - 清理测试用户数据
- `cleanup_test_event()` - 清理测试事件和市场

### 辅助函数

- `generate_test_user_id()` - 生成唯一的测试用户ID
- `generate_test_event_id()` - 生成唯一的测试事件ID
- `parse_decimal()` - 解析 Decimal 类型

## 重要概念

### 余额 = 持仓

在本系统中，**余额就是持仓**，所有余额相关的数据都存储在 `positions` 表中：

- **总余额** = `balance` + `frozen_balance`
- **可用余额** = `balance`（可以用于交易、取款等操作）
- **冻结余额** = `frozen_balance`（已被订单冻结，无法使用）

测试工具模块正确地从 `positions` 表查询这些字段。

## 注意事项

1. **串行运行**：虽然测试使用唯一ID，但如果遇到并发问题，可以使用 `--test-threads=1` 强制串行运行：
   ```bash
   cargo test --package asset -- --test-threads=1
   ```

2. **数据清理**：每个测试结束后会自动清理测试数据，但如果测试异常退出，可能需要手动清理数据库

3. **环境变量**：测试会从 `deploy/common.env` 读取以下环境变量：
   - `PREDICTION_EVENT_postgres_write_host`
   - `PREDICTION_EVENT_postgres_write_port`
   - `PREDICTION_EVENT_postgres_write_user`
   - `PREDICTION_EVENT_postgres_write_password`
   - `PREDICTION_EVENT_postgres_write_database`

4. **连接池管理**：每个测试函数都会创建和关闭自己的数据库连接池，确保资源正确释放

## 故障排查

### 测试失败：数据库连接错误

检查 `deploy/common.env` 中的数据库配置是否正确，确保 PostgreSQL 服务正在运行。

### 测试失败：表不存在

运行 `script/database.sql` 创建所需的数据库schema。

### 测试超时

如果测试运行时间过长，可以增加超时时间或检查数据库性能。

## 统计信息

- **总测试函数**: 19 个
- **测试文件**: 4 个
- **总代码行数**: 约 1,150+ 行
