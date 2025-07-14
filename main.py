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

app.add_middleware(
    CORSMiddleware,
    allow_origins=["*"],
    allow_credentials=True,
    allow_methods=["*"],
    allow_headers=["*"],
)

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

# 初始化客户端配置
clients = init_openai_clients()
client_configs = load_clients()

# --------API---------

@app.get("/")
def read_root(request: Request):
    return {"message": "Hello World", "client_ip": request.client.host}

@app.get("/models")
@app.get("/v1/models")
async def list_models():
    global clients, client_configs
    try:
        clients = init_openai_clients()
        client_configs = load_clients()
        logger.info(f"成功刷新模型和服务配置！")
    except Exception as e:
        logger.error(f"刷新配置文件时出现错误: {e}")

    unique_models = {}
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


