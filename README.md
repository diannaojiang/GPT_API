# OpenAI API Gateway (GPT_API - Rust Edition)

[![Release](https://img.shields.io/github/v/release/diannaojiang/GPT_API)](https://github.com/diannaojiang/GPT_API/releases) [![CI/CD](https://github.com/diannaojiang/GPT_API/actions/workflows/ci.yml/badge.svg)](https://github.com/diannaojiang/GPT_API/actions/workflows/ci.yml)

**高性能、企业级的 OpenAI API 聚合网关，基于 Rust 重构。**

GPT_API 是一个轻量级但在高并发下表现优异的反向代理服务。它允许您统一管理多个 LLM 后端（如 OpenAI, Azure, DeepSeek, Groq 等），并通过统一的 OpenAI 兼容接口对外提供服务。

相比原有的 Python 版本，Rust 重构版在保持 100% 业务逻辑兼容的同时，提供了毫秒级的路由延迟、更低的内存占用和极高的吞吐量。

## ✨ 核心特性

### 🚀 极致性能与架构
- **Rust 驱动**: 基于 `Axum` web 框架和 `Tokio` 异步运行时，专为高并发设计。
- **零停机热重载**: 修改 `config.yaml` 配置文件后，服务会自动检测并热加载新配置，无需重启即可生效。
- **多架构支持**: 提供 `linux/amd64` 和 `linux/arm64` 的 Docker 镜像，适配各类服务器与边缘设备。

### 🔀 智能路由与负载均衡
- **多策略路由**:
    - **Keyword**: 根据模型名称中的关键字（如 "gpt-4"）路由到特定后端。
    - **Exact**: 精确匹配模型名称。
- **加权负载均衡**: 支持为不同后端设置 `priority` 权重，实现流量的加权分配与负载分担。
- **自动故障转移 (Fallback)**: 当主后端请求失败时，自动无缝切换到配置的 `fallback` 备用模型，确保服务高可用。

### 🛠️ 高级参数处理与数据清洗
- **智能参数注入**:
    - **`special_prefix`**: 支持在响应内容（包括流式响应）前自动注入特定前缀（如 `<think>` 标签）。
    - **`stop`**: 自动向后端转发停止词配置，精准控制生成结束。
- **响应清洗**: 自动移除特定模型（如 DeepSeek）响应中的 `<think>` 思考过程标签，保持输出内容的整洁性。
- **消息优化**: 自动合并连续的 User 消息，过滤空消息，确请求格式符合上游要求。
- **Key 管理**: 支持从配置文件统一管理 API Key，也支持允许客户端在请求头中透传 Key。

### 📊 全面的可观测性
- **SQLite 审计日志**: 自动将所有请求详情（Prompt, Completion, Tokens, Latency, Client IP 等）持久化到 SQLite 数据库。
- **多模态与工具调用记录**: 智能识别并标记多模态（Vision）请求和 Function Calling 请求，便于后续分析。
- **自动归档**: 数据库文件按月自动轮转归档，防止单文件过大影响性能。
- **健康检查**: 提供 `/health` 端点用于负载均衡器探活。

## 🚀 快速部署 (Docker)

这是最简单的部署方式。

### 1. 创建配置文件

在宿主机创建配置文件 `config.yaml`：

```yaml
# config/config.yaml
openai_clients:
  # 示例 1: 官方 OpenAI (高优先级)
  - name: "official_openai"
    api_key: "${OPENAI_API_KEY}" # 支持从环境变量读取
    base_url: "https://api.openai.com/v1"
    priority: 10
    model_match:
      type: "keyword"
      value: ["gpt-4", "gpt-3.5"]
    fallback: "deepseek_backup"

  # 示例 2: DeepSeek (自定义参数)
  - name: "deepseek_service"
    api_key: "sk-xxxxxxxx"
    base_url: "https://api.deepseek.com"
    priority: 1
    model_match:
      type: "exact"
      value: ["deepseek-chat"]
    special_prefix: "【DeepSeek 思考】\n" # 在响应前添加前缀
    stop: ["<|endoftext|>"]

  # 示例 3: 备用服务 (仅在故障时调用)
  - name: "deepseek_backup"
    api_key: "sk-yyyyyyyy"
    base_url: "https://api.deepseek.com"
    priority: 999
    model_match:
      type: "exact"
      value: ["deepseek-chat-fallback"]
```

### 2. 启动容器

```bash
docker run -d \
  --name openai-api \
  -p 8000:8000 \
  -v $(pwd)/config/config.yaml:/app/config/config.yaml \
  -v $(pwd)/logs:/app/logs \
  -e OPENAI_API_KEY="sk-your-key-here" \
  --restart always \
  ghcr.io/diannaojiang/openai-api:b60
```

## 🛠️ 本地构建与开发

### 环境要求
- Rust 1.75+
- Cargo

### 编译与运行

```bash
# 1. 克隆项目
git clone https://github.com/diannaojiang/GPT_API.git
cd GPT_API

# 2. 编译 Release 版本
cargo build --release

# 3. 运行
./target/release/gpt_api
```

程序默认会在当前目录下的 `config/config.yaml` 查找配置，在 `logs/` 目录写入日志和数据库。

## 📖 API 接口说明

服务完全兼容 OpenAI API 规范。

| 方法 | 路径 | 描述 |
| :--- | :--- | :--- |
| `POST` | `/v1/chat/completions` | 标准对话接口 (支持流式) |
| `POST` | `/v1/completions` | 文本补全接口 (Legacy) |
| `GET` | `/v1/models` | 获取所有可用模型列表 (聚合) |
| `GET` | `/health` | 服务健康状态检查 |

---

## 📂 项目结构

```
GPT_API/
├── Cargo.toml              # Rust 项目依赖与配置
├── Dockerfile              # 多架构构建脚本
├── config/                 # 配置文件目录
├── src/
│   ├── main.rs             # 程序入口
│   ├── client/             # 客户端管理与负载均衡逻辑
│   ├── config/             # 配置热加载逻辑
│   ├── db/                 # SQLite 数据库操作与轮转
│   ├── handlers/           # HTTP 请求处理核心逻辑
│   │   ├── chat_handler.rs # 聊天接口处理 (含 SSE 流式逻辑)
│   │   └── utils.rs        # 消息清洗工具 (去重、标签移除等)
│   ├── routes/             # API 路由定义
│   └── state/              # 全局应用状态管理
└── target/                 # 编译产物
```