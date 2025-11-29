# Build Stage
FROM rust:latest as builder

WORKDIR /usr/src/app
COPY . .
RUN cargo build --release

# Runtime Stage
FROM debian:bookworm-slim

# 设置时区为 Asia/Shanghai
ENV TZ=Asia/Shanghai

# 安装必要的系统依赖
# reqwest (使用 rustls) 通常不需要 OpenSSL 开发库，但 ca-certificates 是必须的
# 同时安装 tzdata 以支持时区设置
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    tzdata \
    && ln -snf /usr/share/zoneinfo/$TZ /etc/localtime && echo $TZ > /etc/timezone \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# 从 builder 阶段复制二进制文件
COPY --from=builder /usr/src/app/target/release/gpt_api ./gpt_api

# 复制配置目录（如果运行时需要）
# 注意：Config map 可能会在部署时挂载，但这里提供默认结构
COPY config ./config
# 确保日志目录存在
RUN mkdir -p logs

# 设置数据库路径环境变量
ENV RECD_PATH="sqlite:./logs/record.db"

# 暴露端口
EXPOSE 8000

# 启动命令
CMD ["./gpt_api"]
