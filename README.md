# GPT_API (Rust Edition)

[![Release](https://img.shields.io/github/v/release/diannaojiang/GPT_API)](https://github.com/diannaojiang/GPT_API/releases) [![CI/CD](https://github.com/diannaojiang/GPT_API/actions/workflows/ci.yml/badge.svg)](https://github.com/diannaojiang/GPT_API/actions/workflows/ci.yml)

**高性能、生产级 LLM API 聚合网关与负载均衡器。**

GPT_API 是一个基于 Rust 重构的高性能反向代理服务。它允许你统一管理多个 LLM 后端（如 OpenAI, Azure, Anthropic, DeepSeek, vLLM, Llama.cpp 等），并通过标准的 OpenAI 兼容接口以及 **Anthropic Messages API** 对外提供服务。

本项目专为**高并发**与**高可靠性**设计，利用 Rust 的零成本抽象和 SIMD 指令集优化，在极低的资源占用下提供毫秒级的路由延迟，并实现了业内领先的**流式响应全量审计**功能。

---

## ✨ 核心特性

### 🚀 极致性能
- **Rust 驱动**: 基于 `Axum` 和 `Tokio` 异步运行时，单核轻松处理数千并发。
- **SIMD 加速**: 核心路径集成 `simd-json`，利用 AVX2/NEON 指令集加速 JSON 解析与序列化。
- **Rayon 并行预处理**: 消息清洗、思考标签移除等预处理阶段通过 `rayon` 并行化，充分利用多核性能。
- **SSE 快速通道 (Fast Path)**: 无需 thinking 转换和前缀注入时，SSE 数据流无需 JSON 解析/重序列化，直接零拷贝透传，显著降低流式延迟。
- **配置缓存**: `extra_body` 在配置加载时预解析为 Value 树，运行时零开销注入。
- **内存优化**: 默认集成 `mimalloc` 分配器，在高并发场景下大幅减少内存碎片与锁竞争。
- **无锁流式处理**: 流式响应处理采用 `mpsc` 通道与后台任务分离架构，避免深拷贝 (Deep Clone)，确保首字节延迟 (TTFT) 最小化。
- **连接池调优**: TCP keepalive（30s）、空闲连接存活（5min）、每 host 最大空闲连接（64），减少高并发建连开销。

### 🔀 智能流量调度
- **多策略路由**: 支持关键字匹配 (`keyword`) 和精确匹配 (`exact`)。
- **三种负载均衡策略**:
  - **Deterministic**（默认）: 基于内容的 **Multi-Anchor Voting (Rendezvous Hashing)** 算法。相同的提示词（Prompt）永远路由到同一后端，显著提升 **KV Cache 命中率**。集成 **Efraimidis-Spirakis** 加权采样，确保长期流量分布严格遵循 `priority` 权重比例。
  - **Random**: 纯加权随机路由，无状态调度。
  - **LeastConnections**: 加权最少连接路由，实时读取活跃请求数（Prometheus gauge），将新请求调度至当前负载最低的后端。
- **自动故障转移 (Failover)**: 当主渠道发生 5xx 错误或网络超时，自动无缝切换至 `fallback` 备用模型，确保服务高可用。
- **灵活的客户端配置**: 支持自定义请求头 (`headers`)、提示词前缀 (`special_prefix`)、停止序列 (`stop`)、最大令牌数 (`max_tokens`) 和请求体字段注入 (`extra_body`)。

### 🔄 思考格式归一化 (Thinking Format Normalization)
- **多格式互转**: 将模型输出的推理/思考内容在 ` thinking... response`（ThinkTag）、`reasoning` 字段、`reasoning_content` 字段之间任意转换。
- **流式有状态转换**: 内置 `ThinkingStreamTransformer` 状态机，正确处理跨 SSE chunk 的标记断裂（如 `<thi` + `nk>`），确保流式场景下格式转换零丢失。
- **灵活配置**: 支持全局默认格式 + 每后端覆盖，按需适配不同客户端的需求。

### 📊 深度可观测性
- **全接口 Metrics 覆盖**: `/v1/chat/completions`、`/v1/responses`、`/v1/messages`、`/v1/completions` 均有独立的 Token 用量和延迟指标上报。
- **流式审计**: 即使是 SSE 流式响应，也能完整记录 Token 消耗 (`usage`)、推理过程 (`reasoning_content`) 和工具调用 (`tool_calls`)。
- **Token 统计**: 完美兼容 OpenAI 标准 `stream_options`、Anthropic `input_tokens`/`output_tokens` 和 llama.cpp 原生 `timings`，自动归一化 Token 统计格式。
- **请求回放**: 完整的请求体和响应体（包括流式拼接后的结果）异步写入 SQLite 数据库，支持按月自动轮转归档。
- **统一错误日志**: 无论上游返回何种错误格式，网关层统一标准化错误响应，确保日志与客户端接收到的错误信息完全一致 (422/500)。
- **Prometheus 监控**: 内置 `/metrics` 端点，提供请求总数、活跃请求数、成功率、延迟直方图等关键指标。

### 🛠️ 企业级功能
- **零停机热重载**: 修改 `config.yaml` 后自动生效，无需重启服务。
- **数据清洗**: 自动移除特定模型的 ` thinking` 思考标签，通过静态编译的正则引擎高效处理。
- **多模态支持**: 完整支持 Vision (图片) 和 Audio (Whisper) 请求转发。
- **消息优化**: 自动合并连续的用户消息、过滤空消息，减少 token 消耗。
- **上游健康检查**: 支持配置上游服务健康检查，自动摘除不健康节点。
- **可选 Check_API 认证插件**: 通过编译特性 `check-api-auth` 集成上游认证服务，无 feature 时编译器完全移除相关代码，零性能损失。

---

## 🐳 快速部署 (Docker)

### 1. 准备配置
在宿主机创建 `config/config.yaml`：

```yaml
# === 全局负载均衡策略 ===
load_balancing:
  strategy: deterministic   # deterministic | random | least_connections

# === 全局思考格式（可选，每客户端可覆盖）===
# thinking_format: "reasoning"  # passthrough | think_tag | reasoning | reasoning_content

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

# 可选：上游健康检查配置
check_config:
  enabled: true
  endpoint: http://127.0.0.1:3000/health
  interval: 30
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
  ghcr.io/diannaojiang/openai-api:b422
```

---

## ⚙️ 环境变量

| 变量名 | 默认值 | 说明 |
| :--- | :--- | :--- |
| `SERVER_PORT` | `8000` | 服务监听端口 |
| `RECD_PATH` | `sqlite:./record.db` | SQLite 数据库连接字符串 |
| `DB_ROTATION_CHECK_INTERVAL_SEC` | `60` | 数据库轮转检查间隔（秒） |
| `RUST_LOG` | `info` | 日志级别 (`error`, `warn`, `info`, `debug`, `trace`) |

---

## 📖 支持接口

完全兼容 OpenAI API 规范、Anthropic Messages API 及 vLLM/Rerank 扩展：

| 方法 | 路径 | 描述 |
| :--- | :--- | :--- |
| `GET` | `/health` | 健康检查 |
| `GET` | `/v1/models` | 获取聚合模型列表 |
| `GET` | `/metrics` | Prometheus 监控指标 |
| `POST` | `/v1/chat/completions` | 对话接口 (支持流式、多模态、工具调用) |
| `POST` | `/v1/responses` | OpenAI Responses API (支持流式 + 审计日志) |
| `POST` | `/v1/messages` | Anthropic Messages API (支持流式 SSE 透传 + 审计日志) |
| `POST` | `/v1/completions` | 文本补全接口 (Legacy) |
| `POST` | `/v1/embeddings` | 向量化接口 |
| `POST` | `/v1/audio/transcriptions` | 语音转文字 (Whisper) |
| `POST` | `/v1/audio/translations` | 语音翻译 |
| `POST` | `/v1/rerank` | Rerank 重排序接口 |
| `POST` | `/rerank` | Rerank 重排序 (兼容 vLLM) |
| `POST` | `/score` | 文本评分接口 (兼容 vLLM) |
| `POST` | `/classify` | 文本分类接口 (兼容 vLLM) |

---

## 📋 配置参考

### 全局配置项

```yaml
# 负载均衡策略（默认：deterministic）
load_balancing:
  strategy: deterministic   # deterministic | random | least_connections

# 全局思考格式归一化（可选，每客户端可覆盖）
# thinking_format: "reasoning"   # passthrough | think_tag | reasoning | reasoning_content

# 可选：上游健康检查
check_config:
  enabled: true
  endpoint: http://127.0.0.1:3000/health
  interval: 30
```

### 客户端配置项

```yaml
openai_clients:
  - name: "client_name"
    api_key: "sk-xxx"            # 可选，为空时使用客户端传入的 Key
    base_url: "https://api.example.com/v1"
    model_match:
      type: "keyword"            # 或 "exact"
      value: ["gpt-4", "claude"] # 匹配关键字/精确模型名
    priority: 10                 # 权重，越高越优先
    fallback: "backup_client"    # 故障转移目标
    special_prefix: "<PREFIX>"   # 可选：添加特殊前缀
    stop: ["<STOP1>"]            # 可选：停止序列
    max_tokens: 4096             # 可选：最大令牌数覆盖
    headers:                     # 可选：自定义请求头
      "X-Custom-Header": "value"
    extra_body: |                # 可选：JSON 对象，仅当请求未提供同名字段时注入
      {"frequency_penalty": 1, "presence_penalty": 0.91}
    thinking_format: "reasoning" # 可选：覆盖全局思考格式
```

### 思考格式 (ThinkingFormat) 说明

| 值 | 效果 |
| :--- | :--- |
| `passthrough` | 不做任何转换，原样透传上游输出（默认） |
| `think_tag` | 统一封装为 ` thinking... response` 包裹在 `content` 中 |
| `reasoning` | 统一放入独立的 `reasoning` 字段 |
| `reasoning_content` | 统一放入独立的 `reasoning_content` 字段 |

支持在流式和非流式两种模式下无损转换，正确处理跨 SSE chunk 的标记断裂。

---

## 🔧 可选编译特性

| 特性 | 命令 | 说明 |
| :--- | :--- | :--- |
| `check-api-auth` | `cargo build --features check-api-auth` | 集成 Check_API 认证 + Token 追踪插件，编译时无特性时零开销 |

---

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

---

## 📊 性能调优

本项目已在代码层面做了深度优化 (Zero-Copy Networking, SIMD, Rayon, SSE Fast Path)，建议在生产环境配置以下参数：

- **文件描述符**: 确保宿主机 `ulimit -n` 大于 65535。
- **数据库**: `record.db` (SQLite) 应置于高性能 SSD 上以避免 I/O 瓶颈。
- **日志级别**: 设置 `RUST_LOG=info` 以获取关键审计信息，`error` 仅记录故障。
- **连接池**: reqwest 客户端已内置 TCP keepalive（30s）、空闲连接存活（5min）、每 host 最大空闲连接数（64），无需额外配置。

---
