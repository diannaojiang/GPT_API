# utils/client_handler.py

import random
from typing import List, Tuple
from openai import OpenAI
from config import ClientConfig

def find_matching_clients(model_name: str, client_configs: list[ClientConfig]) -> List[ClientConfig]:
    """
    根据模型名称查找所有匹配的客户端配置。
    """
    matching_configs = []
    for cfg in client_configs:
        match_type = cfg.model_match.get("type", "keyword")
        match_values = cfg.model_match.get("value", [])

        is_match = False
        if match_type == "exact":
            if model_name in match_values:
                is_match = True
        elif match_type == "keyword":
            if any(keyword in model_name for keyword in match_values):
                is_match = True
        
        if is_match:
            matching_configs.append(cfg)
            
    return matching_configs

def select_client_by_weight(matching_configs: List[ClientConfig], clients: dict) -> Tuple[OpenAI, ClientConfig]:
    """
    从匹配的配置列表中，根据 priority (权重) 随机选择一个。
    priority 值直接作为权重。如果未设置，默认为 1。
    """
    if not matching_configs:
        raise ValueError("Cannot select a client from an empty list of configs.")

    # priority 默认值为 1
    weights = [cfg.priority if cfg.priority and cfg.priority > 0 else 1 for cfg in matching_configs]
    
    # 使用 priorities 作为权重进行随机选择
    selected_cfg = random.choices(matching_configs, weights=weights, k=1)[0]
    
    return clients[selected_cfg.name], selected_cfg

def get_client_for_model(model_name: str, client_configs: list[ClientConfig], clients: dict) -> Tuple[OpenAI, ClientConfig]:
    """
    主函数，封装了查找、选择和返回客户端的整个过程。
    """
    # 1. 找到所有匹配的配置
    matching_configs = find_matching_clients(model_name, client_configs)
    
    # 2. 如果没有找到匹配项，则引发错误
    if not matching_configs:
        raise ValueError(f"No client configuration found for model {model_name}")
        
    # 3. 如果只有一个匹配项，直接返回
    if len(matching_configs) == 1:
        selected_cfg = matching_configs[0]
        return clients[selected_cfg.name], selected_cfg
        
    # 4. 如果有多个匹配项，根据权重选择
    return select_client_by_weight(matching_configs, clients)
