# 流式请求 Latency 测量问题修复方案

## 1. 问题分析

### 1.1 问题根因

当前 `gpt_api_latency_seconds` 对于流式请求只测量了 **TTFB（首字节时间）**，而非完整的流式传输时间。

**代码证据**：

- **`src/metrics/middleware.rs:72-86`**：
  ```rust
  let start = Instant::now();
  // ...
  let response = next.run(req).await;  // <-- 流式响应在这里立即返回
  let elapsed = start.elapsed().as_secs_f64();  // <-- 此时流还没结束
  ```

- **`src/handlers/stream_handler.rs:359-373`**：
  ```rust
  tokio::spawn(async move {
      stream_logger_task(...).await;  // <-- 后台运行，流结束后才结束
  });
  // <-- 响应对象立即返回，不等待 stream_logger_task 完成
  ```

### 1.2 技术原因

Axum 的 SSE 响应使用**惰性流（Lazy Stream）**：
- `Sse::new(sse_stream).into_response()` 只创建响应对象
- 实际的流数据在客户端连接时才会被消费
- `next.run(req).await` 在响应对象创建后**立即返回**，不等待流结束

### 1.3 现状对比

| 请求类型 | 当前测量 | 期望测量 |
|---------|---------|---------|
| 非流式 | 完整请求时间 | 完整请求时间 |
| 流式 | TTFB（首字节） | 完整请求时间 |

---

## 2. 推荐修复方案

### 方案 B（改进版）：在 `stream_logger_task` 完成后发送指标事件

**推荐理由**：
1. `stream_logger_task` 本身就知道流何时结束（channel 关闭时）
2. 不需要修改 middleware 逻辑，保持非侵入性
3. 复用现有的 `MetricEvent` 和 `MetricsSender` 基础设施
4. 与 `common_handler.rs` 中非流式请求的指标发送逻辑一致

**方案示意图**：
```
┌─────────────────┐     ┌──────────────────┐     ┌─────────────────┐
│ stream_handler  │     │ stream_logger    │     │  metrics worker │
│  (主请求处理)    │     │  _task (后台)    │     │   (指标处理)    │
└────────┬────────┘     └────────┬─────────┘     └────────┬────────┘
         │                       │                        │
         │ spawn task            │                        │
         │──────────────────────>│                        │
         │                       │                        │
         │ return response       │                        │
         │ immediately           │                        │
         │<──────────────────────┤                        │
         │                       │                        │
         │                       │ receive chunks         │
         │                       │───────────────────────>│
         │                       │                        │
         │                       │ stream ends            │
         │                       │ (rx.recv() = None)     │
         │                       │                        │
         │                       │ send MetricEvent       │
         │                       │───────────────────────>│
         │                       │                        │
```

---

## 3. 详细的代码修改计划

### 3.1 修改文件：`src/handlers/stream_handler.rs`

**位置**：在 `stream_logger_task` 函数末尾添加指标发送逻辑

**修改内容**：

1. **添加 import**（文件顶部）：
   ```rust
   // 在现有 imports 后添加
   use crate::metrics::middleware::get_metrics_sender;
   use crate::metrics::worker::MetricEvent;
   ```

2. **修改 `stream_logger_task` 函数签名**（第 121 行）：
   ```rust
   // 添加两个新参数用于指标发送
   async fn stream_logger_task(
       mut rx: mpsc::UnboundedReceiver<String>,
       app_state: Arc<AppState>,
       headers: HeaderMap,
       payload: RequestPayload,
       request_body: Value,
       client_ip: String,
       is_chat: bool,
       model: String,
       backend: String,
       start_time: Instant,
       endpoint: String,  // 新增
       status: String,    // 新增
   )
   ```

3. **在函数末尾添加指标发送**（第 288 行后，`}` 之前）：
   ```rust
   // 计算完整流式传输时间
   let total_elapsed = start_time.elapsed().as_secs_f64();
   
   // 尝试提取 usage 信息（如果之前捕获到了）
   let (completion_tokens, prompt_tokens) = captured_usage
       .as_ref()
       .map(|u| {
           let comp = u.get("completion_tokens").and_then(|v| v.as_u64());
           let prompt = u.get("prompt_tokens").and_then(|v| v.as_u64());
           (comp, prompt)
       })
       .unwrap_or((None, None));
   
   // 发送指标事件到 worker
   if let Some(sender) = get_metrics_sender() {
       let event = MetricEvent {
           endpoint,
           status,
           model: model.clone(),
           backend: backend.clone(),
           latency: total_elapsed,
           is_success: true,
           completion_tokens,
           prompt_tokens,
           elapsed: Some(total_elapsed),
       };
       let _ = sender.try_send(event);
   }
   ```

4. **修改 `process_streaming_response` 函数中的任务 spawn**（第 359-373 行）：
   ```rust
   // 添加 endpoint 和 status 参数
   tokio::spawn(async move {
       stream_logger_task(
           rx,
           app_state_clone,
           headers_clone,
           payload_clone,
           request_body_clone,
           client_ip_clone,
           is_chat,
           model_name,
           backend,
           stream_start_time,
           "/v1/chat/completions".to_string(),  // 新增：endpoint
           "200".to_string(),                    // 新增：status
       )
       .await;
   });
   ```

### 3.2 潜在修改：禁用 middleware 的流式请求指标

为了避免重复计算，可以选择在 middleware 中跳过流式请求的指标测量。

**位置**：`src/metrics/middleware.rs`

**修改内容**（第 71-84 行）：
```rust
pub async fn metrics_middleware(req: Request<Body>, next: Next) -> Response {
    let start = Instant::now();
    let endpoint = req.uri().path().to_string();

    // Skip metrics for non-API endpoints
    if should_skip_metrics(&endpoint) {
        return next.run(req).await;
    }

    // 检测是否为流式请求（通过 Header 或 Body 判断）
    let is_streaming = // ... 检测逻辑（可选）
    
    let response = next.run(req).await;
    
    // 如果是流式请求，跳过 middleware 的指标发送
    // 因为 stream_logger_task 会处理
    if is_streaming {
        return response;
    }
    
    // 原有逻辑继续...
}
```

**更简单的方案**：不做任何修改，让 middleware 正常发送（会记录 TTFB），`stream_logger_task` 再发送完整时间。这样可以得到两个指标：
- TTFB：来自 middleware
- 完整时间：来自 stream_logger_task

---

## 4. 潜在风险和注意事项

### 4.1 风险

1. **重复指标**：如果 middleware 和 stream_logger_task 都发送指标，会导致请求数翻倍
   - **缓解**：只在一处发送指标（推荐在 stream_logger_task 发送）

2. **Channel 溢出**：`try_send` 可能失败，如果 channel 满则丢失指标
   - **缓解**：当前使用 `try_send` 是合理的设计，丢失指标优于影响请求

3. **错误状态码处理**：当前代码假设流式请求都成功（status 200）
   - **注意**：如果上游返回错误（如 500），流式处理可能被提前终止

### 4.2 注意事项

1. **TPS 计算**：非流式请求的 `elapsed` 来自 `common_handler.rs:255`（请求发送时间），而流式请求的 `elapsed` 来自 `stream_logger_task`（流开始时间）。两者语义略有不同但都可接受。

2. **向后兼容**：这个修改不会破坏现有的 Prometheus 指标，只是让流式请求的 latency 指标更准确。

3. **性能影响**：发送指标是异步的（`try_send`），不会阻塞流式响应。

---

## 5. 测试建议

### 5.1 单元测试

- 验证 `stream_logger_task` 在流结束后发送 `MetricEvent`
- 验证 `MetricEvent` 中的 `latency` 大于 TTFB

### 5.2 集成测试

1. **手动测试**：
   ```bash
   # 发送流式请求
   curl -N 'http://localhost:8000/v1/chat/completions' \
     -H 'Content-Type: application/json' \
     -H 'Authorization: Bearer test-key' \
     -d '{"model": "test-model", "messages": [{"role": "user", "content": "Hello"}], "stream": true}'
   
   # 检查 Prometheus 指标
   curl http://localhost:8000/metrics | grep gpt_api_latency
   ```

2. **对比测试**：
   - 测量 TTFB（使用 middleware 修改前的日志）
   - 测量完整流式时间（使用修复后的指标）
   - 验证完整时间 > TTFB

### 5.3 测试模型名称

根据 AGENTS.md，可以使用专用模型名称触发 Mock 服务器返回特定响应以辅助测试。

---

## 6. 替代方案对比

| 方案 | 优点 | 缺点 | 工作量 |
|------|------|------|--------|
| **B（推荐）** | 不改 middleware，复用现有基础设施 | 需要传递额外参数到 task | 短 |
| A | 架构清晰 | 需要建新 channel，复杂 | 中 |
| C | 简单直接 | 阻塞主线程，失去流式优势 | 不推荐 |

---

## 7. 总结

**推荐采用方案 B**：
- 在 `stream_logger_task` 完成后发送 `MetricEvent`
- 计算从流开始到流结束的完整时间
- 复用 `common_handler.rs` 的指标发送模式
- 工作量约 **1-2 小时**

**关键修改文件**：
- `src/handlers/stream_handler.rs`（主要修改）
- `src/metrics/middleware.rs`（可选：禁用流式请求的 middleware 指标以避免重复）
