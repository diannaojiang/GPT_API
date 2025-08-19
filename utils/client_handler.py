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

def select_clients_by_weight(matching_configs: List[ClientConfig]) -> List[ClientConfig]:
    """
    根据 priority (权重) 对匹配的配置列表进行加权随机排序。
    使用 Efraimidis-Spirakis A-Res 算法进行加权随机抽样（无替换）。
    """
    if not matching_configs:
        return []

    # 为每个配置计算一个随机键，该键由其权重决定
    # weight = 1 / priority
    # key = random.uniform(0, 1) ** weight
    weighted_list = []
    for cfg in matching_configs:
        weight = cfg.priority if cfg.priority and cfg.priority > 0 else 1
        key = random.uniform(0, 1) ** (1 / weight)
        weighted_list.append((cfg, key))

    # 按计算出的键进行降序排序
    weighted_list.sort(key=lambda x: x[1], reverse=True)

    # 返回排序后的配置列表
    return [cfg for cfg, key in weighted_list]

def get_clients_for_model(model_name: str, client_configs: list[ClientConfig], clients: dict) -> List[Tuple[OpenAI, ClientConfig]]:
    """
    主函数，封装了查找、加权随机排序和返回客户端列表的整个过程。
    """
    # 1. 找到所有匹配的配置
    matching_configs = find_matching_clients(model_name, client_configs)
    
    # 2. 如果没有找到匹配项，则引发错误
    if not matching_configs:
        raise ValueError(f"No client configuration found for model {model_name}")
        
    # 3. 根据权重（优先级）对匹配的客户端进行加权随机排序
    shuffled_configs = select_clients_by_weight(matching_configs)
    
    # 4. 返回排序后的客户端列表
    return [(clients[cfg.name], cfg) for cfg in shuffled_configs]
