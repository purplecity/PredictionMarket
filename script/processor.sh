#!/bin/bash

SERVICE_NAME="processor"
PID_FILE="./logs/${SERVICE_NAME}.pid"
LOG_FILE="./logs/${SERVICE_NAME}.log"

export RUN_MODE=dev
set -a
source deploy/common.env 2>/dev/null || true
set +a

start() {
    if [ -f "$PID_FILE" ]; then
        PID=$(cat "$PID_FILE")
        if kill -0 "$PID" 2>/dev/null; then
            echo "❌ $SERVICE_NAME 已在运行 (PID: $PID)"
            return 1
        fi
    fi

    echo "🚀 启动 $SERVICE_NAME..."
    cargo build --release --bin $SERVICE_NAME

    ./target/release/$SERVICE_NAME >> "$LOG_FILE" 2>&1 &
    PID=$!
    echo $PID > "$PID_FILE"

    sleep 1
    if kill -0 "$PID" 2>/dev/null; then
        echo "✅ $SERVICE_NAME 已启动 (PID: $PID)"
    else
        echo "❌ $SERVICE_NAME 启动失败"
        rm -f "$PID_FILE"
        return 1
    fi
}

stop() {
    if [ ! -f "$PID_FILE" ]; then
        echo "⚠️  $SERVICE_NAME 未运行"
        return 0
    fi

    PID=$(cat "$PID_FILE")
    if kill -0 "$PID" 2>/dev/null; then
        echo "🛑 停止 $SERVICE_NAME (PID: $PID)..."
        kill "$PID"
        sleep 2

        if kill -0 "$PID" 2>/dev/null; then
            echo "⚠️  强制停止 $SERVICE_NAME..."
            kill -9 "$PID"
        fi
    fi

    rm -f "$PID_FILE"

    echo "🧹 清理构建产物..."
    cargo clean -p $SERVICE_NAME --release

    echo "✅ $SERVICE_NAME 已停止"
}

restart() {
    stop
    sleep 1
    start
}

case "$1" in
    start)
        start
        ;;
    stop)
        stop
        ;;
    restart)
        restart
        ;;
    *)
        echo "用法: $0 {start|stop|restart}"
        exit 1
        ;;
esac
