-- Migration: Remove trace_id, add trade_id and order_id to asset_history
-- Date: 2025-12-01

BEGIN;

-- Drop the old unique constraint that uses trace_id
ALTER TABLE asset_history DROP CONSTRAINT IF EXISTS uq_asset_history_trace;

-- Add new columns
ALTER TABLE asset_history ADD COLUMN trade_id TEXT;
ALTER TABLE asset_history ADD COLUMN order_id TEXT;

-- Drop trace_id column
ALTER TABLE asset_history DROP COLUMN trace_id;

-- Add new unique constraint using user_id, history_type, tx_hash, trade_id, order_id
ALTER TABLE asset_history ADD CONSTRAINT uq_asset_history UNIQUE (user_id, history_type, tx_hash, trade_id, order_id);

COMMIT;
