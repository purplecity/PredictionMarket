#!/bin/bash

# Redis 配置
REDIS_HOST="127.0.0.1"
REDIS_PORT="8889"
REDIS_PASSWORD="123456"
REDIS_DB="0"  # REDIS_DB_COMMON_MQ
STREAM_NAME="deepsense:asset:service:event"
MSG_KEY="event"

# USDC Token ID
USDC_TOKEN_ID="usdc"

# 辅助函数：打印帮助信息
show_help() {
	echo "=========================================="
	echo "       链上消息推送脚本"
	echo "=========================================="
	echo ""
	echo "使用方法: ./script/mock/onchain_msg.sh <action> [参数]"
	echo ""
	echo "可用操作:"
	echo "  deposit   - 充值 USDC"
	echo "  withdraw  - 提现 USDC"
	echo "  split     - 拆分 USDC 为代币"
	echo "  merge     - 合并代币为 USDC"
	echo "  redeem    - 赎回获胜代币"
	echo ""
	echo "示例:"
	echo "  ./script/mock/onchain_msg.sh deposit"
	echo "  ./script/mock/onchain_msg.sh withdraw"
	echo "  ./script/mock/onchain_msg.sh split"
	echo "=========================================="
}

# 如果没有参数，显示帮助
if [ $# -eq 0 ]; then
	show_help
	exit 0
fi

ACTION=$1

case $ACTION in
	deposit)
		echo "💰 推送充值消息到 Redis"

		# 可修改的参数
		USER_ID=2
		PRIVY_ID="cmc1r8lmk014mld0nwn0pwewq"
		USER_ADDRESS="0x1234567890123456789012345678901234567890"
		AMOUNT="1000000000000000000000"  # 1000 USDC (18位精度)
		TX_HASH="0xdeposit_tx_hash_002"

		# 构建 JSON 消息（紧凑格式，无换行符）
		MESSAGE="{\"types\":\"deposit\",\"user_id\":${USER_ID},\"privy_id\":\"${PRIVY_ID}\",\"user_address\":\"${USER_ADDRESS}\",\"token_id\":\"${USDC_TOKEN_ID}\",\"amount\":\"${AMOUNT}\",\"tx_hash\":\"${TX_HASH}\"}"

		# 推送到 Redis Stream
		redis-cli -h ${REDIS_HOST} -p ${REDIS_PORT} -a ${REDIS_PASSWORD} -n ${REDIS_DB} XADD ${STREAM_NAME} "*" ${MSG_KEY} "${MESSAGE}"

		echo "✅ 充值消息已推送"
		echo "   用户ID: ${USER_ID}"
		echo "   金额: ${AMOUNT} (原始值，包含18位精度)"
		echo "   实际金额: 1000 USDC"
		;;

	withdraw)
		echo "💸 推送提现消息到 Redis"

		# 可修改的参数
		USER_ID=1
		PRIVY_ID="cmc1r8lmk014mld0nwn0pwewq"
		USER_ADDRESS="0x1234567890123456789012345678901234567890"
		AMOUNT="500000000000000000000"  # 500 USDC (18位精度)
		TX_HASH="0xwithdraw_tx_hash_002"

		# 构建 JSON 消息（紧凑格式，无换行符）
		MESSAGE="{\"types\":\"withdraw\",\"user_id\":${USER_ID},\"privy_id\":\"${PRIVY_ID}\",\"user_address\":\"${USER_ADDRESS}\",\"token_id\":\"${USDC_TOKEN_ID}\",\"amount\":\"${AMOUNT}\",\"tx_hash\":\"${TX_HASH}\"}"

		# 推送到 Redis Stream
		redis-cli -h ${REDIS_HOST} -p ${REDIS_PORT} -a ${REDIS_PASSWORD} -n ${REDIS_DB} XADD ${STREAM_NAME} "*" ${MSG_KEY} "${MESSAGE}"

		echo "✅ 提现消息已推送"
		echo "   用户ID: ${USER_ID}"
		echo "   金额: ${AMOUNT} (原始值，包含18位精度)"
		echo "   实际金额: 500 USDC"
		;;

	split)
		echo "✂️  推送拆分消息到 Redis"

		# 可修改的参数
		USER_ID=2
		PRIVY_ID="cmc1r8lmk014mld0nwn0pwewq"
		USER_ADDRESS="0x1234567890123456789012345678901234567890"
		AMOUNT="100000000000000000000"  # 200 USDC (18位精度)
		CONDITION_ID="0xcondition_id_market_2"
		TX_HASH="0xsplit_tx_hash_002"

		# 构建 JSON 消息（紧凑格式，无换行符）
		MESSAGE="{\"types\":\"split\",\"user_id\":${USER_ID},\"privy_id\":\"${PRIVY_ID}\",\"user_address\":\"${USER_ADDRESS}\",\"amount\":\"${AMOUNT}\",\"condition_id\":\"${CONDITION_ID}\",\"tx_hash\":\"${TX_HASH}\"}"

		# 推送到 Redis Stream
		redis-cli -h ${REDIS_HOST} -p ${REDIS_PORT} -a ${REDIS_PASSWORD} -n ${REDIS_DB} XADD ${STREAM_NAME} "*" ${MSG_KEY} "${MESSAGE}"

		echo "✅ 拆分消息已推送"
		echo "   用户ID: ${USER_ID}"
		echo "   金额: ${AMOUNT} (原始值，包含18位精度)"
		echo "   实际金额: 100 USDC"
		echo "   Condition ID: ${CONDITION_ID}"
		;;

	merge)
		echo "🔀 推送合并消息到 Redis"

		# 可修改的参数
		USER_ID=1
		PRIVY_ID="cmc1r8lmk014mld0nwn0pwewq"
		USER_ADDRESS="0x1234567890123456789012345678901234567890"
		AMOUNT="50000000000000000000"  # 50 USDC (18位精度)
		CONDITION_ID="0xcondition_id_market_1"
		TX_HASH="0xmerge_tx_hash_002"

		# 构建 JSON 消息（紧凑格式，无换行符）
		MESSAGE="{\"types\":\"merge\",\"user_id\":${USER_ID},\"privy_id\":\"${PRIVY_ID}\",\"user_address\":\"${USER_ADDRESS}\",\"amount\":\"${AMOUNT}\",\"condition_id\":\"${CONDITION_ID}\",\"tx_hash\":\"${TX_HASH}\"}"

		# 推送到 Redis Stream
		redis-cli -h ${REDIS_HOST} -p ${REDIS_PORT} -a ${REDIS_PASSWORD} -n ${REDIS_DB} XADD ${STREAM_NAME} "*" ${MSG_KEY} "${MESSAGE}"

		echo "✅ 合并消息已推送"
		echo "   用户ID: ${USER_ID}"
		echo "   金额: ${AMOUNT} (原始值，包含18位精度)"
		echo "   实际金额: 50 USDC"
		echo "   Condition ID: ${CONDITION_ID}"
		;;

	redeem)
		echo "🎁 推送赎回消息到 Redis"

		# 可修改的参数
		USER_ID=1
		PRIVY_ID="cmc1r8lmk014mld0nwn0pwewq"
		USER_ADDRESS="0x1234567890123456789012345678901234567890"
		PAYOUT="200000000000000000000"  # 200 USDC (18位精度)
		CONDITION_ID="0xcondition_id_market_1"
		TX_HASH="0xredeem_tx_hash_002"

		# 构建 JSON 消息（紧凑格式，无换行符）
		MESSAGE="{\"types\":\"redeem\",\"user_id\":${USER_ID},\"privy_id\":\"${PRIVY_ID}\",\"user_address\":\"${USER_ADDRESS}\",\"payout\":\"${PAYOUT}\",\"condition_id\":\"${CONDITION_ID}\",\"tx_hash\":\"${TX_HASH}\"}"

		# 推送到 Redis Stream
		redis-cli -h ${REDIS_HOST} -p ${REDIS_PORT} -a ${REDIS_PASSWORD} -n ${REDIS_DB} XADD ${STREAM_NAME} "*" ${MSG_KEY} "${MESSAGE}"

		echo "✅ 赎回消息已推送"
		echo "   用户ID: ${USER_ID}"
		echo "   赎回金额: ${PAYOUT} (原始值，包含18位精度)"
		echo "   实际金额: 200 USDC"
		echo "   Condition ID: ${CONDITION_ID}"
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
