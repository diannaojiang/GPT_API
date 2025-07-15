#!/usr/bin/env python
# -*- encoding: utf-8 -*-
# main.py

# 标准库导入
import time
import random
import string
import sys
import os
import uvicorn
import signal  # signal模块仍然保留，以备将来使用，但当前逻辑不使用它
import pathlib # 导入pathlib用于处理文件路径

# 第三方库导入
from fastapi import FastAPI, Request
from fastapi.responses import JSONResponse
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel

# 本地应用导入
from utils.log import logger
from utils.api_handler import handle_api_request
from config import load_clients, init_openai_clients

BASE_DIR = os.path.dirname(os.path.abspath(__file__))
sys.path.append(os.path.join(BASE_DIR, 'algo_sdk'))
app = FastAPI()

# -------- 新增：配置热重载逻辑 --------

# 每个工作进程都会有自己独立的全局变量副本
RELOAD_TRIGGER_FILE = pathlib.Path(os.path.join(BASE_DIR, "reload.trigger"))
last_config_reload_time = 0.0  # 上次加载配置的时间戳
clients = {}
client_configs = {}

def perform_config_reload():
    """
    为当前工作进程执行实际的配置重载逻辑。
    """
    global clients, client_configs, last_config_reload_time
    # logger.info(f"进程 {os.getpid()}: 检测到配置更新信号，正在重新加载...")
    try:
        # 重新初始化客户端和配置
        new_clients = init_openai_clients()
        new_client_configs = load_clients()
        clients = new_clients
        client_configs = new_client_configs
        
        # 更新本进程的最后加载时间为触发文件的修改时间
        last_config_reload_time = RELOAD_TRIGGER_FILE.stat().st_mtime
        logger.info(f"进程 {os.getpid()}: 配置加载成功。")
    except Exception as e:
        logger.error(f"进程 {os.getpid()}: 加载配置时出错: {e}")

@app.on_event("startup")
async def startup_event():
    """
    应用启动时执行的事件。
    1. 确保触发文件存在。
    2. 执行一次初始的配置加载。
    """
    RELOAD_TRIGGER_FILE.touch(exist_ok=True)
    perform_config_reload()

@app.middleware("http")
async def check_reload_middleware(request: Request, call_next):
    """
    在每个请求前检查是否需要重载配置的中间件。
    """
    global last_config_reload_time
    try:
        # 文件系统stat调用非常快，对性能影响可以忽略不计
        if RELOAD_TRIGGER_FILE.stat().st_mtime > last_config_reload_time:
            perform_config_reload()
    except FileNotFoundError:
        logger.warning(f"重载触发文件 '{RELOAD_TRIGGER_FILE}' 未找到。将重新创建并重载。")
        RELOAD_TRIGGER_FILE.touch(exist_ok=True)
        perform_config_reload()
    except Exception as e:
        # 即使检查失败，也应继续处理请求
        logger.error(f"检查配置重载时发生意外错误: {e}")
    
    response = await call_next(request)
    return response

# -------- 热重载逻辑结束 --------


app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

# 注意：上面的check_reload_middleware会先于下面的log_requests执行
@app.middleware("http")
async def log_requests(request, call_next):
    idem = ''.join(random.choices(string.ascii_uppercase + string.digits, k=6))
    logger.info(f"rid={idem} client_host={request.client.host} start request path={request.url.path}")
    start_time = time.time()
    response = await call_next(request)
    process_time = (time.time() - start_time) * 1000
    formatted_process_time = f"{process_time:.2f}"
    logger.info(f"rid={idem} completed_in={formatted_process_time}ms status_code={response.status_code}")
    return response

class Item(BaseModel):
    text: str

# 初始化的逻辑已移至 startup_event 中，此处不再需要
# clients = init_openai_clients()
# client_configs = load_clients()

# --------API---------

@app.get("/")
def read_root(request: Request):
    return {"message": "Hello World", "client_ip": request.client.host}

@app.get("/health", status_code=200)
async def health_check():
    """
    提供给Docker HEALTHCHECK的健康检查接口。
    """
    return {"status": "ok"}

@app.get("/models")
@app.get("/v1/models")
async def list_models():
    """
    列出所有可用的模型，并通过更新触发文件来通知所有工作进程在下次请求时重新加载配置。
    """
    try:
        # 1. 更新触发文件的修改时间，以此向所有工作进程发送“重载”信号
        logger.info(f"进程 {os.getpid()}: 收到/models请求，已发出全局配置重载信号。")
        RELOAD_TRIGGER_FILE.touch(exist_ok=True)

        # 2. 为确保本次请求能立即返回最新的模型列表，强制当前进程也重载一次
        perform_config_reload()

    except Exception as e:
        logger.error(f"刷新配置文件时出现错误: {e}")
        return JSONResponse(status_code=500, content={"error": f"触发重载失败: {e}"})

    # 3. 收集并返回当前（已更新）工作进程的模型列表
    unique_models = {}
    # 使用全局的clients变量，它已经被perform_config_reload更新了
    for client_name, client in clients.items():
        try:
            models_list = client.models.list()
            for model in models_list.data:
                unique_models[model.id] = model.model_dump()
        except Exception as e:
            logger.error(f"从客户端 '{client_name}' ({client.base_url}) 获取模型时发生错误: {e}")
            continue

    return JSONResponse(content={"object": "list", "data": list(unique_models.values())})

@app.post("/completions")
@app.post("/v1/completions")
async def legacy_completions(post_data: dict, request: Request):
    """处理旧版 /v1/completions (文本补全) 接口请求"""
    return await handle_api_request(request, post_data, clients, client_configs, "completions")

@app.post("/chat/completions")
@app.post("/v1/chat/completions")
async def completions(post_data: dict, request: Request):
    """处理新版 /v1/chat/completions (聊天) 接口请求"""
    return await handle_api_request(request, post_data, clients, client_configs, "chat.completions")

if __name__ == '__main__':
    uvicorn.run(app, host="0.0.0.0", port=7000, log_config=None)