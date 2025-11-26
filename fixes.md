## 修复 `test_api_key_from_input_only` 测试项

### 问题分析
在 `try_chat_completion` 和 `try_completion` 函数中，获取 API Key 的逻辑如下：
```rust
let api_key = client_config.api_key.as_ref() 
    .map(|s| s.as_str())
    .or_else(|| headers.get("Authorization")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer ")))
    .unwrap_or("");
```
这个逻辑会优先使用配置文件中的 API Key，如果没有配置，则从请求头中获取。但是，当配置文件中没有提供 API Key 时，从请求头获取的 API Key 会包含 "Bearer " 前缀，这导致 mock server 收到的 API Key 是 "Bearer key-from-input-only" 而不是 "key-from-input-only"。

### 解决方案
修改获取 API Key 的逻辑，确保从请求头中提取的 API Key 不包含 "Bearer " 前缀。

### 修复步骤
1. 修改 `try_chat_completion` 函数中的 API Key 获取逻辑。
2. 修改 `try_completion` 函数中的 API Key 获取逻辑。
3. 修改 `try_streaming_chat_completion` 函数中的 API Key 获取逻辑。
4. 修改 `try_streaming_completion` 函数中的 API Key 获取逻辑。