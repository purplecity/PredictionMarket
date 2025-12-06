#!/bin/bash

# 核心服务列表（按启动顺序）
CORE_SERVICES=(
	"match_engine"
	"store"
	"asset"
)

echo "=========================================="
echo "       启动核心服务"
echo "=========================================="
echo ""

# 检查环境变量
if [ -z "$RUN_MODE" ]; then
	export RUN_MODE=dev
	echo "⚠️  RUN_MODE 未设置，使用默认值: dev"
fi

echo "运行模式: $RUN_MODE"
echo ""

START_COUNT=0
SKIP_COUNT=0

for SERVICE in "${CORE_SERVICES[@]}"; do
	PID_FILE="./logs/${SERVICE}.pid"
	LOG_FILE="./logs/${SERVICE}.log"

	# 检查服务是否已经运行
	if [ -f "$PID_FILE" ]; then
		PID=$(cat "$PID_FILE")
		if kill -0 "$PID" 2>/dev/null; then
			echo "⚠️  $SERVICE 已在运行 (PID: $PID)，跳过启动"
			SKIP_COUNT=$((SKIP_COUNT + 1))
			continue
		fi
	fi

	# 启动服务
	echo "🚀 启动 $SERVICE..."
	nohup cargo run --bin "$SERVICE" > "$LOG_FILE" 2>&1 &
	PID=$!
	echo $PID > "$PID_FILE"

	# 等待服务启动
	sleep 1

	# 验证服务是否成功启动
	if kill -0 "$PID" 2>/dev/null; then
		echo "   ✅ $SERVICE 启动成功 (PID: $PID)"
		START_COUNT=$((START_COUNT + 1))
	else
		echo "   ❌ $SERVICE 启动失败，请查看日志: $LOG_FILE"
		rm -f "$PID_FILE"
	fi

	echo ""
done

echo "=========================================="
echo "核心服务启动完成"
echo "新启动: $START_COUNT / 跳过: $SKIP_COUNT"
echo "=========================================="
echo ""
echo "💡 提示: 使用 ./script/status.sh 查看服务状态"
