# GPT_API 开发者指南

## 1. 架构深度解析

GPT_API 采用基于 Tokio 的全异步、无锁分层架构设计，旨在实现极致的吞吐量和低延迟。

### 1.1 核心组件交互

```text
Request -> [Axum Router] 
             |
             v
      [DispatcherService] (负载均衡与故障转移)
             |
             +---> [CommonHandler] (非流式请求 / Embedding / Rerank)
             |          |
             |          +--> [ClientManager] -> [Upstream]
             |          |
             |          +--> [Database Logger] (SQLx Pool)
             |
             +---> [StreamHandler] (SSE 流式请求)
                        |
                        +--> [ClientManager] -> [Upstream]
                        |
                        +--> [Non-blocking Stream Processor] (mpsc channel)
                                    |
                                    v
                             [Stream Logger Task] -> [Database Logger]
```

### 1.2 关键模块设计

#### 1.2.1 DispatcherService (调度层)
- **职责**: 负责所有请求的路由决策、负载均衡权重计算、以及最重要的 **Failover (故障转移)** 循环。
- **逻辑**: 采用责任链模式。如果一个上游节点返回 5xx 错误或连接超时，Dispatcher 会自动捕获错误，根据配置查找 Fallback 模型，并无缝重试，直到成功或所有备选方案耗尽。
- **日志一致性**: 无论重试多少次，最终返回给客户端的错误信息会包含完整的重试路径 (`Tried: ["A", "B"]`)，并确保与服务端 Access Log 完全一致。

#### 1.2.2 StreamHandler (流式处理层)
这是系统的性能热点区域。我们采用了一种**读写分离**的架构来处理 SSE 流：

- **主路径 (Hot Path)**: 负责将上游的 SSE 数据包透传给下游客户端。
    - **优化**: 使用 `String` 传递数据而非 `serde_json::Value`，避免了昂贵的 Deep Clone 操作。
    - **延迟**: 主路径仅做最少的必要处理（如前缀注入），确保 TTFT (Time To First Token) 极低。
- **后台日志任务 (Background Task)**:
    - 通过 `mpsc::unbounded_channel` 接收数据副本。
    - 在独立的 Tokio Task 中进行 JSON 反序列化、内容累积、Token Usage 统计和工具调用合并。
    - 这种设计将繁重的计算任务移出了关键路径。

## 2. 构建系统与优化

### 2.1 CPU 指令集优化
为了在现代云服务器上获得最佳性能，同时保证兼容性，我们制定了明确的编译策略：

- **x86-64-v3 (AVX2)**: 这是 Docker 镜像的默认目标。它在绝大多数现代服务器（Haswell 及以上）上支持 AVX2 指令集，能显著加速 `simd-json` 的性能。
    - *注意*: 我们不默认使用 `v4` (AVX-512)，因为部分 CI Runner (如 GitHub Actions) 并不支持，会导致构建过程中的 proc-macro 崩溃 (`SIGILL`)。
- **tsv110 (ARM64)**: 针对鲲鹏 920 等高性能 ARM 服务器优化。

### 2.2 内存分配器
项目默认链接 `mimalloc`。在 Rust 的异步高并发场景下，`mimalloc` 能有效减少内存碎片，并降低多线程下的分配器锁竞争，相比 `glibc` malloc 有约 10%-30% 的性能提升。

## 3. 测试与验证

我们提供了一个增强型的测试运行器 `GPT_API_TESTS/test_runner_enhanced.py`，它集成了 Mock Server、数据库验证和性能基准测试。

### 运行测试

必须在虚拟环境中运行：

```bash
# 1. 激活环境
source GPT_API_TESTS/.venv/bin/activate

# 2. 运行全量测试 (推荐)
python GPT_API_TESTS/test_runner_enhanced.py --api-version rust --stage all

# 3. 仅运行特定阶段
python GPT_API_TESTS/test_runner_enhanced.py --api-version rust --stage phase1 # 基础功能
python GPT_API_TESTS/test_runner_enhanced.py --api-version rust --stage phase4 # 性能压测
```

### 测试阶段说明
- **Phase 1**: 基础功能验证（Chat, Completion, Embedding, Input Validation, Logging）。
- **Phase 2**: 端到端流程验证（数据库记录完整性）。
- **Phase 3**: 负载均衡与权重分配验证。
- **Phase 4**: 性能基准测试 (RPS/Latency)。

## 4. 开发规范

1.  **提交前检查**: 必须运行 `cargo fmt` 和 `cargo build --release`。
2.  **新增依赖**: 引入新 crate 需谨慎，必须检查其对编译体积和启动速度的影响。
3.  **错误处理**: 所有网关层生成的错误必须使用 `AppError` 统一定义，并确保 Log 和 Response Body 内容一致。
