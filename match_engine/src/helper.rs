use {
	common::consts::PRICE_MULTIPLIER,
	rust_decimal::{Decimal, prelude::FromPrimitive},
};

pub fn format_price(price: i32) -> String {
	let price = Decimal::from_i32(price).unwrap_or(Decimal::ZERO);
	price.checked_div(Decimal::from(PRICE_MULTIPLIER)).unwrap_or(Decimal::ZERO).normalize().to_string()
}
pub fn price_decimal(price: i32) -> Decimal {
	Decimal::from_i32(price).unwrap_or(Decimal::ZERO).checked_div(Decimal::from(PRICE_MULTIPLIER)).unwrap_or(Decimal::ZERO)
}

pub fn format_quantity(quantity: u64) -> String {
	let quantity = Decimal::from_u64(quantity).unwrap_or(Decimal::ZERO);
	quantity.checked_div(Decimal::ONE_HUNDRED).unwrap_or(Decimal::ZERO).normalize().to_string()
}

pub fn quantity_decimal(quantity: u64) -> Decimal {
	Decimal::from_u64(quantity).unwrap_or(Decimal::ZERO).checked_div(Decimal::ONE_HUNDRED).unwrap_or(Decimal::ZERO)
}

pub fn get_usdc_amount(price: Decimal, quantity: Decimal) -> String {
	price.checked_mul(quantity).unwrap_or(Decimal::ZERO).normalize().to_string()
}

use std::collections::{HashSet, VecDeque};

///用于检查消息重复 如果是消息id重复或者消息中业务不允许的重复比如重复订单 放在Manager中
///但是我们这个demo当前用不着
pub struct SlidingWindowDedup {
	buckets: VecDeque<HashSet<String>>, // 最多 60 个
}

impl SlidingWindowDedup {
	pub fn new(window_hours: u64) -> Self {
		let window_minutes = (window_hours * 60) as usize;
		let buckets = VecDeque::with_capacity(window_minutes);
		Self { buckets }
	}

	pub fn is_duplicate(&mut self, msg_id: &str) -> bool {
		for bucket in &self.buckets {
			if !bucket.is_empty() && bucket.contains(msg_id) {
				return true;
			}
		}
		false
	}

	pub fn record(&mut self, msg_id: String) {
		// 插入到最新桶
		if let Some(last) = self.buckets.back_mut() {
			last.insert(msg_id);
		}
	}

	/// 每分钟调用一次：滚动窗口
	pub fn rotate(&mut self) {
		self.buckets.pop_front(); // 丢弃最老的 1 分钟
		self.buckets.push_back(HashSet::new()); // 新增当前分钟
	}
}
