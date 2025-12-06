#!/bin/bash

# 10个 Rust 服务列表
SERVICES=(
	"match_engine"
	"store"
	"asset"
	"event"
	"processor"
	"depth"
	"websocket_user"
	"websocket_depth"
	"onchain_msg"
	"api"
)

echo "=========================================="
echo "       强制停止所有服务"
echo "=========================================="
echo ""

for SERVICE in "${SERVICES[@]}"; do
	PID_FILE="./logs/${SERVICE}.pid"

	if [ -f "$PID_FILE" ]; then
		PID=$(cat "$PID_FILE")
		if kill -0 "$PID" 2>/dev/null; then
			echo "⏹  停止 $SERVICE (PID: $PID)..."
			kill -9 "$PID" 2>/dev/null
			sleep 0.2
			if ! kill -0 "$PID" 2>/dev/null; then
				echo "   ✅ $SERVICE 已停止"
			else
				echo "   ⚠️  $SERVICE 停止失败"
			fi
		else
			echo "⚪ $SERVICE 进程不存在，跳过"
		fi
		rm -f "$PID_FILE"
	else
		echo "⚪ $SERVICE 未运行，跳过"
	fi
done

echo ""
echo "=========================================="
echo "所有服务已停止"
echo "=========================================="
