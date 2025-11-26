# GPT_API Rust 重构任务清单 (Checklist)

此文件将详细工作计划分解为可跟踪的、原子化的任务项。

## 阶段一：环境搭建、核心框架与基础 API

### 项目初始化与配置
- [x] 使用 `cargo new gpt_api_rust` 创建项目。
- [x] 配置 `Cargo.toml`，添加核心依赖：
  - [x] Web: `axum`, `tokio` (full features)
  - [x] 序列化: `serde`, `serde_json`, `serde_yaml`
  - [x] HTTP 客户端: `reqwest` (features: `json`, `stream`)
  - [x] 日志: `tracing`, `tracing-subscriber`
  - [x] 配置热重载: `notify`
  - [x] 数据校验: `validator`
  - [x] 数据库: `sqlx` (features: `runtime-tokio-rustls`, `sqlite`)

### 配置管理
- [x] 定义 `struct ClientConfig`，映射 `config.yaml` 中的单个客户端配置。
- [x] 定义 `struct Config`，包含 `check_config` 和 `openai_clients` 列表。
- [x] 实现 `load_config` 函数，从 `config.yaml` 文件加载配置。
- [x] 实现 `AppState` 结构来存储全局共享状态（如配置、数据库连接池）。

### 基础 Web 服务器
- [x] 使用 `Axum` 搭建服务器，监听指定端口。
- [x] 实现 `GET /health` 端点，返回 `{"status": "ok"}`。
- [x] 实现 `GET /v1/models` 端点（占位符，后续完善）。
- [x] 实现 `GET /v1/completions` 端点（占位符，后续完善）。
- [x] 实现 `GET /v1/chat/completions` 端点（占位符，后续完善）。

### 日志系统
- [x] 集成 `tracing`，配置日志级别、格式和输出目标（控制台、文件）。

### 配置热重载
- [x] 使用 `notify` crate 监控 `config.yaml` 文件变化。
- [x] 当文件变化时，重新加载配置并更新 `AppState` 中的配置引用（需考虑线程安全，如使用 `Arc<RwLock<Config>>`）。

## 阶段二：客户端管理与路由逻辑

### 客户端管理器
- [x] 创建 `ClientManager` 结构，负责根据 `ClientConfig` 列表初始化和存储 `reqwest::Client` 实例。
- [x] 实现 `get_clients_for_model` 函数，封装查找和排序逻辑。

### 路由逻辑实现
- [x] **`find_matching_clients`**: 根据 `model_match` 的 `type` (keyword/exact) 和 `value` 查找匹配的客户端配置。
- [x] **`select_clients_by_weight`**: 实现加权随机算法（如 Efraimidis-Spirakis A-Res 算法），根据 `priority` 对匹配的客户端进行排序。

### 集成到 `/v1/models`
- [x] 完善 `GET /v1/models` 端点，使其能够并发地从所有已配置的后端客户端获取模型列表，并聚合返回。

## 阶段三：请求预处理与主处理逻辑 (上)

### 数据结构定义
- [x] 使用 `serde` 定义 `ChatCompletionRequest`, `CompletionRequest`, `Message`, `Tool` 等结构体，用于解析和操作请求/响应数据。

### 请求预处理 (`request_handler.rs`)
- [x] **`process_messages`**: 实现消息清理（`strip` 空白字符）和合并（连续 `user` 消息覆盖）。
- [x] **`filter_empty_messages`**: 实现空消息过滤。
- [x] **`remove_think_tags`**: 实现 `assistant` 消息中 `<tool_call>` 标签的移除。

### 参数处理
- [x] 在主处理函数中，集成 `max_tokens` 调整和 `stop` 参数合并逻辑。

### 非流式主处理逻辑 (`api_handler.rs`)
- [x] 实现 `handle_chat_completion` 和 `handle_completion` 函数。
- [x] **客户端选择**: 调用 `get_clients_for_model`。
- [x] **API Key 透传**: 解析请求头，根据配置决定使用哪个 Key，并设置到 `reqwest::Client`。
- [x] **请求构建**: 将预处理后的数据序列化为 JSON，构建 `reqwest::Request`。
- [x] **后端调用**: 发送请求到选中的后端。
- [x] **响应处理**:
  - [x] 解析后端响应。
  - [x] 应用 `special_prefix`。
  - [x] 准备日志数据。
- [x] **错误处理与回退**: 捕获 `reqwest` 错误，如果配置了 `fallback`，则尝试调用备用模型。
- [x] **返回响应**: 将处理后的响应返回给客户端。

## 阶段四：流式处理与数据库日志

### 流式主处理逻辑 (`api_handler.rs`)
- [x] 实现 `handle_streaming_chat_completion` 和 `handle_streaming_completion` 函数。
- [x] **客户端选择与 API Key**: 同非流式。
- [x] **后端调用**: 发起流式请求，获取 `reqwest::Response`。
- [x] **流式响应生成**:
  - [x] 创建一个 `Stream` (如 `Pin<Box<dyn Stream<Item = Result<Bytes, Error>> + Send>>`)。
  - [x] 在流中，逐块读取后端响应。
  - [x] 解析 SSE 块，应用 `special_prefix` (仅在第一个有内容的块)。
  - [x] 收集流中的完整内容（用于日志）。
  - [x] 格式化为 SSE 并发送给客户端。
  - [x] 在流结束时，触发日志记录。

### 数据库集成 (`db_handler.rs`)
- [x] 配置 `sqlx`，设置 SQLite 数据库连接池。
- [x] 定义 `Record` 结构体，对应数据库表结构。
- [x] 实现 `log_request` 函数，将请求/响应数据和元数据（IP, Model, Tokens, Tool, Multimodal 等）插入数据库。
- [x] **数据库轮转**: 实现一个后台任务或在 `log_request` 中检查，按月对数据库文件进行重命名归档。

### 集成日志
- [x] 在非流式和流式请求处理的最后阶段，调用 `log_request` 函数。

## 阶段五：测试、验证与优化

### 集成测试
- [x] 修改 `GPT_API_TESTS/test_runner_enhanced.py`，使其能够编译并启动 Rust 版本的 `GPT_API` 二进制文件。
- [x] 运行所有功能测试 (`TestBasicFunctionality`, `TestRoutingStrategies`, `TestParameterHandling`, `TestRequestHandling`, `TestLogging`, `TestServiceHealth`, `TestLoadBalancing`)，确保 Rust 版本与 Python 版本行为一致。
- [x] 重点关注路由、参数处理、流式响应、日志记录等核心功能的测试通过率。

### 性能测试
- [x] 使用 `GPT_API_TESTS` 中的性能测试或 `wrk`/`hey` 等工具进行基准测试，与 Python 版本对比。

### 优化
- [x] 根据测试结果进行性能调优，如调整 `Tokio` 运行时配置、优化数据库查询等。

### 文档
- [x] 编写 `README.md`，说明如何构建、配置和运行 Rust 版本的 `GPT_API`。