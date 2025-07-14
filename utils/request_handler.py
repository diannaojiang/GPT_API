# utils/request_handler.py

def process_messages(messages: list) -> list:
    """
    处理消息列表，清理内容中的空白字符，并合并连续的'user'角色消息。
    后一个'user'消息会覆盖前一个。
    """
    if not messages:
        return []

    result = []
    for msg in messages:
        # 1. 首先清理当前消息内容中的空白字符
        content = msg.get("content")
        # 处理纯文本内容
        if isinstance(content, str):
            msg["content"] = content.strip()
        # 处理多模态内容列表
        elif isinstance(content, list):
            for part in content:
                # 只处理内容列表中的文本部分
                if isinstance(part, dict) and part.get("type") == "text" and "text" in part:
                    part["text"] = part["text"].strip()

        # 2. 然后处理合并逻辑
        # 检查result是否不为空，且当前消息和result的最后一条消息是否都是 'user'
        if result and msg.get('role') == 'user' and result[-1].get('role') == 'user':
            # 如果是，用当前处理过的消息替换掉前一个'user'消息
            result[-1] = msg
        else:
            # 否则，将当前处理过的消息添加到结果列表中
            result.append(msg)
            
    return result