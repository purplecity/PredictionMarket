use {
	serde::{Deserialize, Serialize},
	std::collections::HashMap,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheTokenPriceInfo {
	pub best_bid: String,
	pub best_ask: String,
	pub latest_trade_price: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CacheMarketPriceInfo {
	pub update_id: u64,
	pub timestamp: i64,
	pub prices: HashMap<String, CacheTokenPriceInfo>,
}
