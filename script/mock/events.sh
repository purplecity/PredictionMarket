#!/bin/bash

# Redis 配置
REDIS_HOST="127.0.0.1"
REDIS_PORT="8889"
REDIS_PASSWORD="123456"
REDIS_DB="0"  # REDIS_DB_COMMON_MQ
STREAM_NAME="event-stream"
MSG_KEY="event-stream"

# 辅助函数：打印帮助信息
show_help() {
	echo "=========================================="
	echo "       事件消息推送脚本"
	echo "=========================================="
	echo ""
	echo "使用方法: ./script/mock/events.sh <action> [参数]"
	echo ""
	echo "可用操作:"
	echo "  create  - 创建事件 (包含市场信息)"
	echo "  close   - 关闭事件"
	echo ""
	echo "示例:"
	echo "  ./script/mock/events.sh create"
	echo "  ./script/mock/events.sh close"
	echo "=========================================="
}

# 如果没有参数，显示帮助
if [ $# -eq 0 ]; then
	show_help
	exit 0
fi

ACTION=$1

case $ACTION in
	create)
		echo "🎯 推送事件创建消息到 Redis"

		# 可修改的参数
		EVENT_IDENTIFIER="event_test_002"
		SLUG="will-btc-reach-100k"
		TITLE="Will BTC reach 100K in 2025?"
		DESCRIPTION="Predict if Bitcoin will reach $100,000 by end of 2025"
		IMAGE="https://example.com/btc.jpg"
		END_DATE=$(($(date +%s) + 180000))  # 当前时间 + 5分钟（300秒）
		TOPIC="crypto"

		# Market 1
		PARENT_COLLECTION_ID_1="0xparent_collection_002"
		CONDITION_ID_1="0xcondition_id_market_2"
		MARKET_IDENTIFIER_1="market_btc_200k"
		MARKET_QUESTION_1="Will BTC reach 200K?"
		MARKET_SLUG_1="btc-200k"
		MARKET_TITLE_1="BTC 200K"
		MARKET_DESC_1="Will BTC reach 200K?"
		MARKET_IMAGE_1="https://example.com/market1.jpg"
		TOKEN_0_ID_1="token_yes_2"
		TOKEN_1_ID_1="token_no_2"

		# 构建 JSON 消息 (EventCreate 类型)
		MESSAGE=$(cat <<EOF
{
	"types": "EventCreate",
	"event_identifier": "${EVENT_IDENTIFIER}",
	"slug": "${SLUG}",
	"title": "${TITLE}",
	"description": "${DESCRIPTION}",
	"image": "${IMAGE}",
	"end_date": ${END_DATE},
	"topic": "${TOPIC}",
	"markets": [
		{
			"parent_collection_id": "${PARENT_COLLECTION_ID_1}",
			"condition_id": "${CONDITION_ID_1}",
			"market_identifier": "${MARKET_IDENTIFIER_1}",
			"question": "${MARKET_QUESTION_1}",
			"slug": "${MARKET_SLUG_1}",
			"title": "${MARKET_TITLE_1}",
			"description": "${MARKET_DESC_1}",
			"image": "${MARKET_IMAGE_1}",
			"outcomeNames": ["Yes", "No"],
			"tokenIds": ["${TOKEN_0_ID_1}", "${TOKEN_1_ID_1}"]
		}
	]
}
EOF
)

		# 推送到 Redis Stream
		redis-cli -h ${REDIS_HOST} -p ${REDIS_PORT} -a ${REDIS_PASSWORD} -n ${REDIS_DB} XADD ${STREAM_NAME} "*" ${MSG_KEY} "${MESSAGE}"

		echo "✅ 事件创建消息已推送"
		echo "   Event Identifier: ${EVENT_IDENTIFIER}"
		echo "   Title: ${TITLE}"
		echo "   Topic: ${TOPIC}"
		echo "   End Date: ${END_DATE} ($(date -r ${END_DATE} '+%Y-%m-%d %H:%M:%S'))"
		echo "   Markets: 1"
		;;

	create_multi)
		echo "🎯 推送多市场事件创建消息到 Redis"

		# 可修改的参数
		EVENT_IDENTIFIER="event_test_002"
		SLUG="election-2024"
		TITLE="2024 Election Prediction"
		DESCRIPTION="Predict the outcomes of 2024 election"
		IMAGE="https://example.com/election.jpg"
		END_DATE=$(($(date +%s) + 300))  # 当前时间 + 5分钟（300秒）
		TOPIC="politics"

		# Market 1
		PARENT_COLLECTION_ID_1="0xparent_collection_001"
		CONDITION_ID_1="0xcondition_id_market_1"
		MARKET_IDENTIFIER_1="market_candidate_a"
		MARKET_QUESTION_1="Will Candidate A win?"
		MARKET_SLUG_1="candidate-a-wins"
		MARKET_TITLE_1="Candidate A Wins"
		MARKET_DESC_1="Will Candidate A win?"
		MARKET_IMAGE_1="https://example.com/candidate_a.jpg"
		TOKEN_0_ID_1="token_yes_1"
		TOKEN_1_ID_1="token_no_1"

		# Market 2
		PARENT_COLLECTION_ID_2="0xparent_collection_002"
		CONDITION_ID_2="0xcondition_id_market_2"
		MARKET_IDENTIFIER_2="market_candidate_b"
		MARKET_QUESTION_2="Will Candidate B win?"
		MARKET_SLUG_2="candidate-b-wins"
		MARKET_TITLE_2="Candidate B Wins"
		MARKET_DESC_2="Will Candidate B win?"
		MARKET_IMAGE_2="https://example.com/candidate_b.jpg"
		TOKEN_0_ID_2="token_yes_2"
		TOKEN_1_ID_2="token_no_2"

		# 构建 JSON 消息 (EventCreate 类型，包含多个市场)
		MESSAGE=$(cat <<EOF
{
	"types": "EventCreate",
	"event_identifier": "${EVENT_IDENTIFIER}",
	"slug": "${SLUG}",
	"title": "${TITLE}",
	"description": "${DESCRIPTION}",
	"image": "${IMAGE}",
	"end_date": ${END_DATE},
	"topic": "${TOPIC}",
	"markets": [
		{
			"parent_collection_id": "${PARENT_COLLECTION_ID_1}",
			"condition_id": "${CONDITION_ID_1}",
			"market_identifier": "${MARKET_IDENTIFIER_1}",
			"question": "${MARKET_QUESTION_1}",
			"slug": "${MARKET_SLUG_1}",
			"title": "${MARKET_TITLE_1}",
			"description": "${MARKET_DESC_1}",
			"image": "${MARKET_IMAGE_1}",
			"outcomeNames": ["Yes", "No"],
			"tokenIds": ["${TOKEN_0_ID_1}", "${TOKEN_1_ID_1}"]
		},
		{
			"parent_collection_id": "${PARENT_COLLECTION_ID_2}",
			"condition_id": "${CONDITION_ID_2}",
			"market_identifier": "${MARKET_IDENTIFIER_2}",
			"question": "${MARKET_QUESTION_2}",
			"slug": "${MARKET_SLUG_2}",
			"title": "${MARKET_TITLE_2}",
			"description": "${MARKET_DESC_2}",
			"image": "${MARKET_IMAGE_2}",
			"outcomeNames": ["Yes", "No"],
			"tokenIds": ["${TOKEN_0_ID_2}", "${TOKEN_1_ID_2}"]
		}
	]
}
EOF
)

		# 推送到 Redis Stream
		redis-cli -h ${REDIS_HOST} -p ${REDIS_PORT} -a ${REDIS_PASSWORD} -n ${REDIS_DB} XADD ${STREAM_NAME} "*" ${MSG_KEY} "${MESSAGE}"

		echo "✅ 多市场事件创建消息已推送"
		echo "   Event Identifier: ${EVENT_IDENTIFIER}"
		echo "   Title: ${TITLE}"
		echo "   Topic: ${TOPIC}"
		echo "   End Date: ${END_DATE} ($(date -r ${END_DATE} '+%Y-%m-%d %H:%M:%S'))"
		echo "   Markets: 2"
		;;

	close)
		echo "🔒 推送事件关闭消息到 Redis"

		# 可修改的参数
		EVENT_IDENTIFIER="event_test_002"

		# Market 1 结果
		M1_MARKET_IDENTIFIER="market_btc_100k"
		M1_WIN_TOKEN_ID="token_yes_2"
		M1_WIN_OUTCOME_NAME="Yes"

		# 构建 JSON 消息 (EventClose 类型)
		MESSAGE=$(cat <<EOF
{
	"types": "EventClose",
	"event_identifier": "${EVENT_IDENTIFIER}",
	"markets_result": [
		{
			"market_identifier": "${M1_MARKET_IDENTIFIER}",
			"win_outcome_token_id": "${M1_WIN_TOKEN_ID}",
			"win_outcome_name": "${M1_WIN_OUTCOME_NAME}"
		}
	]
}
EOF
)

		# 推送到 Redis Stream
		redis-cli -h ${REDIS_HOST} -p ${REDIS_PORT} -a ${REDIS_PASSWORD} -n ${REDIS_DB} XADD ${STREAM_NAME} "*" ${MSG_KEY} "${MESSAGE}"

		echo "✅ 事件关闭消息已推送"
		echo "   Event Identifier: ${EVENT_IDENTIFIER}"
		echo "   Markets with results: 1"
		;;

	*)
		echo "❌ 未知的操作: $ACTION"
		echo ""
		show_help
		exit 1
		;;
esac

echo ""
echo "💡 提示: 使用以下命令查看 Redis Stream:"
echo "   redis-cli XREAD COUNT 10 STREAMS ${STREAM_NAME} 0"
