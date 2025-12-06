#!/bin/bash

# API 基础地址
BASE_URL="http://localhost:5002/api"

# 测试用的 JWT Token (需要替换为真实的 Privy JWT)
JWT_TOKEN="eyJhbGciOiJFUzI1NiIsInR5cCI6IkpXVCIsImtpZCI6IndSUFVRemY4M3NQX2tZVHNHWGt0eUtFaDBVOU1OWmZ3VzlqVWE2VTVlSFkifQ.eyJzaWQiOiJjbWlpcXJlMGgxMWk1amswZG1pN21icnIyIiwiaXNzIjoicHJpdnkuaW8iLCJpYXQiOjE3NjQzNzg3NDUsImF1ZCI6ImNtN3ZtaXlmdjAwaXRnaHVqZzA3NnZqbWkiLCJzdWIiOiJkaWQ6cHJpdnk6Y21jMXI4bG1rMDE0bWxkMG53bjBwd2V3cSIsImV4cCI6MTc2NDM4MjM0NX0.E2VoJdh-p4jDLurYmP7nict9hiVx0PoR2SsxdVQv7gAOWVA9fGNFUfOXG4Lkf3pqJ4epwIoRfpzuo7lPTKtvng"

# 辅助函数：打印帮助信息
show_help() {
	echo "=========================================="
	echo "       API 测试脚本"
	echo "=========================================="
	echo ""
	echo "使用方法: ./script/mock/api.sh <api_name> [参数]"
	echo ""
	echo "公开接口 (无需鉴权):"
	echo "  topics              - 获取主题列表"
	echo "  events              - 获取事件列表"
	echo "  event_detail        - 获取事件详情"
	echo "  depth               - 获取市场深度"
	echo ""
	echo "需要鉴权的接口:"
	echo "  user_data           - 获取用户信息"
	echo "  user_profile        - 更新用户资料"
	echo "  user_image          - 更新用户头像"
	echo "  image_sign          - 获取图片上传签名"
	echo "  place_order         - 提交订单"
	echo "  cancel_order        - 取消订单"
	echo "  cancel_all_orders   - 取消所有订单"
	echo "  portfolio_value     - 获取投资组合价值"
	echo "  traded_volume       - 获取交易量"
	echo "  positions           - 获取持仓列表"
	echo "  closed_positions    - 获取已平仓列表"
	echo "  activity            - 获取活动历史"
	echo "  open_orders         - 获取未完成订单"
	echo "  event_balance       - 获取事件余额"
	echo ""
	echo "示例:"
	echo "  ./script/mock/api.sh topics"
	echo "  ./script/mock/api.sh events"
	echo "  ./script/mock/api.sh user_data"
	echo "=========================================="
}

# 如果没有参数，显示帮助
if [ $# -eq 0 ]; then
	show_help
	exit 0
fi

API_NAME=$1

case $API_NAME in
	# ========== 公开接口 ==========

	topics)
		echo "🌐 调用 GET /topics"
		curl -X GET "${BASE_URL}/topics" \
			-H "Content-Type: application/json" | jq
		;;

	events)
		echo "🌐 调用 GET /events"
		# 可修改的参数
		PAGE=1
		PAGE_SIZE=10
		TOPIC=""  # 可选: crypto, sports, politics

		curl -X GET "${BASE_URL}/events?page=${PAGE}&page_size=${PAGE_SIZE}" \
			-H "Content-Type: application/json" | jq
		;;

	event_detail)
		echo "🌐 调用 GET /event_detail"
		# 可修改的参数
		EVENT_ID=1

		curl -X GET "${BASE_URL}/event_detail?event_id=${EVENT_ID}" \
			-H "Content-Type: application/json" | jq
		;;

	depth)
		echo "🌐 调用 GET /depth"
		# 可修改的参数
		EVENT_ID=1
		MARKET_ID=1

		curl -X GET "${BASE_URL}/depth?event_id=${EVENT_ID}&market_id=${MARKET_ID}" \
			-H "Content-Type: application/json" | jq
		;;

	# ========== 需要鉴权的接口 ==========

	user_data)
		echo "🔐 调用 GET /user_data"
		curl -X GET "${BASE_URL}/user_data" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" | jq
		;;

	user_profile)
		echo "🔐 调用 POST /user_profile"
		# 可修改的参数
		NAME="测试用户"
		BIO="这是我的个人简介"

		curl -X POST "${BASE_URL}/user_profile" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" \
			-d "{
				\"name\": \"${NAME}\",
				\"bio\": \"${BIO}\"
			}" | jq
		;;

	user_image)
		echo "🔐 调用 POST /user_image"
		# 可修改的参数
		IMAGE_URL="https://storage.googleapis.com/example/image.jpg"
		IMAGE_TYPE="header"

		curl -X POST "${BASE_URL}/user_image" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" \
			-d "{
				\"image_url\": \"${IMAGE_URL}\",
				\"image_type\": \"${IMAGE_TYPE}\"
			}" | jq
		;;

	image_sign)
		echo "🔐 调用 GET /image_sign"
		# 可修改的参数
		BUCKET_TYPE="header"
		IMAGE_TYPE="jpg"

		curl -X GET "${BASE_URL}/image_sign?bucket_type=${BUCKET_TYPE}&image_type=${IMAGE_TYPE}" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" | jq
		;;

	place_order)
		echo "🔐 调用 POST /place_order"
		# 可修改的参数 - 这是一个示例订单，需要根据实际情况修改
		EVENT_ID=1
		MARKET_ID=1
		TOKEN_ID="token_yes_2"
		SIDE="BUY"
		PRICE="0.9"
		ORDER_TYPE="Limit"
		MAKER="0x1234567890123456789012345678901234567890"
		MAKER_AMOUNT="90000000000000000000" #100
		TAKER_AMOUNT="100000000000000000000" #150
		SIGNATURE="0xabcdef..."

		curl -X POST "${BASE_URL}/place_order" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" \
			-d "{
				\"event_id\": ${EVENT_ID},
				\"market_id\": ${MARKET_ID},
				\"tokenId\": \"${TOKEN_ID}\",
				\"side\": \"${SIDE}\",
				\"price\": \"${PRICE}\",
				\"order_type\": \"${ORDER_TYPE}\",
				\"maker\": \"${MAKER}\",
				\"makerAmount\": \"${MAKER_AMOUNT}\",
				\"taker\": \"0x0000000000000000000000000000000000000000\",
				\"takerAmount\": \"${TAKER_AMOUNT}\",
				\"expiration\": \"1735689600\",
				\"nonce\": \"1\",
				\"feeRateBps\": \"100\",
				\"salt\": 12345,
				\"signature\": \"${SIGNATURE}\",
				\"signatureType\": 2,
				\"signer\": \"${MAKER}\"
			}" | jq
		;;

	cancel_order)
		echo "🔐 调用 POST /cancel_order"
		# 可修改的参数
		ORDER_ID="2814d219-eaaa-46dd-892b-fe829cb20524"

		curl -X POST "${BASE_URL}/cancel_order" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" \
			-d "{
				\"order_id\": \"${ORDER_ID}\"
			}" | jq
		;;

	cancel_all_orders)
		echo "🔐 调用 POST /cancel_all_orders"
		curl -X POST "${BASE_URL}/cancel_all_orders" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" | jq
		;;

	portfolio_value)
		echo "🔐 调用 GET /portfolio_value"
		curl -X GET "${BASE_URL}/portfolio_value" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" | jq
		;;

	traded_volume)
		echo "🔐 调用 GET /traded_volume"
		curl -X GET "${BASE_URL}/traded_volume" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" | jq
		;;

	positions)
		echo "🔐 调用 GET /positions"
		# 可修改的参数
		PAGE=1
		EVENT_ID=""  # 可选
		MARKET_ID="" # 可选

		QUERY="page=${PAGE}"
		[ -n "$EVENT_ID" ] && QUERY="${QUERY}&event_id=${EVENT_ID}"
		[ -n "$MARKET_ID" ] && QUERY="${QUERY}&market_id=${MARKET_ID}"

		curl -X GET "${BASE_URL}/positions?${QUERY}" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" | jq
		;;

	closed_positions)
		echo "🔐 调用 GET /closed_positions"
		# 可修改的参数
		PAGE=1

		curl -X GET "${BASE_URL}/closed_positions?page=${PAGE}" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" | jq
		;;

	activity)
		echo "🔐 调用 GET /activity"
		# 可修改的参数
		PAGE=1
		PAGE_SIZE=20

		curl -X GET "${BASE_URL}/activity?page=${PAGE}&page_size=${PAGE_SIZE}" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" | jq
		;;

	open_orders)
		echo "🔐 调用 GET /open_orders"
		# 可修改的参数
		PAGE=1
		PAGE_SIZE=20

		curl -X GET "${BASE_URL}/open_orders?page=${PAGE}&page_size=${PAGE_SIZE}" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" | jq
		;;

	event_balance)
		echo "🔐 调用 GET /event_balance"
		# 可修改的参数
		EVENT_ID=1
		MARKET_ID=""  # 可选，留空则返回整个事件的余额

		QUERY="event_id=${EVENT_ID}"
		[ -n "$MARKET_ID" ] && QUERY="${QUERY}&market_id=${MARKET_ID}"

		curl -X GET "${BASE_URL}/event_balance?${QUERY}" \
			-H "Content-Type: application/json" \
			-H "Authorization: Bearer ${JWT_TOKEN}" | jq
		;;

	*)
		echo "❌ 未知的 API: $API_NAME"
		echo ""
		show_help
		exit 1
		;;
esac
