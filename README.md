# OpenAI API Gateway (GPT_API)

[![CI/CD](https://github.com/diannaojiang/GPT_API/actions/workflows/ci.yml/badge.svg)](https://github.com/diannaojiang/GPT_API/actions/workflows/ci.yml)

一个功能强大、高可配置性的 OpenAI API 代理网关。它允许您统一管理和分发来自不同渠道、不同模型的 API 请求，并提供了丰富的功能，如负载均衡、故障转移、日志记录和多模态支持。

## ✨ 主要功能

- **多后端支持**: 可同时配置多个不同的 API 服务商（如 OpenAI、Azure、Groq、以及任何兼容 OpenAI 接口的自定义服务）。
- **智能模型路由**: 根据请求中的模型名称 (`model`)，自动将请求转发到预先配置好的后端服务。支持精确匹配和关键字匹配。
- **故障转移 (Fallback)**: 当主服务请求失败时，可自动切换到备用服务，确保服务的持续可用性。
- **统一日志记录**:
    - **请求日志**: 使用 `loguru` 记录详细的请求与响应信息，自动分割为 `INFO` 和 `ERROR` 日志文件，并按大小和时间进行轮转。
    - **数据库记录**: 将每一次 API 调用的详细信息（包括 Token 使用量、是否为多模态、是否使用工具等）持久化到 SQLite 数据库中。
- **数据库自动归档**: 数据库会按月份自动进行归档，便于管理和查阅历史数据。
- **流式与非流式响应**:完美支持官方的流式（`stream=True`）和非流式响应模式。
- **参数扩展与兼容**:
    - 支持 `/v1/completions` 和 `/v1/chat/completions` 两个主要的 OpenAI 接口。
    - 支持 `/v1/models` 接口，可动态从所有后端拉取并聚合模型列表。
    - 支持多模态请求（如 `gpt-4-vision-preview`）并进行记录。
- **动态配置重载**: 无需重启服务，通过访问 `/models` 接口即可刷新并加载最新的服务配置。
- **容器化部署**: 提供优化的 `Dockerfile` 和部署说明，方便快速上線。
- **CI/CD 集成**: 提供开箱即用的 GitHub Actions 工作流，实现自动化测试、构建和部署。

## 🚀 快速开始

### 1. 环境准备

- Python 3.8+
- Git

### 2. 本地部署

**a. 克隆仓库**
```bash
git clone [https://github.com/diannaojiang/GPT_API.git](https://github.com/diannaojiang/GPT_API.git)
cd GPT_API
```

**b. 安装依赖**
```bash
pip install -r requirements.txt
```

**c. 创建配置文件**

在 `openai_api/config/` 目录下创建一个 `config.yaml` 文件。这是项目的核心配置，用于定义您的后端 API 服务。

`openai_api/config/config.yaml`:
```yaml
openai_clients:
  # --- 第一个后端服务：官方 OpenAI ---
  - name: "official_openai"
    # API Key 可以直接写入，或使用 ${ENV_VAR} 格式从环境变量读取
    api_key: "${OPENAI_API_KEY}"
    base_url: "[https://api.openai.com/v1](https://api.openai.com/v1)"
    # 优先级，数字越小，优先级越高
    priority: 1
    # 模型匹配规则
    model_match:
      type: "keyword" # 可选 "exact" (精确匹配) 或 "keyword" (关键词匹配)
      value: ["gpt-4", "gpt-3.5-turbo"]
    # 备用/故障转移模型，当此服务请求失败时，会尝试使用此模型重新请求
    # 它会根据 fallback 模型名称去匹配另一个 client
    fallback: "deepseek-chat-fallback"

  # --- 第二个后端服务：自定义服务（如 Groq, Deepseek 等） ---
  - name: "deepseek_api"
    api_key: "${DEEPSEEK_API_KEY}"
    base_url: "[https://api.deepseek.com](https://api.deepseek.com)"
    priority: 2
    model_match:
      type: "exact"
      value: ["deepseek-chat", "deepseek-coder"]
    # 为此后端的所有响应添加特殊前缀
    special_prefix: "[DeepSeek]"
    # 为此后端添加预设的停止词
    stop: ["<|endoftext|>"]

  # --- 作为备用的模型配置 ---
  - name: "deepseek_api_fallback_client"
    api_key: "${DEEPSEEK_API_KEY}"
    base_url: "[https://api.deepseek.com](https://api.deepseek.com)"
    priority: 999 # 优先级设为最低，仅在被 fallback 调用时使用
    model_match:
      type: "exact"
      value: ["deepseek-chat-fallback"] # 定义一个独特的模型名
    fallback: False # 备用服务不应再有备用
```

**d. 启动服务**
```bash
# 直接通过 uvicorn 启动
uvicorn openai_api.main:app --host 0.0.0.0 --port 8000 --workers 16
```
或者，如果您在类 Unix 系统中，可以使用项目提供的脚本：
```bash
bash openai_api/run.sh
```

### 3. Docker 部署

我们提供了优化后的 `Dockerfile` 用于容器化部署。

**a. 构建镜像**
```bash
docker build -t openai-gateway .
```

**b. 运行容器**

推荐将配置、日志和数据库目录挂载到宿主机，方便管理和持久化。

```bash
# 1. 在宿主机创建所需目录
mkdir -p ./config ./logs ./database

# 2. 将您的 config.yaml 放入 ./config 目录

# 3. 运行容器
docker run -d \
  -p 8000:8000 \
  -v $(pwd)/config:/app/openai_api/config \
  -v $(pwd)/logs:/app/logs \
  -v $(pwd)/database:/app/database \
  -e OPENAI_API_KEY="sk-..." \
  -e DEEPSEEK_API_KEY="sk-..." \
  -e RECD_PATH="/app/database/record.db" \
  --name openai-gateway \
  openai-gateway
```
**注意**:
- `-e RECD_PATH` 环境变量用于指定数据库文件的路径，应指向挂载的卷内。
- API Keys 可以通过 `-e` 参数传入环境变量。

## 📖 API 端点

服务启动后，您可以像调用官方 OpenAI API 一样调用以下端点：

- `POST /v1/chat/completions`: 聊天接口。
- `POST /v1/completions`: 文本补全接口。
- `GET /v1/models`: 获取所有后端聚合后的模型列表。访问此接口会自动刷新配置。

**请求示例:**
```bash
curl http://localhost:8000/v1/chat/completions \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4-turbo",
    "messages": [{"role": "user", "content": "Hello!"}],
    "stream": false
  }'
```
该请求会因为 `model` 包含 "gpt-4" 关键字而被路由到 `official_openai` 后端。

## 📁 项目结构

```
.
├── openai_api/
│   ├── config/
│   │   └── config.yaml.example  # 配置文件示例
│   ├── utils/
│   │   ├── client_handler.py    # 客户端匹配逻辑
│   │   ├── db_handler.py        # 数据库处理
│   │   ├── log.py               # 日志配置
│   │   └── request_handler.py   # 请求预处理
│   ├── __init__.py
│   ├── config.py                # 配置加载模块
│   ├── main.py                  # FastAPI 主应用
│   └── run.sh                   # 启动脚本
├── requirements.txt             # 依赖
└── Dockerfile                   # Docker 配置
```

## 🤖 CI/CD (GitHub Actions)

本仓库已配置 GitHub Actions 工作流 (`.github/workflows/ci.yml`)，功能包括：

1.  **代码检查 (Linting)**: 在 `push` 和 `pull_request` 时自动使用 `flake8` 检查代码规范。
2.  **构建并推送 Docker 镜像**: 当代码合并到 `main` 分支时，会自动构建 Docker 镜像并将其推送到 GitHub Container Registry (GHCR)。

要使其生效，您需要在仓库的 `Settings > Secrets and variables > Actions` 中配置以下 secret：
- `DOCKERHUB_USERNAME`: 您的 Docker Hub 用户名（或 GHCR 等其他 registry 用户名）。
- `DOCKERHUB_TOKEN`: 您的 Docker Hub 访问令牌。

---