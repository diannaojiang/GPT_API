# utils/client_handler.py

from typing import Tuple
from openai import OpenAI
from config import ClientConfig

def find_matching_client(model_name: str, client_configs: list[ClientConfig], clients: dict) -> Tuple[OpenAI, ClientConfig]:
    """
    根据模型名称，从已加载的配置中查找匹配的OpenAI客户端。

    Args:
        model_name: 请求中指定的模型名称。
        client_configs: 从配置文件加载的所有客户端配置列表。
        clients: 已初始化的所有OpenAI客户端实例字典。

    Raises:
        ValueError: 如果没有找到与模型匹配的客户端。

    Returns:
        一个包含匹配的客户端实例和其配置的元组。
    """
    for cfg in client_configs:
        # 精确匹配
        if cfg.model_match["type"] == "exact" and model_name in cfg.model_match["value"]:
            return clients[cfg.name], cfg
        # 关键字匹配
        elif cfg.model_match["type"] == "keyword" and any(kw in model_name for kw in cfg.model_match["value"]):
            return clients[cfg.name], cfg
    raise ValueError(f"No matching client for model {model_name}")