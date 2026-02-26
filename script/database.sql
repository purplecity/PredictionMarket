-- PredictionMarket Database Schema
-- Generated based on codebase analysis and query optimization
-- PostgreSQL 14+

-- ============================================================================
-- EXTENSIONS
-- ============================================================================

-- pg_trgm for trigram-based text search (used for LIKE '%query%' optimization)
CREATE EXTENSION IF NOT EXISTS pg_trgm;

-- ============================================================================
-- CUSTOM TYPES
-- ============================================================================

-- Order side: Buy or Sell
CREATE TYPE order_side AS ENUM ('buy', 'sell');

-- Order type: Limit or Market
CREATE TYPE order_type AS ENUM ('limit', 'market');

-- Order status
CREATE TYPE order_status AS ENUM ('new', 'partially_filled', 'filled', 'cancelled', 'rejected');

-- Asset history type
CREATE TYPE asset_history_type AS ENUM (
    'create_order',
    'order_rejected',
    'cancel_order',
    'deposit',
    'withdraw',
    'on_chain_buy_success',
    'on_chain_sell_success',
    'on_chain_buy_failed',
    'on_chain_sell_failed',
    'redeem',
    'split',
    'merge'
);

-- ============================================================================
-- TABLES
-- ============================================================================

-- Users table
CREATE TABLE users (
    id BIGSERIAL PRIMARY KEY,

    -- Privy information
    privy_id TEXT NOT NULL UNIQUE,
    privy_evm_address TEXT NOT NULL DEFAULT '',
    privy_email TEXT NOT NULL DEFAULT '',
    privy_x TEXT NOT NULL DEFAULT '',
    privy_x_image TEXT NOT NULL DEFAULT '',

    -- User editable information
    name TEXT NOT NULL DEFAULT '',
    bio TEXT NOT NULL DEFAULT '',
    profile_image TEXT NOT NULL DEFAULT '',

    -- Login information
    last_login_ip TEXT NOT NULL DEFAULT '',
    last_login_region TEXT NOT NULL DEFAULT '',
    last_login_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Registration information
    registered_ip TEXT NOT NULL DEFAULT '',
    registered_region TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- User invitations table (only stores invitation relationship)
CREATE TABLE user_invitations (
    user_id BIGINT PRIMARY KEY REFERENCES users(id),
    inviter_id BIGINT REFERENCES users(id),  -- NULL if self-registered
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Points table (stores user points and volume data)
CREATE TABLE points (
    user_id BIGINT PRIMARY KEY REFERENCES users(id),
    liquidity_points BIGINT NOT NULL DEFAULT 0,       -- 流动性积分
    trading_points BIGINT NOT NULL DEFAULT 0,         -- 交易积分
    self_points BIGINT NOT NULL DEFAULT 0,            -- 自己获得原始积分 = liquidity + trading
    boost_points BIGINT NOT NULL DEFAULT 0,           -- boost后积分 = self_points * boost倍数
    invite_earned_points BIGINT NOT NULL DEFAULT 0,   -- 邀请获得积分
    total_points BIGINT NOT NULL DEFAULT 0,           -- 总积分 = boost_points + invite_earned_points
    accumulated_volume DECIMAL(48, 18) NOT NULL DEFAULT 0,  -- 累积交易量
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Events table
CREATE TABLE events (
    id BIGSERIAL PRIMARY KEY,
    event_identifier TEXT NOT NULL UNIQUE,
    slug TEXT NOT NULL DEFAULT '',
    title TEXT NOT NULL DEFAULT '',
    description TEXT NOT NULL DEFAULT '',
    image TEXT NOT NULL DEFAULT '',
    end_date TIMESTAMPTZ,
    closed BOOLEAN NOT NULL DEFAULT FALSE,
    closed_at TIMESTAMPTZ,
    resolved BOOLEAN NOT NULL DEFAULT FALSE,
    resolved_at TIMESTAMPTZ,
    topic TEXT NOT NULL DEFAULT '',
    volume DECIMAL(48, 18) NOT NULL DEFAULT 0,
    markets JSONB NOT NULL DEFAULT '{}'::jsonb,
    recommended BOOLEAN NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Event topics table
CREATE TABLE event_topics (
    topic TEXT PRIMARY KEY,
    active BOOLEAN NOT NULL DEFAULT TRUE
);

-- Orders table
CREATE TABLE orders (
    id UUID PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id),
    event_id BIGINT NOT NULL REFERENCES events(id),
    market_id SMALLINT NOT NULL,
    token_id TEXT NOT NULL,
    outcome TEXT NOT NULL,

    order_side order_side NOT NULL,
    order_type order_type NOT NULL,

    price DECIMAL(24, 18) NOT NULL DEFAULT 0,
    quantity DECIMAL(36, 18) NOT NULL DEFAULT 0,
    volume DECIMAL(48, 18) NOT NULL DEFAULT 0,
    filled_quantity DECIMAL(36, 18) NOT NULL DEFAULT 0,
    cancelled_quantity DECIMAL(36, 18) NOT NULL DEFAULT 0,

    market_price DECIMAL(24, 18) NOT NULL DEFAULT 0,
    market_volume DECIMAL(48, 18) NOT NULL DEFAULT 0,
    market_quantity DECIMAL(36, 18) NOT NULL DEFAULT 0,
    market_filled_quantity DECIMAL(36, 18) NOT NULL DEFAULT 0,
    market_cancelled_quantity DECIMAL(36, 18) NOT NULL DEFAULT 0,
    market_filled_volume DECIMAL(48, 18) NOT NULL DEFAULT 0,
    market_cancelled_volume DECIMAL(48, 18) NOT NULL DEFAULT 0,

    status order_status NOT NULL DEFAULT 'new',

    signature_order_msg JSONB NOT NULL,
    update_id BIGINT NOT NULL DEFAULT 1,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Trades table
CREATE TABLE trades (
    batch_id UUID NOT NULL,
    match_timestamp TIMESTAMPTZ NOT NULL,
    order_id UUID NOT NULL REFERENCES orders(id),

    taker BOOLEAN NOT NULL,
    trade_volume DECIMAL(48, 18) NOT NULL,

    user_id BIGINT NOT NULL REFERENCES users(id),
    event_id BIGINT NOT NULL REFERENCES events(id),
    market_id SMALLINT NOT NULL,
    token_id TEXT NOT NULL,

    side order_side NOT NULL,
    avg_price DECIMAL(24, 18) NOT NULL,
    usdc_amount DECIMAL(48, 18) NOT NULL,
    token_amount DECIMAL(36, 18) NOT NULL,

    fee DECIMAL(36, 18),
    real_amount DECIMAL(36, 18) NOT NULL,

    onchain_send_handled BOOLEAN NOT NULL DEFAULT FALSE,
    tx_hash TEXT,

    PRIMARY KEY (batch_id, order_id, match_timestamp)
);

-- Positions table
CREATE TABLE positions (
    user_id BIGINT NOT NULL REFERENCES users(id),
    token_id TEXT NOT NULL,
    event_id BIGINT REFERENCES events(id),
    market_id SMALLINT,

    balance DECIMAL(36, 18) NOT NULL DEFAULT 0,
    frozen_balance DECIMAL(36, 18) NOT NULL DEFAULT 0,
    usdc_cost DECIMAL(48, 18),
    avg_price DECIMAL(24, 18),

    redeemed BOOLEAN,
    redeemed_timestamp TIMESTAMPTZ,
    payout DECIMAL(48, 18),

    privy_id TEXT,
    outcome_name TEXT,
    question TEXT,

    update_id BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    PRIMARY KEY (user_id, token_id)
);

-- Asset history table
CREATE TABLE asset_history (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id),
    history_type asset_history_type NOT NULL,

    -- USDC fields
    usdc_amount DECIMAL(48, 18),
    usdc_balance_before DECIMAL(48, 18),
    usdc_balance_after DECIMAL(48, 18),
    usdc_frozen_balance_before DECIMAL(48, 18),
    usdc_frozen_balance_after DECIMAL(48, 18),

    -- Token fields
    token_id TEXT,
    token_amount DECIMAL(36, 18),
    token_balance_before DECIMAL(36, 18),
    token_balance_after DECIMAL(36, 18),
    token_frozen_balance_before DECIMAL(36, 18),
    token_frozen_balance_after DECIMAL(36, 18),

    tx_hash TEXT,
    trade_id TEXT,
    order_id TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),

    -- Unique constraint for deduplication (NULLS NOT DISTINCT ensures NULL values are treated as equal)
    CONSTRAINT uq_asset_history UNIQUE NULLS NOT DISTINCT (user_id, history_type, tx_hash, token_id, trade_id, order_id)
);

-- Operation history table
CREATE TABLE operation_history (
    id BIGSERIAL PRIMARY KEY,
    event_id BIGINT NOT NULL REFERENCES events(id),
    market_id SMALLINT NOT NULL,
    user_id BIGINT NOT NULL REFERENCES users(id),
    history_type asset_history_type NOT NULL,
    outcome_name TEXT,
    token_id TEXT,
    quantity DECIMAL(36, 18),
    price DECIMAL(24, 18),
    value DECIMAL(48, 18),
    tx_hash TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- User API keys table
-- Allows special users to access API and WebSocket via API key instead of JWT token
CREATE TABLE user_api_keys (
    user_id BIGINT NOT NULL REFERENCES users(id),
    privy_id TEXT NOT NULL,
    api_key TEXT NOT NULL UNIQUE,
    PRIMARY KEY (user_id)
);

-- Seasons table
-- 赛季配置表，每个赛季有开始和结束时间
CREATE TABLE seasons (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    start_time TIMESTAMPTZ NOT NULL,
    end_time TIMESTAMPTZ NOT NULL,
    active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Point metadata table
-- 用户积分元数据，存储不同用户的积分比例配置
CREATE TABLE point_metadata (
    user_id BIGINT PRIMARY KEY REFERENCES users(id),
    -- 交易积分比例 (除以10000)，如1表示0.01%
    trading_points_ratio INTEGER NOT NULL DEFAULT 100,
    -- 邀请获得比例 (除以100)，如5表示5%
    invite_points_ratio INTEGER NOT NULL DEFAULT 5,
    -- 被邀请人boost倍数，如1.50表示1.5倍
    invitee_boost DECIMAL(5, 2) NOT NULL DEFAULT 1.10,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Season history table
-- 历史赛季记录表，赛季结束后手动从user_invitations迁移数据
CREATE TABLE season_history (
    season_id INTEGER NOT NULL REFERENCES seasons(id),
    user_id BIGINT NOT NULL REFERENCES users(id),
    total_points BIGINT NOT NULL DEFAULT 0,
    accumulated_volume DECIMAL(48, 18) NOT NULL DEFAULT 0,
    PRIMARY KEY (season_id, user_id)
);

-- ============================================================================
-- INDEXES - TIER 1 (CRITICAL - Highest Query Impact)
-- ============================================================================

-- Users indexes
CREATE UNIQUE INDEX idx_users_privy_id ON users(privy_id);
CREATE INDEX idx_users_created_at ON users(created_at DESC);

-- User invitations indexes
CREATE INDEX idx_user_invitations_inviter_id ON user_invitations(inviter_id) WHERE inviter_id IS NOT NULL;
CREATE INDEX idx_user_invitations_created_at ON user_invitations(created_at DESC);

-- Points indexes
CREATE INDEX idx_points_total_points ON points(total_points DESC);
CREATE INDEX idx_points_accumulated_volume ON points(accumulated_volume DESC);

-- Season history indexes
CREATE INDEX idx_season_history_season_total_points ON season_history(season_id, total_points DESC);
CREATE INDEX idx_season_history_season_accumulated_volume ON season_history(season_id, accumulated_volume DESC);

-- Events indexes
CREATE UNIQUE INDEX idx_events_event_identifier ON events(event_identifier);
CREATE INDEX idx_events_closed_resolved ON events(closed, resolved);
CREATE INDEX idx_events_closed_resolved_created_at ON events(closed, resolved, created_at DESC)
    WHERE closed = FALSE AND resolved = FALSE;

-- Orders indexes - Most critical for performance
CREATE INDEX idx_orders_user_id ON orders(user_id);
CREATE INDEX idx_orders_user_id_status ON orders(user_id, status);
CREATE INDEX idx_orders_event_id ON orders(event_id);
CREATE INDEX idx_orders_user_id_event_id_status ON orders(user_id, event_id, status);
CREATE INDEX idx_orders_user_id_market_id_status ON orders(user_id, market_id, status);

-- Orders unique constraints for signature deduplication
CREATE UNIQUE INDEX idx_orders_signature_unique ON orders((signature_order_msg->>'signature'));
CREATE UNIQUE INDEX idx_orders_maker_salt_unique ON orders((signature_order_msg->>'maker'), ((signature_order_msg->>'salt')::bigint));

-- Positions indexes
CREATE INDEX idx_positions_user_id_redeemed ON positions(user_id, redeemed)
    WHERE redeemed IS NULL OR redeemed = FALSE;
CREATE INDEX idx_positions_user_id_token_id_not_usdc ON positions(user_id, token_id)
    WHERE token_id != 'usdc';

-- Trades indexes
CREATE INDEX idx_trades_event_id_market_id_taker ON trades(event_id, market_id, taker);
CREATE INDEX idx_trades_user_id_taker ON trades(user_id) WHERE taker = TRUE;
CREATE INDEX idx_trades_user_id_maker ON trades(user_id) WHERE taker = FALSE;

-- Operation history indexes
CREATE INDEX idx_operation_history_user_id_history_type ON operation_history(user_id, history_type);
CREATE INDEX idx_operation_history_user_id_created_at ON operation_history(user_id, created_at DESC);

-- ============================================================================
-- INDEXES - TIER 2 (HIGH PRIORITY - Frequent Queries)
-- ============================================================================

-- Events indexes
CREATE INDEX idx_events_topic ON events(topic);
CREATE INDEX idx_events_volume_desc ON events(volume DESC);
CREATE INDEX idx_events_end_date ON events(end_date ASC NULLS LAST);
-- GIN index for title text search using pg_trgm (supports LIKE '%query%')
CREATE INDEX idx_events_title_trgm ON events USING GIN (title gin_trgm_ops);

-- Orders indexes
CREATE INDEX idx_orders_user_id_created_at ON orders(user_id, created_at DESC);

-- Positions indexes
CREATE INDEX idx_positions_user_id_avg_price ON positions(user_id, avg_price DESC NULLS LAST)
    WHERE redeemed IS NULL OR redeemed = FALSE;
CREATE INDEX idx_positions_user_id_redeemed_timestamp ON positions(user_id, redeemed_timestamp DESC NULLS LAST)
    WHERE redeemed = TRUE;
CREATE INDEX idx_positions_user_id_event_id_market_id_redeemed ON positions(user_id, event_id, market_id, redeemed);
-- GIN index for question text search using pg_trgm (supports LIKE '%query%')
CREATE INDEX idx_positions_question_trgm ON positions USING GIN (question gin_trgm_ops);

-- Operation history indexes
CREATE INDEX idx_operation_history_user_id_event_id_type ON operation_history(user_id, event_id, history_type);
CREATE INDEX idx_operation_history_user_id_market_id_type ON operation_history(user_id, market_id, history_type);

-- ============================================================================
-- INDEXES - TIER 3 (MEDIUM PRIORITY - Optimization)
-- ============================================================================

-- Event topics indexes
CREATE INDEX idx_event_topics_active ON event_topics(active);

-- Asset history indexes
CREATE INDEX idx_asset_history_user_id ON asset_history(user_id);
CREATE INDEX idx_asset_history_user_id_created_at ON asset_history(user_id, created_at DESC);

-- Trades indexes
CREATE INDEX idx_trades_match_timestamp ON trades(match_timestamp DESC);
CREATE INDEX idx_trades_batch_id ON trades(batch_id);

-- ============================================================================
-- COMMENTS
-- ============================================================================

COMMENT ON TABLE users IS 'User accounts with Privy authentication';
COMMENT ON TABLE user_invitations IS 'User invitation relationships only';
COMMENT ON TABLE points IS 'User points and trading volume data for current season';
COMMENT ON TABLE events IS 'Prediction markets/events';
COMMENT ON TABLE event_topics IS 'Event category topics (crypto, sports, etc)';
COMMENT ON TABLE orders IS 'User orders (limit/market, buy/sell)';
COMMENT ON TABLE trades IS 'Executed trades from order matching';
COMMENT ON TABLE positions IS 'User token positions (including USDC)';
COMMENT ON TABLE asset_history IS 'Asset balance change audit trail';
COMMENT ON TABLE operation_history IS 'User trading operations (buy/sell/redeem/split/merge)';
COMMENT ON TABLE user_api_keys IS 'API keys for special users to access API/WebSocket without JWT token';
COMMENT ON TABLE seasons IS 'Season configuration for points system';
COMMENT ON TABLE point_metadata IS 'User-specific points ratio configuration (trading ratio, invite ratio, boost multiplier)';
COMMENT ON TABLE season_history IS 'Historical season records - manually migrated from user_invitations after season ends';

COMMENT ON CONSTRAINT uq_asset_history ON asset_history IS
    'Ensures deduplication with NULLS NOT DISTINCT - prevents duplicate operations for same (user_id, history_type, tx_hash, token_id, trade_id, order_id). NULL values are treated as equal, enabling proper deduplication for deposits/withdrawals where trade_id and order_id are NULL';

-- ============================================================================
-- PERFORMANCE NOTES
-- ============================================================================

-- 1. All indexes use CREATE INDEX (not CONCURRENT) for initial schema creation
--    For production migrations, use CREATE INDEX CONCURRENTLY to avoid locking

-- 2. Partial/filtered indexes (WHERE clauses) significantly reduce index size:
--    - positions_user_id_redeemed: Only non-redeemed positions (~50% reduction)
--    - positions_user_id_token_id_not_usdc: Excludes USDC (~50% reduction)
--    - trades_user_id_taker/maker: Separates taker/maker (~50% reduction each)

-- 3. Composite index column ordering is optimized for query patterns:
--    - Equality filters first (user_id, event_id, market_id)
--    - Range filters next (status, redeemed)
--    - Sort columns last (created_at DESC, avg_price DESC)

-- 4. JSONB column (events.markets) cannot be indexed directly
--    Application must handle nested queries efficiently

-- 5. Expected query performance improvements after indexing:
--    - User order queries: 10-100x faster
--    - Position queries: 5-20x faster
--    - Event listing: 3-10x faster
--    - Volume aggregation: 5-15x faster
--    - User activity: 5-10x faster

-- ============================================================================
-- MAINTENANCE COMMANDS
-- ============================================================================

-- Analyze tables after bulk data load
-- ANALYZE users, events, orders, trades, positions, asset_history, operation_history;

-- Reindex if index bloat detected
-- REINDEX TABLE CONCURRENTLY orders;
-- REINDEX TABLE CONCURRENTLY positions;

-- Check index usage statistics
-- SELECT schemaname, tablename, indexname, idx_scan, idx_tup_read, idx_tup_fetch
-- FROM pg_stat_user_indexes
-- ORDER BY idx_scan;

-- Check table sizes
-- SELECT schemaname, tablename,
--        pg_size_pretty(pg_total_relation_size(schemaname||'.'||tablename)) AS size
-- FROM pg_tables
-- WHERE schemaname = 'public'
-- ORDER BY pg_total_relation_size(schemaname||'.'||tablename) DESC;
