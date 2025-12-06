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
echo "       服务运行状态检查"
echo "=========================================="
echo ""

RUNNING_COUNT=0
STOPPED_COUNT=0

for SERVICE in "${SERVICES[@]}"; do
	PID_FILE="./logs/${SERVICE}.pid"

	if [ -f "$PID_FILE" ]; then
		PID=$(cat "$PID_FILE")
		if kill -0 "$PID" 2>/dev/null; then
			printf "✅ %-20s [运行中] PID: %s\n" "$SERVICE" "$PID"
			RUNNING_COUNT=$((RUNNING_COUNT + 1))
		else
			printf "❌ %-20s [已停止] (PID文件存在但进程不存在)\n" "$SERVICE"
			STOPPED_COUNT=$((STOPPED_COUNT + 1))
		fi
	else
		printf "⚪ %-20s [已停止] (无PID文件)\n" "$SERVICE"
		STOPPED_COUNT=$((STOPPED_COUNT + 1))
	fi
done

echo ""
echo "=========================================="
echo "总计: 运行中 $RUNNING_COUNT / 已停止 $STOPPED_COUNT"
echo "=========================================="
