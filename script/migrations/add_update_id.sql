-- 为 orders 表添加 update_id 字段
ALTER TABLE orders ADD COLUMN IF NOT EXISTS update_id BIGINT NOT NULL DEFAULT 1;

-- 为 positions 表添加 update_id 字段
ALTER TABLE positions ADD COLUMN IF NOT EXISTS update_id BIGINT NOT NULL DEFAULT 1;
