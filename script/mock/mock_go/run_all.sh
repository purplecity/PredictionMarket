#!/bin/bash

# 快速启动所有 mock 服务
# 使用 tmux 在不同窗格中运行

SESSION="mock_go"

# 检查 tmux 是否安装
if ! command -v tmux &> /dev/null; then
    echo "❌ tmux 未安装，无法启动多窗格模式"
    echo "请手动在不同终端运行："
    echo "  make trade"
    echo "  make depth"
    echo "  make user"
    exit 1
fi

# 检查是否已有同名 session
tmux has-session -t $SESSION 2>/dev/null

if [ $? == 0 ]; then
    echo "⚠️  Session '$SESSION' 已存在，附加到现有 session"
    tmux attach-session -t $SESSION
    exit 0
fi

# 创建新 session 并运行第一个程序
echo "🚀 启动 Mock Go 服务..."
tmux new-session -d -s $SESSION -n "mock_go"

# 分割窗口
tmux split-window -h -t $SESSION
tmux split-window -v -t $SESSION:0.0

# 在每个窗格运行不同的程序
tmux send-keys -t $SESSION:0.0 "cd $PWD && echo '🔥 Trade Responder' && make trade" C-m
tmux send-keys -t $SESSION:0.1 "cd $PWD && sleep 2 && echo '📊 WebSocket Depth Client' && make depth" C-m
tmux send-keys -t $SESSION:0.2 "cd $PWD && sleep 2 && echo '👤 WebSocket User Client' && make user" C-m

# 选择第一个窗格
tmux select-pane -t $SESSION:0.0

# 附加到 session
echo "✅ 所有服务已启动在 tmux session: $SESSION"
echo ""
echo "快捷键："
echo "  Ctrl+b →/←/↑/↓  - 切换窗格"
echo "  Ctrl+b d        - 分离 session（服务继续运行）"
echo "  Ctrl+b &        - 关闭所有窗格并退出"
echo ""
echo "重新附加："
echo "  tmux attach -t $SESSION"
echo ""

tmux attach-session -t $SESSION
