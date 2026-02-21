# GPT_API (Rust Edition)

[![Release](https://img.shields.io/github/v/release/diannaojiang/GPT_API)](https://github.com/diannaojiang/GPT_API/releases) [![CI/CD](https://github.com/diannaojiang/GPT_API/actions/workflows/ci.yml/badge.svg)](https://github.com/diannaojiang/GPT_API/actions/workflows/ci.yml)

**高性能、生产级 OpenAI API 聚合网关与负载均衡器。**

GPT_API 是一个基于 Rust 重构的高性能反向代理服务。它允许你统一管理多个 LLM 后端（如 OpenAI, Azure, DeepSeek, vLLM, Llama.cpp 等），并通过标准的 OpenAI 兼容接口对外提供服务。

本项目专为**高并发**与**高可靠性**设计，利用 Rust 的零成本抽象和 SIMD 指令集优化，在极低的资源占用下提供毫秒级的路由延迟，并实现了业内领先的**流式响应全量审计**功能。

## ✨ 核心特性

### 🚀 极致性能
- **Rust 驱动**: 基于 `Axum` 和 `Tokio` 异步运行时，单核轻松处理数千并发。
- **SIMD 加速**: 核心路径集成 `simd-json`，利用 AVX2/NEON 指令集加速 JSON 解析与序列化。
- **内存优化**: 默认集成 `mimalloc` 分配器，在高并发场景下大幅减少内存碎片与锁竞争。
- **无锁流式处理**: 流式响应处理采用 `mpsc` 通道与后台任务分离架构，避免深拷贝 (Deep Clone)，确保首字节延迟 (TTFT) 最小化。

### 🔀 智能流量调度 (Advanced)
- **多策略路由**: 支持关键字匹配 (`keyword`) 和精确匹配 (`exact`)。
- **确定性路由 (Deterministic Routing)**: 采用基于内容的 **Multi-Anchor Voting (Rendezvous Hashing)** 算法。相同的提示词（Prompt）永远路由到同一后端，显著提升 **KV Cache 命中率**。
- **科学的加权算法**: 集成 **Efraimidis-Spirakis** 加权采样算法，在保持确定性的同时，确保长期流量分布严格遵循 `priority` 配置的权重比例，避免高权重节点抢占过多流量。
- **自动故障转移 (Failover)**: 当主渠道发生 5xx 错误或网络超时，自动无缝切换至 `fallback` 备用模型，确保服务高可用。

### 📊 深度可观测性 (New!)
- **流式审计**: 即使是 SSE 流式响应，也能完整记录 Token 消耗 (`usage`)、推理过程 (`reasoning_content`) 和工具调用 (`tool_calls`)。
- **Token 统计**: 完美兼容 OpenAI 标准 `stream_options` 和 llama.cpp 原生 `timings`，自动归一化 Token 统计格式。
- **请求回放**: 完整的请求体和响应体（包括流式拼接后的结果）异步写入 SQLite 数据库，支持按月自动轮转归档。
- **统一错误日志**: 无论上游返回何种错误格式，网关层统一标准化错误响应，确保日志与客户端接收到的错误信息完全一致 (422/500)。

### 🛠️ 企业级功能
- **零停机热重载**: 修改 `config.yaml` 后自动生效，无需重启服务。
- **数据清洗**: 自动移除特定模型的 `<think>` 思考标签（可配置），通过静态编译的正则引擎高效处理。
- **多模态支持**: 完整支持 Vision (图片) 和 Audio (Whisper) 请求转发。

## 🚀 快速部署 (Docker)

### 1. 准备配置
在宿主机创建 `config/config.yaml`：

```yaml
openai_clients:
  - name: "primary_gpt4"
    api_key: "sk-xxxx"
    base_url: "https://api.openai.com/v1"
    model_match:
      type: "keyword"
      value: ["gpt-4"]
    priority: 10
    fallback: "azure_backup"

  - name: "azure_backup"
    api_key: "azure-key"
    base_url: "https://azure-endpoint.com/v1"
    model_match:
      type: "exact"
      value: ["gpt-4-backup"]
    priority: 1
```

### 2. 启动服务

建议使用针对 x86-64-v3 (AVX2) 优化的镜像标签：

```bash
docker run -d \
  --name openai-api \
  -p 8000:8000 \
  -v $(pwd)/config:/app/config \
  -v $(pwd)/logs:/app/logs \
  --restart always \
  ghcr.io/diannaojiang/openai-api:b227
```

## 🛠️ 本地编译与开发

### 环境要求
- Rust 1.75+
- C 编译器 (gcc/clang)

### 编译 Release 版本

```bash
# 推荐：通用高性能构建 (x86-64-v3, 兼容大多数现代服务器)
RUSTFLAGS="-C target-cpu=x86-64-v3" cargo build --release

# 可选：针对本机 CPU 极致优化 (仅限本机运行)
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## 📖 支持接口

完全兼容 OpenAI API 规范及 vLLM/Rerank 扩展：

| 方法 | 路径 | 描述 |
| :--- | :--- | :--- |
| `POST` | `/v1/chat/completions` | 对话接口 (支持流式、多模态、工具调用) |
| `POST` | `/v1/completions` | 文本补全接口 (Legacy) |
| `POST` | `/v1/embeddings` | 向量化接口 |
| `POST` | `/v1/audio/transcriptions` | 语音转文字 (Whisper) |
| `POST` | `/v1/rerank` | Rerank 重排序接口 |
| `POST` | `/v1/score` | 文本评分接口 |
| `POST` | `/v1/classify` | 文本分类接口 |
| `GET` | `/v1/models` | 获取聚合模型列表 |
| `GET` | `/health` | 健康检查 |

## 📊 性能调优

本项目已在代码层面做了深度优化 (Zero-Copy Networking, SIMD)，建议在生产环境配置以下参数：

- **文件描述符**: 确保宿主机 `ulimit -n` 大于 65535。
- **数据库**: `record.db` (SQLite) 应置于高性能 SSD 上以避免 I/O 瓶颈。
- **日志级别**: 设置 `RUST_LOG=info` 以获取关键审计信息，`error` 仅记录故障。

---

