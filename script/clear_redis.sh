#!/bin/bash

# Redis 连接配置
REDIS_HOST="127.0.0.1"
REDIS_PORT="8889"
REDIS_PASSWORD="123456"

# Redis DB 配置（根据 common/src/consts.rs）
DBS="0:ENGINE_INPUT_MQ
1:ENGINE_OUTPUT_MQ
2:WEBSOCKET_MQ
3:COMMON_MQ
4:CACHE
5:LOCK"

echo "========================================"
echo "开始清除所有 Redis DB 的 key"
echo "========================================"
echo "Redis 服务器: $REDIS_HOST:$REDIS_PORT"
echo ""

# 统计总共删除的 key 数量
TOTAL_DELETED=0

echo "$DBS" | while IFS=: read -r DB_NUM DB_NAME; do
    echo "----------------------------------------"
    echo "处理 DB $DB_NUM ($DB_NAME)..."

    # 检查 DB 中的 key 数量
    KEY_COUNT=$(redis-cli -h $REDIS_HOST -p $REDIS_PORT -a $REDIS_PASSWORD -n $DB_NUM DBSIZE 2>/dev/null | grep -o '[0-9]*')

    if [ -z "$KEY_COUNT" ]; then
        echo "✗ 无法连接到 Redis DB $DB_NUM"
        continue
    fi

    if [ "$KEY_COUNT" -eq 0 ]; then
        echo "ℹ DB $DB_NUM 中没有 key，跳过"
        continue
    fi

    echo "  发现 $KEY_COUNT 个 key"

    # 清空当前 DB
    redis-cli -h $REDIS_HOST -p $REDIS_PORT -a $REDIS_PASSWORD -n $DB_NUM FLUSHDB 2>/dev/null > /dev/null

    if [ $? -eq 0 ]; then
        echo "✓ DB $DB_NUM 清空成功，删除了 $KEY_COUNT 个 key"
    else
        echo "✗ DB $DB_NUM 清空失败"
    fi
done

echo ""
echo "========================================"
echo "✓ Redis 清理完成！"
echo "========================================"

# 验证清理结果
echo ""
echo "验证清理结果..."
echo "$DBS" | while IFS=: read -r DB_NUM DB_NAME; do
    KEY_COUNT=$(redis-cli -h $REDIS_HOST -p $REDIS_PORT -a $REDIS_PASSWORD -n $DB_NUM DBSIZE 2>/dev/null | grep -o '[0-9]*')
    echo "  DB $DB_NUM ($DB_NAME): $KEY_COUNT keys"
done

echo ""
echo "注意: Redis Stream 的 consumer group 信息也已被清除"
echo "首次启动服务时会自动重新创建 consumer group"
