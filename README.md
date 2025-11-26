# GPT API (Rust Version)

High-performance, OpenAI-compatible API Gateway written in Rust.

## Features
- **High Performance**: Built with Axum and Tokio for handling high concurrency.
- **Compatible**: Fully supports OpenAI Chat and Completion endpoints.
- **Advanced Routing**: Keyword matching, exact matching, load balancing, and fallback mechanisms.
- **Streaming**: Efficient Server-Sent Events (SSE) handling.
- **Observability**: Detailed logging to SQLite with automatic rotation.

## Docker Usage

```bash
docker run -d \
  -p 8000:8000 \
  -v $(pwd)/config:/app/config \
  -v $(pwd)/logs:/app/logs \
  ghcr.io/OWNER/openai-api:latest
```

```