# utils/api_handler.py

import json
import traceback
import re
from typing import AsyncGenerator, Tuple

from fastapi import Request
from fastapi.responses import StreamingResponse, JSONResponse
from openai import APIError

from utils.log import logger
from utils.db_handler import log_request
from utils.client_handler import find_matching_client
from utils.request_handler import process_messages
from config import ClientConfig

async def handle_api_request(
    request: Request,
    post_data: dict,
    clients: dict,
    client_configs: list[ClientConfig],
    api_type: str  # "completions" or "chat.completions"
) -> StreamingResponse | JSONResponse:
    """
    处理通用的API请求逻辑，包括客户端查找、请求参数处理、API调用、错误处理和日志记录。
    """
    try:
        model = post_data.get("model")
        try:
            client, cfg = find_matching_client(model, client_configs, clients)
        except ValueError as e:
            # 对于聊天接口，提供更友好的模型不存在提示
            if api_type == "chat.completions":
                model_name = str(e).split("model ")[-1].strip()
                return JSONResponse(status_code=404, content={"object": "error", "message": f"The model `{model_name}` does not exist.", "type": "NotFoundError"})
            else:
                return JSONResponse(status_code=404, content={"object": "error", "message": str(e), "type": "NotFoundError"})

        api_post_data = post_data.copy()

        # 处理max_tokens
        if api_type == "chat.completions" and cfg.max_tokens:
            api_post_data["max_tokens"] = min(api_post_data.get("max_tokens", cfg.max_tokens), cfg.max_tokens)

        # 处理messages和多模态判断
        is_multimodal = False
        if api_type == "chat.completions" and "messages" in api_post_data:
            messages = api_post_data["messages"]
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
                        for item in content:
                            if isinstance(item, dict) and item.get("type") != "text":
                                is_multimodal = True
                                break
                messages_fix.append(message)
                if is_multimodal:
                    break
            api_post_data["messages"] = process_messages(messages_fix)

        # 处理stop参数
        if cfg.stop:
            if "stop" in api_post_data and api_post_data["stop"]:
                existing_stop = api_post_data["stop"]
                if not isinstance(existing_stop, list): existing_stop = [existing_stop]
                cfg_stop = cfg.stop if isinstance(cfg.stop, list) else [cfg.stop]
                api_post_data["stop"] = list(set(existing_stop + cfg_stop))
            else:
                api_post_data["stop"] = cfg.stop

        # 移除不允许的键
        if api_type == "completions":
            allowed_keys = {
                "model", "prompt", "best_of", "echo", "frequency_penalty", "logit_bias", "logprobs",
                "max_tokens", "n", "presence_penalty", "seed", "stop", "stream", "stream_options",
                "suffix", "temperature", "top_p", "user"
            }
        else: # chat.completions
            allowed_keys = {
                "model", "messages", "max_tokens", "stream", "temperature", "top_p",
                "presence_penalty", "frequency_penalty", "stop", "tool_choice", "tools",
                "audio", "function_call", "functions", "logit_bias", "logprobs",
                "max_completion_tokens", "metadata", "modalities", "n",
                "parallel_tool_calls", "prediction", "reasoning_effort",
                "response_format", "seed", "service_tier", "store", "stream_options",
                "top_logprobs", "user", "web_search_options"
            }
        api_post_data = {key: value for key, value in api_post_data.items() if key in allowed_keys}

        tools_used = "tools" in api_post_data and api_post_data["tools"] is not None

        try:
            if api_type == "completions":
                response = client.completions.create(**api_post_data)
            else:
                response = client.chat.completions.create(**api_post_data)
        except APIError as e:
            if cfg.fallback:
                logger.warning(f"Initial request failed: {e}, retrying with fallback model {cfg.fallback}...")
                fallback_post_data = api_post_data.copy()
                fallback_post_data["model"] = cfg.fallback
                # 重新查找客户端和配置
                client, cfg = find_matching_client(cfg.fallback, client_configs, clients)
                if api_type == "completions":
                    response = client.completions.create(**fallback_post_data)
                else:
                    response = client.chat.completions.create(**fallback_post_data)
            else:
                raise e

        client_ip = request.client.host
        auth_header = request.headers.get("Authorization")

        if api_post_data.get("stream", False):
            return StreamingResponse(stream_response_generator(response, client_ip, model, api_post_data, auth_header, api_type, tools_used, is_multimodal, cfg.special_prefix), media_type="text/event-stream")
        else:
            response_data = response
            if api_type == "chat.completions" and response_data and len(response_data.choices) > 0 and cfg.special_prefix:
                response_data.choices[0].message.content = f"{cfg.special_prefix}\n" + response_data.choices[0].message.content
            log_request(client_ip, model, api_post_data, response_data, auth_header,
                        request_type=response_data.object, tools_used=tools_used, is_multimodal=is_multimodal)
            return response_data

    except APIError as e:
        return JSONResponse(status_code=e.status_code, content=e.body)
    except Exception as e:
        logger.error(traceback.format_exc())
        return JSONResponse(status_code=500, content={"object": "error", "message": str(e), "type": "InternalError"})

async def stream_response_generator(
    response: AsyncGenerator, # 虽然类型提示是 AsyncGenerator，但实际传入的是 Stream 对象
    client_ip: str,
    model: str,
    request_payload: dict,
    auth_header: str,
    api_type: str,
    tools_used: bool,
    is_multimodal: bool,
    special_prefix: str
) -> AsyncGenerator[str, None]:
    """
    生成流式响应的异步生成器，并处理日志记录。
    """
    full_content_parts = [] # For chat completions
    full_text_parts = []    # For legacy completions
    chunk_buffer = []
    first_token = True

    try:
        # ✅ *** 主要修改点在这里 ***
        for chunk in response: # <--- 将 'async for' 改为 'for'
            chunk_buffer.append(chunk)
            if api_type == "completions":
                if chunk.choices and chunk.choices[0].text:
                    full_text_parts.append(chunk.choices[0].text)
            else: # chat.completions
                if chunk.choices and len(chunk.choices) > 0 and chunk.choices[0].delta.content:
                    delta_content = chunk.choices[0].delta.content
                    if first_token and special_prefix:
                        # 这里有一个小问题，修改 delta.content 会直接影响 chunk 对象
                        # 更好的做法是创建一个新的字典来发送
                        modified_chunk_dict = chunk.to_dict()
                        modified_chunk_dict["choices"][0]["delta"]["content"] = f"{special_prefix}\n{delta_content}"
                        yield f"data: {json.dumps(modified_chunk_dict)}\n\n"
                        full_content_parts.append(delta_content) # 记录原始 content
                        first_token = False
                        continue # 跳过下面的 yield
                    else:
                        full_content_parts.append(delta_content)
            yield f"data: {json.dumps(chunk.to_dict())}\n\n"

        if chunk_buffer:
            final_chunk = chunk_buffer[-1]
            # ... 后续的日志记录逻辑保持不变 ...
            if api_type == "completions":
                if final_chunk.choices:
                    final_chunk.choices[0].text = "".join(full_text_parts)
                log_request(client_ip, model, request_payload, final_chunk, auth_header,
                            request_type=final_chunk.object)
            else: # chat.completions
                if not final_chunk.choices or len(final_chunk.choices) == 0:
                    if len(chunk_buffer) > 1: 
                        final_chunk.choices = chunk_buffer[-2].choices
                    else:
                        class MockChoice:
                            def __init__(self):
                                self.message = {}
                        final_chunk.choices = [MockChoice()]
                
                if not hasattr(final_chunk.choices[0], 'message'):
                    final_chunk.choices[0].message = {}

                final_chunk.choices[0].message["content"] = "".join(full_content_parts)
                # 确保 delta 对象存在且不为空
                if chunk_buffer[0].choices and chunk_buffer[0].choices[0].delta:
                    final_chunk.choices[0].message["role"] = chunk_buffer[0].choices[0].delta.role or "assistant"
                else:
                    final_chunk.choices[0].message["role"] = "assistant"
                log_request(client_ip, model, request_payload, final_chunk, auth_header,
                            request_type=final_chunk.object, tools_used=tools_used, is_multimodal=is_multimodal)

        yield "data: [DONE]\n\n"
    except Exception as e:
        logger.error(traceback.format_exc())
        yield f"data: {json.dumps({'error': str(e)})}\n\n"