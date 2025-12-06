#!/bin/bash

# 获取脚本所在目录的父目录（项目根目录）
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"
COMMON_ENV_FILE="$PROJECT_ROOT/deploy/common.env"
ZSHRC_FILE="$HOME/.zshrc"

# 检查 common.env 文件是否存在
if [ ! -f "$COMMON_ENV_FILE" ]; then
    echo "Error: $COMMON_ENV_FILE not found!"
    exit 1
fi

# 检查 .zshrc 文件是否存在，如果不存在则创建
if [ ! -f "$ZSHRC_FILE" ]; then
    touch "$ZSHRC_FILE"
    echo "Created $ZSHRC_FILE"
fi

# 先备份 .zshrc 文件
ZSHRC_BACKUP="${ZSHRC_FILE}.backup.$(date +%Y%m%d_%H%M%S)"
if ! cp "$ZSHRC_FILE" "$ZSHRC_BACKUP"; then
    echo "Error: Failed to backup $ZSHRC_FILE!"
    exit 1
fi
echo "Backed up $ZSHRC_FILE to $ZSHRC_BACKUP"

# 读取 common.env 中的所有变量名
declare -a ENV_KEYS=()
while IFS= read -r line || [ -n "$line" ]; do
    # 跳过空行和注释行
    if [[ -z "$line" || "$line" =~ ^[[:space:]]*# ]]; then
        continue
    fi

    # 如果行包含 =，提取变量名
    if [[ "$line" =~ ^([A-Za-z_][A-Za-z0-9_]*)= ]]; then
        key="${BASH_REMATCH[1]}"
        ENV_KEYS+=("$key")
    fi
done < "$COMMON_ENV_FILE"

echo "Found ${#ENV_KEYS[@]} environment variables in $COMMON_ENV_FILE"

# 创建临时文件
TEMP_FILE=$(mktemp)
trap "rm -f $TEMP_FILE" EXIT

# 从 .zshrc 中删除所有包含 common.env 变量名的 export 行
cp "$ZSHRC_FILE" "$TEMP_FILE"

for key in "${ENV_KEYS[@]}"; do
    # 删除包含该变量名的 export 行
    sed -i '' "/^export.*${key}/d" "$TEMP_FILE"
done

echo "Removed existing export statements for common.env variables"

# 添加 common.env 中的所有环境变量
echo "" >> "$TEMP_FILE"
echo "# Environment variables from $COMMON_ENV_FILE (added on $(date))" >> "$TEMP_FILE"

while IFS= read -r line || [ -n "$line" ]; do
    # 跳过空行和注释行
    if [[ -z "$line" || "$line" =~ ^[[:space:]]*# ]]; then
        continue
    fi

    # 如果行不包含 =，跳过
    if [[ ! "$line" =~ = ]]; then
        continue
    fi

    # 添加 export 前缀
    echo "export $line" >> "$TEMP_FILE"
done < "$COMMON_ENV_FILE"

# 将临时文件内容写回 .zshrc
mv "$TEMP_FILE" "$ZSHRC_FILE"

echo "Updated $ZSHRC_FILE with environment variables from $COMMON_ENV_FILE"
echo "Sourcing $ZSHRC_FILE..."

# 执行 source ~/.zshrc
source "$ZSHRC_FILE"

echo "Done! Environment variables have been set."
echo ""
echo "The following variables were added:"
for key in "${ENV_KEYS[@]}"; do
    echo "  - $key"
done
