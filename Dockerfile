# Dockerfile

# --- Stage 1: Builder ---
# 使用一个完整的 Python 镜像来编译依赖，这可以处理需要编译的包
FROM python:3.9-bookworm AS builder

WORKDIR /usr/src/app

# 更新 apt-get 并安装构建工具
RUN apt-get update && apt-get install -y --no-install-recommends build-essential

# 安装 Poetry (可选，但推荐用于依赖管理) 或直接使用 pip
# RUN pip install poetry
# COPY poetry.lock pyproject.toml ./
# RUN poetry export -f requirements.txt --output requirements.txt --without-hashes

# 复制需求文件并安装，利用层缓存
COPY requirements.txt .
RUN pip wheel --no-cache-dir --no-deps --wheel-dir /usr/src/app/wheels -r requirements.txt


# --- Stage 2: Final Image ---
# 使用一个更轻量的基础镜像
FROM python:3.9-slim-bookworm
# 设置工作目录
WORKDIR /app

# 从 builder 阶段复制编译好的 wheels 并安装
# 这样可以避免在最终镜像中保留构建工具
COPY --from=builder /usr/src/app/wheels /wheels
COPY --from=builder /usr/src/app/requirements.txt .
RUN pip install --no-cache /wheels/*

# 配置环境变量（路径自动适配新目录）
ENV RECD_PATH=logs/record.db

# 分步复制文件以利用构建缓存
COPY *.py ./
COPY utils/ ./utils
COPY algo_sdk/ ./algo_sdk  
# 新增的算法SDK目录复制

# 创建标准目录结构
RUN mkdir -p logs config

# 暴露端口
EXPOSE 8000

# 启动命令
# 使用 log_config=None 来让 loguru 接管日志格式
CMD ["uvicorn", "main:app", "--host", "0.0.0.0", "--port", "8000", "--proxy-headers", "--workers", "128"]
