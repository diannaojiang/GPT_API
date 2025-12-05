## 第二部分：项目开发文档 (DEVELOPER.md)

这份文档旨在帮助新加入的开发者快速理解 `GPT_API` 的代码结构、核心逻辑以及优化细节。

---

# GPT_API 开发者指南

## 1. 架构概览

GPT_API 采用分层架构设计，实现了关注点分离：

1.  **Routes Layer (`src/routes`)**: 这里的代码非常薄，仅负责 HTTP 请求的参数提取（Extractors）和类型校验。
2.  **Handlers Layer (`src/handlers`)**: 处理具体的业务逻辑（如 JSON 预处理、流式转换），但不包含路由决策。
3.  **Service Layer (`src/services`)**: **核心层**。`DispatcherService` 负责负载均衡、节点选择和故障转移循环。
4.  **Client Layer (`src/client`)**: 负责 HTTP 连接池管理和实际的网络 IO。
5.  **State Layer (`src/state`)**: 管理全局共享状态（配置、数据库连接池、服务实例）。

### 数据流向图
```text
Request -> [Router] -> [Handler (Common/Audio)] 
             |
             v
      [Dispatcher Service] <--(Reads Config)-- [Config Manager]
             |
        (Load Balance)
             |
             v
      [Client Manager] --(HTTP/Stream)--> [Upstream API]
```

## 2\. 目录结构解析

```bash
src/
├── main.rs                 # 程序入口，初始化日志、配置、数据库、mimalloc 和 HTTP Server
├── lib.rs                  # 模块导出定义
├── app_error.rs            # 全局错误处理 (统一转为 JSON Response)
├── client/                 # HTTP 客户端层
│   ├── client_manager.rs   # Reqwest Client 初始化 (KeepAlive, Timeout)
│   ├── proxy.rs            # 构建具体 Request (Header/Body 转发)
│   └── routing.rs          # 权重选择算法 (Weighted Round-Robin 变体)
├── config/                 # 配置管理层
│   ├── config_manager.rs   # 实现基于 notify 的配置文件热重载
│   └── types.rs            # 配置结构体定义
├── db/                     # 持久化层
│   ├── mod.rs              # SQLite 连接池初始化与文件轮转逻辑
│   └── records.rs          # 日志结构体定义与 Insert 操作
├── handlers/               # 业务处理层
│   ├── common_handler.rs   # 通用请求处理入口 (Chat/Completions/Embeddings)
│   ├── audio_handler.rs    # 音频专用处理 (Multipart Form 重组与缓存)
│   ├── stream_handler.rs   # SSE 流式响应解析 (使用 simd-json)
│   └── utils.rs            # 工具函数 (IP提取、正则清洗、JSON 转换)
├── middleware/             # Axum 中间件 (Access Log)
├── models/                 # 请求/响应数据模型 (RequestPayload Enum)
├── routes/                 # 路由定义 (Controller)
├── services/               # 核心服务层
│   └── dispatcher.rs       # 封装了 Retry/Fallback 逻辑的统一调度器
└── state/                  # AppState 定义
```

## 3\. 核心模块深度解析

### 3.1 统一调度器 (`services/dispatcher.rs`)

这是系统的“大脑”。为了解耦业务逻辑与路由逻辑，我们设计了 `DispatcherService`。

  * **设计模式**: 策略模式 + 责任链模式的变体。
  * **关键方法**: `execute<F, Fut>(&self, initial_model, request_callback)`
  * **工作原理**:
    1.  接受一个 `request_callback` 闭包（负责发送实际请求）。
    2.  根据 `initial_model` 查找可用后端列表。
    3.  执行负载均衡。
    4.  在一个 `loop` 中尝试调用后端。
    5.  如果失败（5xx错误），自动查找 `fallback` 模型并**更新当前模型名称**，进入下一次循环。
    6.  如果成功，直接返回 Response。

### 3.2 高性能流式处理 (`handlers/stream_handler.rs`)

针对 OpenAI 的 SSE (Server-Sent Events) 响应，我们进行了极致优化。

  * **simd-json**: 我们使用 `simd_json::from_str` 替代标准的 `serde_json` 来解析每一个 Event 数据块。
  * **Zero-Copy (尽力)**: 在处理 SSE 事件时，直接操作字节切片，仅在需要注入 `special_prefix` 时才进行 JSON 修改。

### 3.3 音频处理与重试 (`handlers/audio_handler.rs`)

`Multipart/form-data` 的重试是一个难点，因为 Stream 只能被消费一次。

  * **解决方案**: 定义了 `CachedPart` 结构体。
  * **逻辑**: 在 Handler 入口处将上传的文件流（Bytes）读取到内存中暂存。这使得在 `DispatcherService` 触发 Fallback 重试时，我们能够重新构建 `reqwest::multipart::Form` 发送给备用节点。
  * *注意*: 目前在大文件上传时可能会有内存压力，建议在反向代理层限制上传大小。

### 3.4 动态正则优化 (`handlers/utils.rs`)

为了处理 DeepSeek 等模型的 `<think>` 标签，我们使用了正则表达式。

  * **优化**: 使用 `once_cell::sync::Lazy` 声明全局静态 `Regex` 对象。
  * **收益**: 避免了在每次 HTTP 请求中重新编译正则表达式的昂贵开销（约节省 0.5ms - 2ms CPU 时间）。

## 4\. 性能优化细节

本项目在 Rust 代码层面集成了多项优化，参与开发时请务必保持这些特性：

1.  **全局内存分配器 (Mimalloc)**:
    在 `main.rs` 中配置了 `#[global_allocator]`。Mimalloc 在高并发小对象分配（如 JSON 处理）场景下性能远超系统默认的 malloc。

2.  **SIMD JSON**:
    所有涉及 JSON 解析的热点路径（尤其是 Response Body 解析）都使用了 `simd-json`。这利用了 CPU 的向量指令集（AVX2/NEON）。

3.  **异步日志**:
    日志写入操作通过 `tokio::spawn` 移交后台任务执行，不阻塞主请求线程返回 Response。

## 5\. 开发工作流

### 环境准备

确保安装了 Rust 工具链。

### 运行测试

```bash
# 运行所有单元测试
cargo test

# 运行特定模块测试
cargo test client::routing
```

### 代码格式化

提交代码前请务必运行：

```bash
cargo fmt
cargo clippy -- -D warnings
```

### 添加新的 API 类型

1.  在 `src/models/requests.rs` 的 `RequestPayload`枚举中添加新变体。
2.  在 `src/handlers/utils.rs` 的 `build_request_body_generic` 中添加构建逻辑。
3.  在 `src/handlers/common_handler.rs` 的 `dispatch_request` 中添加 endpoint 映射。
4.  在 `src/routes` 添加路由定义。
