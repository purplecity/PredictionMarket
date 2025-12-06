#!/bin/bash

# PostgreSQL 连接配置
PGHOST="127.0.0.1"
PGPORT="5432"
PGUSER="postgres"
PGPASSWORD="123456"
DBNAME="prediction_market"

# 设置密码环境变量
export PGPASSWORD=$PGPASSWORD

echo "========================================"
echo "开始重置数据库: $DBNAME"
echo "========================================"

# 1. 终止所有连接到目标数据库的活动连接
echo "[1/4] 终止所有连接到数据库的活动连接..."
psql -h $PGHOST -p $PGPORT -U $PGUSER -d postgres -c "
SELECT pg_terminate_backend(pg_stat_activity.pid)
FROM pg_stat_activity
WHERE pg_stat_activity.datname = '$DBNAME'
  AND pid <> pg_backend_pid();" 2>/dev/null

# 2. 删除数据库（如果存在）
echo "[2/4] 删除数据库 $DBNAME (如果存在)..."
psql -h $PGHOST -p $PGPORT -U $PGUSER -d postgres -c "DROP DATABASE IF EXISTS $DBNAME;"

if [ $? -eq 0 ]; then
    echo "✓ 数据库删除成功"
else
    echo "✗ 数据库删除失败"
    exit 1
fi

# 3. 创建新数据库
echo "[3/4] 创建新数据库 $DBNAME..."
psql -h $PGHOST -p $PGPORT -U $PGUSER -d postgres -c "CREATE DATABASE $DBNAME;"

if [ $? -eq 0 ]; then
    echo "✓ 数据库创建成功"
else
    echo "✗ 数据库创建失败"
    exit 1
fi

# 4. 执行 SQL 文件（跳过创建数据库的语句）
echo "[4/4] 执行 database.sql (跳过创建数据库语句)..."

# 获取脚本所在目录
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SQL_FILE="$SCRIPT_DIR/database.sql"

if [ ! -f "$SQL_FILE" ]; then
    echo "✗ SQL 文件不存在: $SQL_FILE"
    exit 1
fi

# 使用 grep 过滤掉 CREATE DATABASE 语句，然后执行
grep -v "^CREATE DATABASE" "$SQL_FILE" | grep -v "^\\c prediction_market" | psql -h $PGHOST -p $PGPORT -U $PGUSER -d $DBNAME

if [ $? -eq 0 ]; then
    echo "✓ SQL 脚本执行成功"
else
    echo "✗ SQL 脚本执行失败"
    exit 1
fi

# 5. 插入初始数据
echo "[5/5] 插入初始数据..."
psql -h $PGHOST -p $PGPORT -U $PGUSER -d $DBNAME -c "INSERT INTO event_topics (topic, active) VALUES ('crypto', true);"

if [ $? -eq 0 ]; then
    echo "✓ 初始数据插入成功"
else
    echo "✗ 初始数据插入失败"
    exit 1
fi

echo ""
echo "========================================"
echo "✓ 数据库重置完成！"
echo "========================================"
echo "数据库名称: $DBNAME"
echo "主机: $PGHOST:$PGPORT"
echo "用户: $PGUSER"
echo ""

# 验证表是否创建成功
echo "验证表创建情况..."
psql -h $PGHOST -p $PGPORT -U $PGUSER -d $DBNAME -c "\dt"

# 清除密码环境变量
unset PGPASSWORD
