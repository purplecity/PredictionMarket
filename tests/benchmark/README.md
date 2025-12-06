# Match Engine 压测

## 目录结构

```
tests/benchmark/
├── README.md                 # 本文档
├── Cargo.toml                # 作为 workspace member
├── src/
│   ├── lib.rs                # 库入口
│   ├── main.rs               # 压测入口
│   ├── scenarios.rs          # 测试场景
│   └── reporter.rs           # 结果报告
├── results/                  # 压测结果存放
└── configs/                  # 压测配置
```

## 压测阶段

### 阶段一：基准测试（Benchmark）
直接调用 `engine.submit_order()`，测试纯撮合性能上限。

**测试场景**:
1. **纯挂单**: 订单不撮合，测试 orderbook 插入性能
2. **完全撮合**: 每个订单都能完全成交
3. **部分撮合**: 混合场景，部分订单成交
4. **交叉撮合**: Yes/No 双向撮合
5. **深度压力**: orderbook 深度达到 10000+ 档位

**关键指标**:
- TPS (Transactions Per Second)
- 延迟分布 (P50/P95/P99/Max)
- 内存使用
- 每次撮合的平均成交笔数

### 阶段二：集成压测
通过 Redis Stream 注入订单，测试完整链路。

## 运行方式

```bash
# 从 workspace 根目录运行
cargo run -p benchmark --release -- --scenario all

# 运行特定场景
cargo run -p benchmark --release -- --scenario pure-insert --orders 100000

# 生成 JSON 报告
cargo run -p benchmark --release -- --scenario all --report json

# 生成 HTML 报告
cargo run -p benchmark --release -- --scenario all --report html

# 或者进入压测目录
cd tests/benchmark
cargo run --release -- --scenario all
```

## 测试场景说明

### 1. pure-insert (纯挂单)
- 所有买单价格低于所有卖单价格
- 测试 orderbook 插入和维护性能
- 预期: 最高 TPS

### 2. full-match (完全撮合)
- 买单价格 >= 卖单价格
- 每个 taker 订单都能完全成交
- 测试撮合算法性能

### 3. partial-match (部分撮合)
- 50% 订单能撮合，50% 挂单
- 更接近真实场景

### 4. cross-match (交叉撮合)
- 买 Yes 匹配卖 Yes + 买 No
- 测试双向撮合逻辑

### 5. deep-orderbook (深度压力)
- 先填充 10000 档深度（共 100000 订单）
- 然后测试在深度 orderbook 下的撮合性能

### 6. multi-market (多市场并发)
- 测试多个市场同时运行的并行性能
- 使用 std::thread 真正并行执行
- 验证多核扩展能力

```bash
# 测试4个市场并发
cargo run -p benchmark --release -- --scenario multi-market --markets 4

# 测试8个市场并发
cargo run -p benchmark --release -- --scenario multi-market --markets 8
```

## 性能基准 (MacBook Pro M系列, 100K订单)

| 场景 | TPS | P50 延迟 | P95 延迟 | P99 延迟 | Max 延迟 |
|------|-----|----------|----------|----------|----------|
| pure-insert | ~44K | 22μs | 24μs | 27μs | 8ms |
| full-match | ~33K | 22μs | 93μs | 97μs | 8ms |
| partial-match | ~33K | 22μs | 94μs | 99μs | 8ms |
| cross-match | ~33K | 22μs | 93μs | 97μs | 7ms |
| deep-orderbook | ~15.5K | 91μs | 96μs | 101μs | 190μs |

## 优化记录

### 2025-12-04: Arc<Order> + SmallVec 优化

**优化内容**:
1. **Arc<Order>** - 订单使用 `Arc<Order>` 共享，避免频繁克隆 Order 结构体
2. **SmallVec<[Arc<Order>; 16]>** - 每个价格档位使用 SmallVec，16个订单以内栈分配
3. **直接返回 Arc<Order>** - `get_cross_matching_orders` 直接返回订单引用，避免二次 HashMap 查询
4. **减少 String clone** - 使用引用避免不必要的 order_id 克隆

**效果**:

| 场景 | 优化前 TPS | 优化后 TPS | 提升 |
|------|------------|------------|------|
| pure-insert | ~43K | ~44K | +2% |
| full-match | ~30K | ~33K | +10% |
| cross-match | ~30K | ~33K | +10% |
| deep-orderbook | ~15K | ~15.5K | +3% |

### 2025-12-04: 交叉撮合架构重构

**问题**: 原有的归并式交叉撮合算法在价格相同时无法正确处理跨orderbook的order_num（时间优先）

**原因**: 当same_price == opposite_price时，归并算法默认取same side订单，但两个源的order_num可能交错

**重构方案**:
1. **提交时交叉插入** - 订单提交时同时插入到本方orderbook和对方orderbook（以相反方向和opposite_result_price）
2. **简化匹配逻辑** - 匹配时只需单向遍历本方orderbook，因为已包含所有候选订单
3. **移除深度快照交叉** - 深度快照无需临时交叉插入，直接读取已完整的orderbook

**优势**:
- 匹配时order_num自然正确（同一个orderbook内按order_num排序）
- 代码更简单，避免复杂的归并逻辑
- 深度快照性能提升（无需克隆和交叉插入）

### 2025-12-04: 多市场并发测试

**测试目的**: 验证多核并行扩展能力

**测试环境**: Apple M1 (8核: 4P核 + 4E核)

**测试结果** (full-match 场景, 每市场 100K 订单):

| 市场数 | 总 TPS | 理论 TPS (45K×N) | 实际效率 | 单市场 TPS |
|--------|--------|-----------------|----------|------------|
| 1 | ~42K | 45K | 93% | ~45K |
| 2 | ~39K | 90K | 43% | ~21K |
| 4 | ~36K | 180K | 20% | ~10K |
| 8 | ~34K | 360K | 9% | ~4.6K |

**分析**:
- 单线程纯撮合性能约 45K TPS（无 tokio 调度开销）
- 多核并行效率不高，主要瓶颈：
  1. **内存分配器竞争** - Rust 默认分配器在多线程下有全局锁
  2. **缓存竞争** - 多线程共享 L3 缓存导致 cache miss 增加
  3. **P核/E核差异** - M1 的 4 个 E 核性能较弱

**优化建议**:
- 生产环境可考虑使用 jemalloc/mimalloc 替代默认分配器
- 按市场分配到不同的 P 核可提升并行效率
- 实际系统中每个市场有独立的 Redis IO，天然形成并行

## 硬件要求

- CPU: 建议 4+ 核心
- 内存: 8GB+
- 存储: SSD（用于结果写入）
