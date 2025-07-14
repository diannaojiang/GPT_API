#!/usr/bin/env python
# -*- encoding: utf-8 -*-

import os
from loguru import logger
import sys

# --- 全局日志配置 ---

# 1. 首先移除所有默认的处理器，以便进行完全自定义配置
logger.remove()

# 2. 添加一个控制台输出，方便在运行时实时查看日志，级别设置为INFO
logger.add(
    sys.stdout,
    level="INFO",
    format="<green>{time:YYYY-MM-DD HH:mm:ss}</green> | <level>{level: <8}</level> | <cyan>{name}:{function}:{line}</cyan> - <level>{message}</level>"
)

# 3. 创建日志文件存放目录
log_path = os.path.join(os.getcwd(), 'logs')
if not os.path.exists(log_path):
    os.mkdir(log_path)

# --- 文件日志输出配置 ---

# 4. 配置普通日志文件 (用于 INFO, WARNING 等)
#    - 输出到 openai-api.log
#    - 使用 filter 精确控制，只记录非 ERROR 级别的日志
#    - 实现基于文件大小的轮换和基于时间的保留策略
info_log_path = os.path.join(log_path, 'openai-api.log')
logger.add(
    info_log_path,
    level="INFO",
    filter=lambda record: record["level"].name != "ERROR",  # 关键：过滤掉ERROR日志
    rotation="100 MB",  # 当文件达到 100 MB 时，创建新文件
    retention="10 days",  # 最多保留最近 10 天的日志文件
    enqueue=True,  # 异步写入，提升性能
    encoding='utf-8',
    format="{time:YYYY-MM-DD HH:mm:ss} {level} | {module}.{function} | {message}"
)

# 5. 配置错误日志文件 (仅用于 ERROR)
#    - 输出到 error_{time:YYYY-MM-DD}.log
#    - level='ERROR' 确保只捕获 ERROR 及以上级别的日志
#    - 实现按天轮换和更长的保留策略
error_log_path = os.path.join(log_path, 'error_{time:YYYY-MM-DD}.log')
logger.add(
    error_log_path,
    level="ERROR",
    rotation="1 day",  # 每天 0 点创建新文件
    retention="30 days",  # 保留最近 30 天的错误日志
    enqueue=True,
    encoding='utf-8',
    format="{time:YYYY-MM-DD HH:mm:ss} {level} From {module}.{function} : {message}"
)