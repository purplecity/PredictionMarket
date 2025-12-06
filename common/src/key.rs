// Redis key and field generation utilities

/// Price cache hash key (used with HSET/HGET)
pub const PRICE_CACHE_KEY: &str = "price";

/// Depth cache hash key (used with HSET/HGET)
pub const DEPTH_CACHE_KEY: &str = "depth";

/// Volume cache hash key (used with HSET/HGET)
pub const VOLUME_CACHE_KEY: &str = "volume";

/// Generate field for price/depth cache: event_id::market_id
pub fn market_field(event_id: i64, market_id: i16) -> String {
	format!("{}::{}", event_id, market_id)
}
