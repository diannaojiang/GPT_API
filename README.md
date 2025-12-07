
# GPT_API (Rust Edition)

[![Release](https://img.shields.io/github/v/release/diannaojiang/GPT_API)](https://github.com/diannaojiang/GPT_API/releases) [![CI/CD](https://github.com/diannaojiang/GPT_API/actions/workflows/ci.yml/badge.svg)](https://github.com/diannaojiang/GPT_API/actions/workflows/ci.yml)

**高性能、生产级的 OpenAI API 聚合网关与负载均衡器。**

GPT_API 是一个基于 Rust 重构的高性能反向代理服务。它允许你统一管理多个 LLM 后端（如 OpenAI, Azure, DeepSeek, vLLM 等），并通过标准的 OpenAI 兼容接口对外提供服务。

本项目专为**高并发**场景设计，利用 Rust 的零成本抽象和 SIMD 指令集优化，在极低的资源占用下提供毫秒级的路由延迟。

## ✨ 核心特性

### 🚀 极致性能
- **Rust 驱动**: 基于 `Axum` 和 `Tokio` 异步运行时。
- **SIMD 加速**: 全面集成 `simd-json`，利用 AVX2/NEON 指令集加速 JSON 解析与序列化。
- **内存优化**: 默认使用 `mimalloc` 分配器，大幅减少高并发下的内存碎片与锁竞争。
- **硬件级优化**: Docker 镜像针对现代高性能多核处理器进行了特定指令集编译。

### 🔀 智能流量调度
- **多策略路由**: 支持关键字匹配 (`keyword`) 和精确匹配 (`exact`)。
- **加权负载均衡**: 支持为不同渠道设置权重 (`priority`)，自动分配流量。
- **自动故障转移 (Failover)**: 当主渠道发生 5xx 错误时，自动无缝切换至 `fallback` 备用模型，确保服务高可用。

### 🛠️ 企业级功能
- **零停机热重载**: 修改 `config.yaml` 后自动生效，无需重启服务。
- **流式处理 (SSE)**: 完美支持打字机效果，并支持在流式响应中注入自定义前缀（如 `<think>` 标签）。
- **数据清洗**: 自动移除特定模型的思考过程标签，通过 `once_cell` 优化的正则引擎处理消息。
- **审计日志**: 请求详情（Token消耗、延迟、IP）异步写入 SQLite 数据库，支持自动按月轮转归档。

## 🚀 快速部署 (Docker)

### 1. 准备配置
在宿主机创建 `config/config.yaml`：

```yaml
openai_clients:
  - name: "primary_gpt4"
    api_key: "sk-xxxx"
    base_url: "[https://api.openai.com/v1](https://api.openai.com/v1)"
    model_match:
      type: "keyword"
      value: ["gpt-4"]
    priority: 10
    fallback: "azure_backup"

  - name: "azure_backup"
    api_key: "azure-key"
    base_url: "[https://azure-endpoint.com/v1](https://azure-endpoint.com/v1)"
    model_match:
      type: "exact"
      value: ["gpt-4-backup"]
    priority: 1
```

### 2\. 启动服务

```bash

docker run -d \
  --name openai-api \
  -p 8000:8000 \
  -v $(pwd)/config:/app/config \
  -v $(pwd)/logs:/app/logs \
  --restart always \
  ghcr.io/diannaojiang/openai-api:b190
```

## 🛠️ 本地编译与开发

### 环境要求

  - Rust 1.75+
  - C 编译器 (gcc/clang)

### 编译 Release 版本

```bash
# 通用编译
cargo build --release

# 针对本机 CPU 极致优化 (推荐)
RUSTFLAGS="-C target-cpu=native" cargo build --release
```

## 📖 API 接口

完全兼容 OpenAI API 规范：

| 方法 | 路径 | 描述 |
| :--- | :--- | :--- |
| `POST` | `/v1/chat/completions` | 对话接口 (支持流式) |
| `POST` | `/v1/embeddings` | 向量化接口 |
| `POST` | `/v1/audio/transcriptions` | 语音转文字 (Whisper) |
| `GET` | `/v1/models` | 获取聚合模型列表 |
| `GET` | `/health` | 健康检查 |

## 📊 性能调优指南

本项目已在代码层面做了深度优化，建议在部署时配置以下环境变量以发挥最大性能：

  - **数据库轮转检查间隔**: `DB_ROTATION_CHECK_INTERVAL_SEC=60` (默认 60秒)
  - **文件描述符限制**: 确保宿主机 `ulimit -n` 大于 65535。


---

