# utils/request_handler.py

def process_messages(messages: list) -> list:
    """
    处理消息列表，合并连续的'user'角色消息。
    后一个'user'消息会覆盖前一个。
    """
    if not messages:
        return []
        
    result = []
    for msg in messages:
        if result and msg.get('role') == 'user' and result[-1].get('role') == 'user':
            # 替换最后一条用户消息
            result[-1] = msg
        else:
            # 添加非连续的用户消息或任何其他角色
            result.append(msg)
    return result