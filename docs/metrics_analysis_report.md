# 完整分析报告：GPT_API 指标系统 success_rate 统计逻辑

**分析日期**：2026-03-04  
**分析目标**：success_rate 及相关滑动窗口指标是否能正确按 model/backend 统计  
**代码版本**：GPT_API (Rust)

---

## 一、分析背景

用户反馈需要确认代码中 `success_rate`（成功率）的统计逻辑是否正确，特别是：
1. 总成功率统计
2. 各时段滑动窗口（1M/10M/1H）统计
3. 是否按 model/backend 正确区分

---

## 二、代码架构

### 2.1 核心模块

| 文件 | 职责 |
|------|------|
| `src/metrics/sliding_window.rs` | 滑动窗口实现，存储和计算指标 |
| `src/metrics/worker.rs` | 指标处理 worker，接收事件并更新指标 |
| `src/metrics/prometheus.rs` | Prometheus 指标定义 |
| `src/client/proxy.rs` | 实际请求执行，维护 active_requests 基础指标 |

### 2.2 数据流

```
请求处理
    │
    ▼
MetricEvent { endpoint, model, backend, latency, is_success }
    │
    ├──▶ proxy.rs: ACTIVE_REQUESTS.inc() / .dec()  ──▶ Prometheus (正常)
    │
    ▼
worker.rs: process_metric_event()
    │
    ├──▶ sliding_window::update_xxx() ──▶ 全局单一 SlidingWindow ❌
    │
    ▼
Prometheus 指标 .with_label_values([model, backend]).set(global_value)
    │
    ▼
所有 label 组合显示相同值 ❌
```

---

## 三、代码发现

### 3.1 滑动窗口实现 (sliding_window.rs)

```rust
// 全局单一窗口，无维度区分
static SUCCESS_WINDOW_1M: Lazy<SlidingWindow> = Lazy::new(|| SlidingWindow::new(60));
static SUCCESS_WINDOW_10M: Lazy<SlidingWindow> = ...
static SUCCESS_WINDOW_1H: Lazy<SlidingWindow> = ...
static SUCCESS_WINDOW_OVERALL: Lazy<SlidingWindow> = ...

// 同样问题存在于 LATENCY 和 ACTIVE
static LATENCY_WINDOW_1M: Lazy<SlidingWindow> = ...
static ACTIVE_WINDOW_1M: Lazy<SlidingWindow> = ...

// 函数只接受值，不接受维度键
pub fn update_success_windows(success: bool) {
    SUCCESS_WINDOW_1M.push(if success { 1.0 } else { 0.0 });
    SUCCESS_WINDOW_10M.push(if success { 1.0 } else { 0.0 });
    SUCCESS_WINDOW_1H.push(if success { 1.0 } else { 0.0 });
}

pub fn get_success_1m() -> f64 {
    SUCCESS_WINDOW_1M.avg()  // 返回全局平均值
}
```

**问题**：
- 所有请求（无论哪个 model/backend）写入同一个桶
- 函数签名无 model/backend 参数

### 3.2 Worker 处理 (worker.rs)

```rust
fn process_metric_event(event: MetricEvent) {
    let MetricEvent { endpoint, status, model, backend, latency, is_success } = event;
    
    // 更新滑动窗口 - 只传值，不传维度
    sliding_window::update_success_overall(is_success);
    sliding_window::update_success_windows(is_success);
    
    // 设置 Prometheus 指标 - 看似带维度，实际读全局
    SUCCESS_RATE_1M
        .with_label_values(&[&endpoint, &model, &backend])
        .set(sliding_window::get_success_1m());  // 读全局窗口！
    
    SUCCESS_RATE
        .with_label_values(&[&endpoint, &model, &backend])
        .set(sliding_window::get_success_overall());  // 读全局窗口！
}
```

**问题**：
- `update_success_xxx(is_success)` 只传 boolean，无 model/backend
- `.set(get_success_1m())` 所有 label 组合都读到同一个全局值

### 3.3 基础指标 vs 滑动窗口指标

| 指标类型 | 示例 | 状态 |
|----------|------|------|
| 基础 Gauge | `gpt_api_active_requests` | ✅ 正常 |
| 基础 Histogram | `gpt_api_latency` | 正常 |
| 滑动窗口 | `gpt_api_success_rate_1m` | ❌ Bug |

**原因**：基础 Prometheus 指标原生支持 labels，每个 label 组合独立存储。滑动窗口层破坏了这一点。

---

## 四、问题总结

### 4.1 问题列表

| # | 问题 | 严重性 | 影响 |
|---|------|--------|------|
| 1 | 滑动窗口无维度区分，所有请求聚合到单一全局桶 | 高 | 无法按 model/backend 查看成功率 |
| 2 | 滑动窗口函数签名缺少 model/backend 参数 | 高 | 架构层面无法支持按维度统计 |
| 3 | SUCCESS_RATE 完全无 per-label 统计 | 高 | 无法定位哪个模型故障 |
| 4 | LATENCY 滑动窗口同样问题 | 中 | 无法查看特定 model/backend 的 P99 延迟 |
| 5 | ACTIVE 滑动窗口同样问题 | 中 | 无法查看特定 model/backend 的峰值活跃数 |

### 4.2 数据验证

**当前行为**（Bug）：
```
gpt_api_success_rate{endpoint="/v1/chat/completions", model="gpt-4", backend="azure"} = 0.95
gpt_api_success_rate{endpoint="/v1/chat/completions", model="gpt-3.5", backend="openai"} = 0.95
gpt_api_success_rate{endpoint="/v1/chat/completions", model="claude", backend="anthropic"} = 0.95
```
所有组合显示相同的全局值。

**期望行为**：
```
gpt_api_success_rate{..., model="gpt-4", backend="azure"} = 0.99
gpt_api_success_rate{..., model="gpt-3.5", backend="openai"} = 0.95
gpt_api_success_rate{..., model="claude", backend="anthropic"} = 0.80  ← 应告警
```

---

## 五、影响评估

### 5.1 运维影响

- ❌ **无法定位故障**：当某个后端成功率下降时，无法从 metrics 快速发现
- ❌ **告警失效**：`success_rate{model="x"} < 0.95` 告警规则无效
- ❌ **SLA 统计困难**：无法按模型统计可用性
- ❌ **容量规划困难**：无法评估特定模型的负载峰值

### 5.2 系统影响

- 当前基础 Prometheus 指标（`gpt_api_active_requests`, `gpt_api_latency` histogram）正常工作
- 仅滑动窗口派生的 `_1m_max`, `_10m_max`, `_1h_max`, `_success_rate*` 指标受影响

---

## 六、修复建议

### 6.1 架构改动

将全局单一 SlidingWindow 改为 **HashMap 存储 per-key 窗口**：

```rust
use dashmap::DashMap;

static SUCCESS_WINDOWS_1M: Lazy<DashMap<String, SlidingWindow>> = 
    Lazy::new(|| DashMap::new());

pub fn update_success_windows(success: bool, model: &str, backend: &str) {
    let key = format!("{}:{}", model, backend);
    // 获取或创建该 key 的窗口
    let window = SUCCESS_WINDOWS_1M.entry(key).or_insert_with(|| SlidingWindow::new(60));
    window.push(if success { 1.0 } else { 0.0 });
}

pub fn get_success_1m(model: &str, backend: &str) -> f64 {
    let key = format!("{}:{}", model, backend);
    SUCCESS_WINDOWS_1M.get(&key).map(|w| w.avg()).unwrap_or(0.0)
}
```

### 6.2 需要修改的文件

| 文件 | 修改内容 |
|------|----------|
| `src/metrics/sliding_window.rs` | 用 DashMap 替代全局静态窗口，修改所有函数签名 |
| `src/metrics/worker.rs` | 传递 model/backend 到滑动窗口函数 |
| `src/metrics/prometheus.rs` | 无需修改（已有正确的 labels） |

### 6.3 额外考虑

1. **内存管理**：新 model/backend 出现时创建窗口，需设定 TTL 或最大数量限制防止内存膨胀
2. **向后兼容**：考虑添加新的 metric name 或保持现有 name（破坏性变更）
3. **测试**：补充滑动窗口 per-key 的单元测试

### 6.4 工作量估计

- 架构改动：1-2 天
- 测试验证：0.5 天
- **总计**：约 2 天

---

## 七、结论

1. ✅ **success_rate 统计确实存在问题**，无法按 model/backend 区分
2. ✅ **LATENCY/ACTIVE 滑动窗口同样问题**，基础指标正常
3. ✅ **根因是滑动窗口层设计缺陷**，需要重构为 HashMap per-key 架构
4. ⚠️ **当前 metrics 可用于全局监控**，但无法用于细粒度故障定位

---

## 附录：指标状态一览表

| 分类 | 指标名 | 按 model/backend 区分 | 状态 |
|------|--------|----------------------|------|
| **基础 Gauge** | `gpt_api_active_requests` | ✅ 是 | 正常 |
| **基础 Histogram** | `gpt_api_latency` | ✅ 是 | 正常 |
| **滑动窗口 - 1M** | `gpt_api_active_requests_1m_max` | ❌ 否 | **Bug** |
| **滑动窗口 - 10M** | `gpt_api_active_requests_10m_max` | ❌ 否 | **Bug** |
| **滑动窗口 - 1H** | `gpt_api_active_requests_1h_max` | ❌ 否 | **Bug** |
| **滑动窗口 - 1M** | `gpt_api_latency_1m_max` | ❌ 否 | **Bug** |
| **滑动窗口 - 10M** | `gpt_api_latency_10m_max` | ❌ 否 | **Bug** |
| **滑动窗口 - 1H** | `gpt_api_latency_1h_max` | ❌ 否 | **Bug** |
| **滑动窗口** | `gpt_api_success_rate` | ❌ 否 | **Bug** |
| **滑动窗口** | `gpt_api_success_rate_1m` | ❌ 否 | **Bug** |
| **滑动窗口** | `gpt_api_success_rate_10m` | ❌ 否 | **Bug** |
| **滑动窗口** | `gpt_api_success_rate_1h` | ❌ 否 | **Bug** |