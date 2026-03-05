# TPS 与指标统计差距分析报告

> 分析日期: 2026-03-04  
> 项目: GPT_API (Rust 实现的高性能 OpenAI API 聚合网关)

---

## 1. 核心结论

### 1.1 非流式请求是否计算 TPS？

**确认：是的，非流式请求完全未计算 TPS。**

在 `stream_handler.rs` 中有完整的 TPS 计算逻辑（`stream_handler.rs:193-212`），而非流式请求处理器 `common_handler.rs` 中的 `process_non_streaming_response` 函数**完全没有** TPS 相关的指标记录。

### 1.2 流式有而非流式没有的指标（除 TTFT 外）

| 指标 | 流式请求 | 非流式请求 | 说明 |
|------|:--------:|:---------:|------|
| **TPS** (Tokens Per Second) | ✅ | ❌ | 每秒生成 Token 数 |
| **TPS_1M_AVG** (1分钟平均) | ✅ | ❌ | 滑动窗口平均值 |
| **TPS_10M_AVG** (10分钟平均) | ✅ | ❌ | 滑动窗口平均值 |
| **TPS_1H_AVG** (1小时平均) | ✅ | ❌ | 滑动窗口平均值 |
| **TTFT** (Time to First Token) | ✅ | ❌ | 流式特有指标 |
| **TTFT_1M_MAX** | ✅ | ❌ | 流式特有指标 |
| **TTFT_10M_MAX** | ✅ | ❌ | 流式特有指标 |
| **TTFT_1H_MAX** | ✅ | ❌ | 流式特有指标 |
| **TOKENS_TOTAL** (completion) | ✅ | ❌ | 累计 completion token 数 |
| **TOKENS_TOTAL** (prompt) | ✅ | ❌ | 累计 prompt token 数 |

---

## 2. 代码证据

### 2.1 流式请求 - TPS 计算逻辑

**文件**: `src/handlers/stream_handler.rs`  
**行号**: 193-212

```rust
// Record TPS when we have usage info
if let (Some(completion), Some(prompt)) = (
    u.get("completion_tokens").and_then(|v| v.as_u64()),
    u.get("prompt_tokens").and_then(|v| v.as_u64()),
) {
    let elapsed = start_time.elapsed().as_secs_f64();
    if completion > 0 && elapsed > 0.0 {
        let tps = completion as f64 / elapsed;
        TPS.with_label_values(&[&model, &backend]).observe(tps);
        sliding_window::update_tps_windows(tps);
        
        TPS_1M_AVG.with_label_values(&[&model, &backend])
            .set(sliding_window::get_tps_1m_avg());
        TPS_10M_AVG.with_label_values(&[&model, &backend])
            .set(sliding_window::get_tps_10m_avg());
        TPS_1H_AVG.with_label_values(&[&model, &backend])
            .set(sliding_window::get_tps_1h_avg());
    }
    
    // Token 计数
    TOKENS_TOTAL.with_label_values(&[&model, "completion"])
        .inc_by(completion as f64);
    TOKENS_TOTAL.with_label_values(&[&model, "prompt"])
        .inc_by(prompt as f64);
}
```

### 2.2 流式请求 - TTFT 计算逻辑

**文件**: `src/handlers/stream_handler.rs`  
**行号**: 157-183

```rust
// Record TTFT on first delta - whether content is empty or not
if is_chat && choice.get("delta").is_some() {
    let ttft = start_time.elapsed().as_secs_f64();
    TTFT.with_label_values(&[&model, &backend]).observe(ttft);
    sliding_window::update_ttft_windows(ttft);
    
    TTFT_1M_MAX.with_label_values(&[&model, &backend])
        .set(sliding_window::get_ttft_1m_max());
    // ... 10M, 1H 滑动窗口
}
```

### 2.3 非流式请求 - 缺失的指标记录

**文件**: `src/handlers/common_handler.rs`  
**函数**: `process_non_streaming_response` (行号 282-364)

该函数仅执行以下操作：
1. 解析响应 JSON (`simd_json::from_slice`)
2. 应用特殊前缀 (`apply_prefix_to_json`)
3. 记录数据库日志 (`log_non_streaming_request`)
4. 返回响应

**零 Prometheus 指标记录**

---

## 3. 指标差异可视化

```
┌─────────────────────────────────────────────────────────────────┐
│                        请求处理流程                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────────────┐    ┌──────────────────┐                  │
│  │   流式请求       │    │   非流式请求     │                  │
│  │  (stream_handler)│    │ (common_handler) │                  │
│  └────────┬─────────┘    └────────┬─────────┘                  │
│           │                        │                            │
│           ▼                        ▼                            │
│  ┌──────────────────┐    ┌──────────────────┐                  │
│  │ TTFT ✅          │    │ TTFT ❌          │                  │
│  │ TPS ✅           │    │ TPS ❌           │                  │
│  │ Tokens ✅        │    │ Tokens ❌        │                  │
│  │ LATENCY ❓       │    │ LATENCY ❓       │                  │
│  └──────────────────┘    └──────────────────┘                  │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## 4. 修改方案

### 4.1 目标文件

| 文件 | 修改内容 |
|------|---------|
| `src/handlers/common_handler.rs` | 添加 TPS、TOKENS 指标记录逻辑 |

### 4.2 需添加的代码位置

**函数**: `process_non_streaming_response` (第 282-364 行)

#### 步骤 1: 添加 Import

在文件顶部添加：
```rust
use crate::metrics::prometheus::{
    TOKENS_TOTAL, TPS, TPS_10M_AVG, TPS_1H_AVG, TPS_1M_AVG,
};
use crate::metrics::sliding_window;
```

#### 步骤 2: 记录请求开始时间

在函数开始处（约第 282 行）添加：
```rust
let start_time = Instant::now();
```

需要确保 `std::time::Instant` 已引入。

#### 步骤 3: 在成功响应块内添加指标记录

在 `if status.is_success()` 块内（约第 313 行）添加：
```rust
// 提取 usage 信息并记录指标
if let Some(usage) = response_body.get("usage") {
    if let (Some(completion), Some(prompt)) = (
        usage.get("completion_tokens").and_then(|v| v.as_u64()),
        usage.get("prompt_tokens").and_then(|v| v.as_u64()),
    ) {
        let elapsed = start_time.elapsed().as_secs_f64();
        
        // 记录 TPS
        if completion > 0 && elapsed > 0.0 {
            let tps = completion as f64 / elapsed;
            TPS.with_label_values(&[&model, &backend]).observe(tps);
            sliding_window::update_tps_windows(tps);
            
            TPS_1M_AVG.with_label_values(&[&model, &backend])
                .set(sliding_window::get_tps_1m_avg());
            TPS_10M_AVG.with_label_values(&[&model, &backend])
                .set(sliding_window::get_tps_10m_avg());
            TPS_1H_AVG.with_label_values(&[&model, &backend])
                .set(sliding_window::get_tps_1h_avg());
        }
        
        // 记录 Token 计数
        TOKENS_TOTAL.with_label_values(&[&model, "completion"])
            .inc_by(completion as f64);
        TOKENS_TOTAL.with_label_values(&[&model, "prompt"])
            .inc_by(prompt as f64);
    }
}
```

### 4.3 依赖项说明

以下模块已完备，无需修改：

| 文件 | 状态 |
|------|------|
| `src/metrics/prometheus.rs` | ✅ TPS, TPS_1M_AVG, TPS_10M_AVG, TPS_1H_AVG, TOKENS_TOTAL 已定义 |
| `src/metrics/sliding_window.rs` | ✅ `update_tps_windows()`, `get_tps_*_avg()` 已实现 |

---

## 5. 指标说明

### 5.1 TPS (Tokens Per Second)

**计算公式**: `completion_tokens / total_elapsed_time`

- 衡量模型每秒生成的 Token 数量
- 是评估模型推理性能的核心指标
- 滑动窗口提供短期和长期的平均值

### 5.2 TTFT (Time to First Token)

**仅适用于流式请求**，衡量从请求发起到收到第一个 Token 的时间。

- 对于非流式请求，此指标无意义（一次性返回完整响应）
- TTFT 反映了模型的"首响速度"，对用户体验至关重要

### 5.3 TOKENS_TOTAL

- 累计计数器，按 `model` 和 `type` (prompt/completion) 标签分类
- 用于统计总 Token 消耗，便于计费和容量规划

---

## 6. 总结

| 项目 | 状态 |
|------|------|
| 非流式请求计算 TPS | ❌ 缺失 |
| 非流式请求记录 TOKENS_TOTAL | ❌ 缺失 |
| 非流式请求记录 TTFT | ⚪ 不适用（流式特有）|
| 修改复杂度 | 低（仅一个函数） |
| 预估工作量 | 1-2 小时 |

**建议**: 在 `common_handler.rs` 的 `process_non_streaming_response` 函数中添加 TPS 和 Token 指标记录，实现与非流式请求的指标对等。