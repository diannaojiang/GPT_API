# utils/db_handler.py

import os
import time
import json
import datetime
import threading
import traceback
from contextlib import contextmanager

import sqlalchemy
import sqlalchemy_utils
from sqlalchemy import create_engine, Column, String, Text, DateTime, Integer, Boolean
from sqlalchemy.orm import Session
from sqlalchemy.ext.declarative import declarative_base

from utils.log import logger

Base = declarative_base()

class Record(Base):
    __tablename__ = 'records'
    id = Column(Integer, primary_key=True, autoincrement=True)
    Time = Column(DateTime, nullable=False)
    IP = Column(String(15))
    Model = Column(String(50))
    # --- 新增字段 ---
    Type = Column(String(50))  # 用于区分 text_completion 和 chat.completions
    CompletionTokens = Column(Integer)
    PromptTokens = Column(Integer)
    TotalTokens = Column(Integer)
    Tool = Column(Boolean, default=False)  # 记录是否使用了 tool
    Multimodal = Column(Boolean, default=False)  # 记录是否有多模态请求
    Headers = Column(Text)  # 新增 Headers 字段，用于存储整个请求头
    # --- 结束 ---
    Request = Column(Text)
    Response = Column(Text)

class DatabaseManager:
    def __init__(self, db_path_str: str):
        self.db_path_str = os.path.abspath(db_path_str)
        self.db_dir = os.path.dirname(self.db_path_str)

        if not os.access(self.db_dir, os.W_OK):
            error_message = (
                f"数据库目录 '{self.db_dir}' 不可写。 "
                f"请检查运行本程序的用户的文件夹写入权限。"
            )
            logger.error(error_message)
            raise PermissionError(error_message)

        self.db_filename = os.path.basename(self.db_path_str)
        self.engine = None
        self._lock = threading.Lock()
        self._initialize_engine()

    def _initialize_engine(self):
        """(私有) 初始化或重新初始化数据库引擎。"""
        logger.info(f"Process {os.getpid()}: Initializing database engine for {self.db_path_str}")
        if self.engine:
            self.engine.dispose()
        self.engine = create_engine(f"sqlite:///{self.db_path_str}?check_same_thread=False")
        Base.metadata.create_all(self.engine)

    def force_reinitialize(self):
        """(公共) 强制重新初始化引擎，用于错误恢复。"""
        with self._lock:
            logger.warning(f"Process {os.getpid()} is forcefully re-initializing the database engine due to a detected error.")
            self._initialize_engine()

    def _check_and_rotate(self):
        """(私有) 检查并执行数据库轮转。"""
        now = datetime.datetime.now()
        if os.path.exists(self.db_path_str):
            mod_time = os.path.getmtime(self.db_path_str)
            mod_date = datetime.datetime.fromtimestamp(mod_time)
            if mod_date.year != now.year or mod_date.month != now.month:
                archive_filename = f"record_{mod_date.strftime('%Y%m')}.db"
                archive_db_path = os.path.join(self.db_dir, archive_filename)
                logger.info(f"Database rotation needed. Archiving {self.db_path_str} to {archive_db_path}")

                if self.engine:
                    self.engine.dispose()
                    self.engine = None

                if os.path.exists(archive_db_path):
                     new_archive_db_path = os.path.join(self.db_dir, f"record_{mod_date.strftime('%Y%m')}_{int(time.time())}.db")
                     logger.warning(f"Archive file {archive_db_path} already exists. Renaming to {new_archive_db_path}")
                     archive_db_path = new_archive_db_path
                os.rename(self.db_path_str, archive_db_path)
                logger.info("Database archived successfully.")

        if self.engine is None:
            self._initialize_engine()

    def get_session(self) -> Session:
        with self._lock:
            self._check_and_rotate()
            return Session(self.engine)

# 全局实例化管理器
rec_path = os.getenv("RECD_PATH", "record.db")
db_manager = DatabaseManager(rec_path)

@contextmanager
def session_scope():
    session = db_manager.get_session()
    try:
        yield session
        session.commit()
    except:
        session.rollback()
        raise
    finally:
        session.close()

def log_request(client_ip: str, model: str, request_payload: dict, response: object, headers: dict, request_type: str = None, tools_used: bool = False,is_multimodal: bool = False):
    """
    将请求写入数据库，并在失败时根据您的思路进行一次自动重试。
    """
    # 从 response 对象中安全地获取 usage 信息
    usage = getattr(response, 'usage', None)
    completion_tokens = getattr(usage, 'completion_tokens', 0) if usage else 0
    prompt_tokens = getattr(usage, 'prompt_tokens', 0) if usage else 0
    total_tokens = getattr(usage, 'total_tokens', 0) if usage else 0

    record_data = {
        "Time": datetime.datetime.now(),
        "IP": client_ip,
        "Model": model,
        "Type": request_type,
        "CompletionTokens": completion_tokens,
        "PromptTokens": prompt_tokens,
        "TotalTokens": total_tokens,
        "Tool": tools_used,
        "Multimodal": is_multimodal,
        "Headers": json.dumps(headers, ensure_ascii=False),
        "Request": json.dumps(request_payload, ensure_ascii=False),
        "Response": json.dumps(response.to_dict(), ensure_ascii=False),
    }

    try:
        # 第一次尝试
        with session_scope() as session:
            session.add(Record(**record_data))
    except sqlalchemy.exc.OperationalError as e:
        # 捕获到特定错误，开始自愈和重试
        if "readonly" in str(e).lower() or "no such table" in str(e).lower():
            logger.warning(f"Caught a recoverable database error ({e.__class__.__name__}). Forcing re-initialization and retrying once.")
            try:
                # 强制该进程的DatabaseManager重新初始化
                db_manager.force_reinitialize()
                # 第二次，也是最后一次尝试
                with session_scope() as session:
                    session.add(Record(**record_data))
                logger.info("Successfully wrote log record after self-healing.")
            except Exception as retry_e:
                logger.error(f"Self-healing retry attempt failed: {retry_e}")
                logger.error(traceback.format_exc())
        else:
            # 如果是其他数据库操作错误，则不重试，直接抛出
            logger.error("Caught a non-recoverable database operational error.")
            logger.error(traceback.format_exc())
    except Exception as e:
        logger.error(f"An unexpected error occurred during log request: {e}")
        logger.error(traceback.format_exc())