.PHONY: all deps trade depth user clean help

# 默认目标
all: help

# 安装依赖
deps:
	@echo "📦 Installing Go dependencies..."
	go mod download
	go mod tidy
	@echo "✅ Dependencies installed"

# 运行 trade responder
trade:
	@echo "🚀 Starting Trade Responder..."
	cd trade_responder && go run main.go

# 运行 WebSocket Depth 客户端
depth:
	@echo "🚀 Starting WebSocket Depth Client..."
	cd websocket_depth && go run main.go

# 运行 WebSocket User 客户端
user:
	@echo "🚀 Starting WebSocket User Client..."
	cd websocket_user && go run main.go

# 构建所有程序
build:
	@echo "🔨 Building all programs..."
	@mkdir -p bin
	cd trade_responder && go build -o ../bin/trade_responder main.go
	cd websocket_depth && go build -o ../bin/websocket_depth main.go
	cd websocket_user && go build -o ../bin/websocket_user main.go
	@echo "✅ Build complete. Binaries in ./bin/"

# 清理构建产物
clean:
	@echo "🧹 Cleaning build artifacts..."
	rm -rf bin/
	@echo "✅ Clean complete"

# 显示帮助
help:
	@echo "=========================================="
	@echo "       Mock Go 客户端工具"
	@echo "=========================================="
	@echo ""
	@echo "可用命令:"
	@echo "  make deps   - 安装 Go 依赖"
	@echo "  make trade  - 运行 Trade Responder"
	@echo "  make depth  - 运行 WebSocket Depth 客户端"
	@echo "  make user   - 运行 WebSocket User 客户端"
	@echo "  make build  - 构建所有程序到 ./bin/"
	@echo "  make clean  - 清理构建产物"
	@echo "  make help   - 显示此帮助信息"
	@echo ""
	@echo "使用示例:"
	@echo "  make deps         # 首次使用，安装依赖"
	@echo "  make trade        # 启动交易响应服务"
	@echo "  make depth        # 启动深度 WebSocket 客户端"
	@echo "  make user         # 启动用户 WebSocket 客户端"
	@echo ""
	@echo "提示: 可以在多个终端同时运行不同的命令"
	@echo "=========================================="
