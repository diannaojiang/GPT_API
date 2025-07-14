#!/bin/bash

# 系统服务管理脚本 - Uvicorn Systemd Service Manager

# 配置参数
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"  # 脚本所在目录
LOG_DIR="${SCRIPT_DIR}/logs"                                # 日志目录路径
SERVICE_NAME="openai-api"                              # 服务名称
SERVICE_FILE="/etc/systemd/system/${SERVICE_NAME}.service"  # 服务文件路径
CONDA_ENV="openai"                                          # Conda环境名称

# Uvicorn启动参数
UVICORN_APP="main:app"              # 启动应用标识
UVICORN_HOST="0.0.0.0"             # 监听地址
UVICORN_PORT=7000                  # 监听端口
UVICORN_WORKERS=64                 # 工作进程数

# 获取conda基础路径
get_conda_path() {
    conda info --base 2>/dev/null
}

# 服务安装函数
install_service() {
    # 权限检测
    if [ "$EUID" -ne 0 ]; then
        echo "请使用sudo运行安装命令"
        exit 1
    fi

    # 获取conda路径
    CONDA_PATH=$(get_conda_path)
    if [ -z "$CONDA_PATH" ]; then
        echo "错误：未找到Conda安装路径"
        exit 1
    fi

    # 准备日志目录
    local log_dir_escaped=$(systemd-escape --path "${LOG_DIR}")
    mkdir -p "${LOG_DIR}"
    chown "${SUDO_USER}" "${LOG_DIR}"

    # 生成服务文件
    cat > /tmp/${SERVICE_NAME}.service <<EOF
[Unit]
Description=Uvicorn Service - ${UVICORN_APP}
After=network.target

[Service]
Type=exec
User=${SUDO_USER}
WorkingDirectory=${SCRIPT_DIR}
Environment="PATH=${CONDA_PATH}/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin"
ExecStartPre=/bin/bash -c "mkdir -p ${LOG_DIR}"
ExecStart=${CONDA_PATH}/bin/conda run --no-capture-output -n ${CONDA_ENV} \\
    uvicorn ${UVICORN_APP} \\
    --host ${UVICORN_HOST} \\
    --port ${UVICORN_PORT} \\
    --proxy-headers \\
    --workers ${UVICORN_WORKERS}
Restart=always
RestartSec=10
StandardOutput=append:${LOG_DIR}/${SERVICE_NAME}.log
StandardError=append:${LOG_DIR}/${SERVICE_NAME}.error.log

[Install]
WantedBy=multi-user.target
EOF

    # 部署服务文件
    mv /tmp/${SERVICE_NAME}.service ${SERVICE_FILE}
    systemctl daemon-reload
    systemctl enable ${SERVICE_NAME}

    echo "系统服务安装完成"
    echo "启动命令：sudo systemctl start ${SERVICE_NAME}"
    echo "状态查看：sudo systemctl status ${SERVICE_NAME}"
}

# 服务卸载函数
uninstall_service() {
    if [ "$EUID" -ne 0 ]; then
        echo "请使用sudo运行卸载命令"
        exit 1
    fi

    systemctl stop ${SERVICE_NAME} 2>/dev/null
    systemctl disable ${SERVICE_NAME} 2>/dev/null
    rm -f ${SERVICE_FILE}
    systemctl daemon-reload

    echo "系统服务已卸载"
}

# 使用说明
usage() {
    echo "使用方法: $0 {install|uninstall|start|stop|restart|status}"
    echo "命令说明:"
    echo "  install    安装系统服务"
    echo "  uninstall  卸载系统服务"
    echo "  start      启动服务"
    echo "  stop       停止服务"
    echo "  restart    重启服务"
    echo "  status     查看服务状态"
}

# 服务状态检查
service_status() {
    systemctl status ${SERVICE_NAME}
}

# 主程序
case "$1" in
    install)
        install_service
        ;;
    uninstall)
        uninstall_service
        ;;
    start)
        systemctl start ${SERVICE_NAME}
        ;;
    stop)
        systemctl stop ${SERVICE_NAME}
        ;;
    restart)
        systemctl restart ${SERVICE_NAME}
        ;;
    status)
        service_status
        ;;
    *)
        usage
        exit 1
        ;;
esac
