#!/usr/bin/env python
# -*- encoding: utf-8 -*-
"""
@File    :   main.py
@Contact :   55k@163.com

@Modify Time      @Author    @Version    @Description
------------      -------    --------    -----------
2023/11/20 12:45   刘行健      1.0         None
2025/01/08 12:31   刘行健      2.0         None
2025/02/12 21:41   刘行健      2.1         支持R1
2025/02/17 17:30   刘明达      2.2         支持R1后处理
2025/02/23 12:13   刘行健      2.3         支持R1-AWQ
2025/03/07 18:33   刘行健      2.4         修复空内容报错
2025/03/07 20:03   刘行健      2.5         支持容错自切换
2025/03/22 23:23   刘行健      3.0         完成配置文件分离
2025/04/15 14:03   刘行健      3.1         支持预设停止词
2025/05/15 19:16   刘行健      3.2         支持工具调用
2025/05/19 18:19   刘行健      3.3         修复工具调用
2025/05/20 10:31   刘行健      3.4         修复工具调用
2025/07/03 16:00   Gemini      3.5         支持v1/models接口
2025/07/03 16:55   刘行健      3.6         支持模型配置刷新
2025/07/04 12:44   刘行健      3.7         支持跨域请求配置
2025/07/11 18:00   Gemini      4.0         实现数据库按月自动归档
2025/07/11 18:30   Gemini      4.1         代码模块化重构
2025/07/11 19:00   Gemini      4.2         增加/completions接口支持
2025/07/14 11:40   刘行健      4.3         数据库区分completions接口及添加tool字段
2025/07/15 14:00   Gemini      4.4         扩展chat completion参数并支持多模态
2025/07/15 14:35   Gemini      4.5         增加多模态请求数据库记录
"""

# 标准库导入
import re
import traceback
import time
import random
import string
import sys
import os
import uvicorn
import json

# 第三方库导入
from fastapi import FastAPI, HTTPException, Request
from fastapi.responses import StreamingResponse, JSONResponse
from fastapi.middleware.cors import CORSMiddleware
from pydantic import BaseModel
from openai import APIError

# 本地应用导入
from utils.log import logger
from utils.db_handler import log_request
from utils.client_handler import find_matching_client
from utils.request_handler import process_messages
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
    try:
        model = post_data.get("model")
        try:
            client, cfg = find_matching_client(model, client_configs, clients)
        except ValueError as e:
            return JSONResponse(status_code=404, content={"object": "error", "message": str(e), "type": "NotFoundError"})

        allowed_keys = {
            "model", "prompt", "best_of", "echo", "frequency_penalty", "logit_bias", "logprobs",
            "max_tokens", "n", "presence_penalty", "seed", "stop", "stream", "stream_options",
            "suffix", "temperature", "top_p", "user"
        }
        api_post_data = {key: value for key, value in post_data.items() if key in allowed_keys}

        if cfg.stop:
            if "stop" in api_post_data and api_post_data["stop"]:
                existing_stop = api_post_data["stop"]
                if not isinstance(existing_stop, list): existing_stop = [existing_stop]
                cfg_stop = cfg.stop if isinstance(cfg.stop, list) else [cfg.stop]
                api_post_data["stop"] = list(set(existing_stop + cfg_stop))
            else:
                api_post_data["stop"] = cfg.stop

        try:
            response = client.completions.create(**api_post_data)
        except APIError as e:
            if cfg.fallback:
                logger.warning(f"Initial request failed: {e}, retrying with fallback model {cfg.fallback}...")
                model = cfg.fallback
                del client, cfg
                client, cfg = find_matching_client(model, client_configs, clients)
                fallback_post_data = api_post_data.copy()
                fallback_post_data["model"] = model
                response = client.completions.create(**fallback_post_data)
            else:
                raise e

        client_ip = request.client.host
        request_payload_for_log = {"prompt": api_post_data.get("prompt")}

        if api_post_data.get("stream", False):
            async def stream_response():
                full_text_parts = []
                chunk_buffer = []
                try:
                    for chunk in response:
                        chunk_buffer.append(chunk)
                        if chunk.choices and chunk.choices[0].text:
                            full_text_parts.append(chunk.choices[0].text)
                        yield f"data: {json.dumps(chunk.to_dict())}\n\n"

                    if chunk_buffer:
                        final_chunk_for_log = chunk_buffer[-1]
                        final_chunk_for_log.choices[0].text = "".join(full_text_parts)
                        log_request(client_ip, model, request_payload_for_log, final_chunk_for_log,
                                    request_type=final_chunk_for_log.object)

                    yield "data: [DONE]\n\n"
                except Exception as e:
                    logger.error(traceback.format_exc())
                    yield f"data: {json.dumps({'error': str(e)})}\n\n"
            return StreamingResponse(stream_response(), media_type="text/event-stream")
        else:
            log_request(client_ip, model, request_payload_for_log, response, request_type=response.object)
            return response

    except APIError as e:
        return JSONResponse(status_code=e.status_code, content=e.body)
    except Exception as e:
        logger.error(traceback.format_exc())
        return JSONResponse(status_code=500, content={"object": "error", "message": str(e), "type": "InternalError"})


@app.post("/chat/completions")
@app.post("/v1/chat/completions")
async def completions(post_data: dict, request: Request):
    """处理新版 /v1/chat/completions (聊天) 接口请求"""
    try:
        model = post_data.get("model")
        try:
            client, cfg = find_matching_client(model, client_configs, clients)
        except ValueError as e:
            model_name = str(e).split("model ")[-1].strip()
            return JSONResponse(status_code=404, content={"object": "error", "message": f"The model `{model_name}` does not exist.", "type": "NotFoundError"})

        if cfg.max_tokens:
            post_data["max_tokens"] = min(post_data.get("max_tokens", cfg.max_tokens), cfg.max_tokens)

        is_multimodal = False
        if "messages" in post_data:
            messages = post_data["messages"]
            messages_fix = []
            for message in messages:
                content = message.get("content")
                if content is None:
                    if "tool_calls" in message:
                        pass
                    else:
                        raise ValueError("未找到 'content'")
                else:
                    if isinstance(content, str):
                        if len(content) == 0:
                            continue
                        if message.get("role") == "assistant":
                            message["content"] = re.sub(r'<think>.*?</think>', '', content, flags=re.DOTALL)
                    elif isinstance(content, list):
                        # 检查是否存在非文本类型的content block
                        for item in content:
                            if isinstance(item, dict) and item.get("type") != "text":
                                is_multimodal = True
                                break
                messages_fix.append(message)
                if is_multimodal:
                    break  # 如果已确认是多模态，无需再检查
            post_data["messages"] = process_messages(messages_fix)

        allowed_keys = {
            "model", "messages", "max_tokens", "stream", "temperature", "top_p",
            "presence_penalty", "frequency_penalty", "stop", "tool_choice", "tools",
            "audio", "function_call", "functions", "logit_bias", "logprobs",
            "max_completion_tokens", "metadata", "modalities", "n",
            "parallel_tool_calls", "prediction", "reasoning_effort",
            "response_format", "seed", "service_tier", "store", "stream_options",
            "top_logprobs", "user", "web_search_options"
        }
        api_post_data = {key: value for key, value in post_data.items() if key in allowed_keys}

        tools_used = "tools" in api_post_data and api_post_data["tools"] is not None

        if cfg.stop:
            if "stop" in api_post_data and api_post_data["stop"]:
                existing_stop = api_post_data["stop"]
                if not isinstance(existing_stop, list): existing_stop = [existing_stop]
                cfg_stop = cfg.stop if isinstance(cfg.stop, list) else [cfg.stop]
                api_post_data["stop"] = list(set(existing_stop + cfg_stop))
            else:
                api_post_data["stop"] = cfg.stop

        try:
            response = client.chat.completions.create(**api_post_data)
        except APIError as e:
            if cfg.fallback:
                logger.warning(f"Initial request failed: {e}, retrying with fallback model {cfg.fallback}...")
                model = cfg.fallback
                del client, cfg
                client, cfg = find_matching_client(model, client_configs, clients)
                fallback_post_data = api_post_data.copy()
                fallback_post_data["model"] = model
                response = client.chat.completions.create(**fallback_post_data)
            else:
                raise e

        client_ip = request.client.host
        request_payload_for_log = {"messages": api_post_data.get("messages", [])}

        if api_post_data.get("stream", False):
            async def stream_response():
                content = ""
                chunk_buffer = []
                first_token = True
                try:
                    for chunk in response:
                        chunk_buffer.append(chunk)
                        if not chunk.choices or len(chunk.choices) == 0:
                            yield f"data: {json.dumps(chunk.to_dict())}\n\n"
                            continue
                        delta_content = chunk.choices[0].delta.content
                        if delta_content:
                            if first_token and cfg.special_prefix:
                                chunk.choices[0].delta.content = f"{cfg.special_prefix}\n{delta_content}"
                                content += delta_content
                                first_token = False
                            else:
                                content += delta_content
                        yield f"data: {json.dumps(chunk.to_dict())}\n\n"

                    if chunk_buffer:
                        final_chunk = chunk_buffer[-1]
                        if not final_chunk.choices or len(final_chunk.choices) == 0:
                            if len(chunk_buffer) > 1: final_chunk.choices = chunk_buffer[-2].choices
                            else:
                                class MockChoice: message = {}
                                final_chunk.choices = [MockChoice()]
                        final_chunk.choices[0].message = {"content": content, "role": chunk_buffer[0].choices[0].delta.role or "assistant"}
                        log_request(client_ip, model, request_payload_for_log, final_chunk,
                                    request_type=final_chunk.object, tools_used=tools_used, is_multimodal=is_multimodal)

                    yield "data: [DONE]\n\n"
                except Exception as e:
                    logger.error(traceback.format_exc())
                    yield f"data: {json.dumps({'error': str(e)})}\n\n"
            return StreamingResponse(stream_response(), media_type="text/event-stream")
        else:
            response_data = response
            if response_data and len(response_data.choices) > 0 and cfg.special_prefix:
                response_data.choices[0].message.content = f"{cfg.special_prefix}\n" + response_data.choices[0].message.content
            log_request(client_ip, model, request_payload_for_log, response_data,
                        request_type=response_data.object, tools_used=tools_used, is_multimodal=is_multimodal)
            return response_data

    except APIError as e:
        return JSONResponse(status_code=e.status_code, content=e.body)
    except Exception as e:
        logger.error(traceback.format_exc())
        return JSONResponse(status_code=500, content={"object": "error", "message": str(e), "type": "InternalError"})

if __name__ == '__main__':
    uvicorn.run(app, host="0.0.0.0", port=7000, log_config=None)
