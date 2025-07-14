import os
import yaml
from pathlib import Path
from typing import List, Dict
from openai import OpenAI

class ClientConfig:
    def __init__(self, config: Dict):
        self.name = config["name"]
        self.api_key = self._parse_env_var(config["api_key"])
        self.base_url = config["base_url"]
        self.model_match = config["model_match"]
        self.priority = config.get("priority", 999)
        self.max_tokens = config.get("max_tokens")
        self.special_prefix = config.get("special_prefix")
        self.stop = config.get("stop")
        self.fallback = config.get("fallback", False)

    def _parse_env_var(self, value: str) -> str:
        """解析环境变量替换语法 ${VAR_NAME}"""
        if value.startswith("${") and value.endswith("}"):
            env_var = value[2:-1]
            return os.getenv(env_var, "")
        return value

def load_clients() -> List[ClientConfig]:
    config_path = Path(__file__).parent / "config/config.yaml"
    with open(config_path, 'r') as f:
        config = yaml.safe_load(f)
    
    clients = [ClientConfig(c) for c in config["openai_clients"]]
    return sorted(clients, key=lambda x: x.priority)

def init_openai_clients() -> Dict[str, OpenAI]:
    """初始化所有OpenAI客户端"""
    return {
        cfg.name: OpenAI(
            api_key=cfg.api_key,
            base_url=cfg.base_url
        ) for cfg in load_clients()
    }
