/* 不放在axum state中可以用这个
use {
	chrono::Utc,
	dashmap::DashMap,
	serde::{Deserialize, Serialize},
	tokio::{sync::OnceCell, time::Duration},
	tracing::info,
};

// const ONE_SECOND: u128 = 1000000000; // 1秒
const CHECK_INTERVAL: u64 = 1800; // 检查间隔时间-时间秒
const CHECK_NUM: u32 = 200; // 每次检查多少个key
const EXPIRE_TIME: i64 = 14400 * 1000; //key多久没访问了-时间毫秒 取整个key对应所有pattern中最小的时间
// const GLOBAL_KEY_PATTERN: &str = "global_key_pattern";

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
	limit: u32,    //每interval秒多少个请求
	interval: i64, //秒
}

//refill的interval就是interval amount就是limit
pub struct Bucket {
	pub available: u32,        //还剩多少tokens
	pub next_refill_at: i64,   //下次填充时间
	pub refill_amount: u32,    //填充数量 = rule的limit
	pub refill_interval: i64,  //填充间隔时间 = rule的interval
	pub last_access_time: i64, //上次访问时间毫秒
}

/// 其实可以pattern多规则的但是我实在懒得写了 比如单个pattern可以3req/s 30req/min 懒得写了
/// 这个限流器就是为key->pattern->rule的映射
/// 对于我们api来说 就是ip/uid等key 在路由/api/xxx这个pattern上执行了啥限流规则rule
/// 对于单个路由的router来说 pattern也可以当作key就是了 对于全局的限流来说 key和patter固定一个常量就行了
/// 注意如果RATELIMITER不用dashmap 也就是说:pub static RATELIMITER: OnceCell<RwLock<RateLimiter>> = OnceCell::const_new();这种形式的话缺点是共享了同一把锁 都是用RATELIMITER的RwLock去操作的 所以还不如用dashmap算了 异步中混着同步锁特例
pub struct RateLimiter {
	pub rules: DashMap<String, Rule>,                                  //pattern->rule
	pub key_pattern_buckets: DashMap<String, DashMap<String, Bucket>>, //key->pattern->bucket
}

impl Default for RateLimiter {
	fn default() -> Self {
		Self { rules: DashMap::new(), key_pattern_buckets: DashMap::new() }
	}
}

pub static RATELIMITER: OnceCell<RateLimiter> = OnceCell::const_new();

async fn get_ratelimiter() -> &'static RateLimiter {
	RATELIMITER.get_or_init(|| async { RateLimiter { rules: DashMap::new(), key_pattern_buckets: DashMap::new() } }).await
}

//确保limit和duration大于0
pub async fn add_rule(pattern: String, limit: u32, interval: i64) -> anyhow::Result<()> {
	if limit == 0 || interval == 0 {
		return Err(anyhow::anyhow!("limit and duration must be greater than 0"));
	}
	let rate_limiter = get_ratelimiter().await;
	rate_limiter.rules.insert(pattern, Rule { limit, interval });
	Ok(())
}

impl Bucket {
	fn new(limit: u32, interval: i64) -> Self {
		let now = Utc::now().timestamp_millis();
		Self { available: limit, next_refill_at: now + interval * 1000, refill_amount: limit, refill_interval: interval * 1000, last_access_time: now }
	}

	fn refill(&mut self) -> i64 {
		let now = Utc::now().timestamp_millis();
		if now >= self.next_refill_at {
			// 根据时间填充
			self.available = self.refill_amount; //没有最大容量 直接就是填充refill_amount个

			// 已经全部是毫秒
			let intervals = (now - self.next_refill_at) / self.refill_interval + 1;

			// calculate when the following refill would be
			self.next_refill_at += intervals * self.refill_interval;
		}
		now
	}

	fn consume(&mut self) -> bool {
		let now = self.refill();
		if self.available > 0 {
			self.available -= 1;
			self.last_access_time = now;
			true
		} else {
			false
		}
	}
}

pub async fn is_allowed(key: String, pattern: String) -> bool {
	let rate_limiter = get_ratelimiter().await;
	let key_pattern_buckets = rate_limiter.key_pattern_buckets.entry(key.clone()).or_insert_with(DashMap::new);
	match rate_limiter.rules.get(&pattern) {
		Some(rule) => {
			let mut bucket = key_pattern_buckets.entry(pattern.clone()).or_insert_with(|| Bucket::new(rule.limit, rule.interval));
			bucket.consume()
		}
		None => false,
	}
}

pub async fn rate_limit_gc() {
	let mut tokio_interval = tokio::time::interval(Duration::from_secs(CHECK_INTERVAL));
	tokio_interval.tick().await;

	loop {
		tokio_interval.tick().await;

		let rate_limiter = get_ratelimiter().await;
		let now = Utc::now().timestamp_millis();
		let mut delete_keys = Vec::new();
		let mut checked_count = 0;

		for entry in rate_limiter.key_pattern_buckets.iter() {
			// 随机抽取
			if fastrand::bool() {
				continue;
			}
			// 超过检查数量就不检查了 避免全部检查
			if checked_count > CHECK_NUM {
				break;
			}

			if entry.value().iter().all(|pattern_buckets| pattern_buckets.last_access_time + EXPIRE_TIME <= now) {
				delete_keys.push(entry.key().clone());
			}
			checked_count += 1;
		}

		// 删除所有delete key
		rate_limiter.key_pattern_buckets.retain(|key, _| !delete_keys.contains(key));
		info!("rate limit gc deleted {} keys", delete_keys.len());
	}
}
*/

use {
	chrono::Utc,
	dashmap::DashMap,
	serde::{Deserialize, Serialize},
	std::sync::Arc,
	tokio::time::Duration,
	tracing::info,
};

// const ONE_SECOND: u128 = 1000000000; // 1秒
const CHECK_INTERVAL: u64 = 1800; // 检查间隔时间-时间秒
const CHECK_NUM: u32 = 200; // 每次检查多少个key
const EXPIRE_TIME: i64 = 14400 * 1000; //key多久没访问了-时间毫秒 取整个key对应所有pattern中最小的时间
// const GLOBAL_KEY_PATTERN: &str = "global_key_pattern";

#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
	limit: u32,    //每interval秒多少个请求
	interval: i64, //秒
}

//refill的interval就是interval amount就是limit
#[derive(Clone)]
pub struct Bucket {
	pub available: u32,        //还剩多少tokens
	pub next_refill_at: i64,   //下次填充时间
	pub refill_amount: u32,    //填充数量 = rule的limit
	pub refill_interval: i64,  //填充间隔时间 = rule的interval
	pub last_access_time: i64, //上次访问时间毫秒
}

impl Bucket {
	fn new(limit: u32, interval: i64) -> Self {
		let now = Utc::now().timestamp_millis();
		Self { available: limit, next_refill_at: now + interval * 1000, refill_amount: limit, refill_interval: interval * 1000, last_access_time: now }
	}

	fn refill(&mut self) -> i64 {
		let now = Utc::now().timestamp_millis();
		if now >= self.next_refill_at {
			// 根据时间填充
			self.available = self.refill_amount; //没有最大容量 直接就是填充refill_amount个

			// 已经全部是毫秒
			let intervals = (now - self.next_refill_at) / self.refill_interval + 1;

			// calculate when the following refill would be
			self.next_refill_at += intervals * self.refill_interval;
		}
		now
	}

	fn consume(&mut self) -> bool {
		let now = self.refill();
		if self.available > 0 {
			//info!("consume available: {}", self.available);
			self.available -= 1;
			self.last_access_time = now;
			true
		} else {
			false
		}
	}
}

/// 其实可以pattern多规则的但是我实在懒得写了 比如单个pattern可以3req/s 30req/min 懒得写了
/// 这个限流器就是为key->pattern->rule的映射
/// 对于我们api来说 就是ip/uid等key 在路由/api/xxx这个pattern上执行了啥限流规则rule
/// 对于单个路由的router来说 pattern也可以当作key就是了 对于全局的限流来说 key和patter固定一个常量就行了
/// 注意如果RATELIMITER不用dashmap 也就是说:pub static RATELIMITER: OnceCell<RwLock<RateLimiter>> = OnceCell::const_new();这种形式的话缺点是共享了同一把锁 都是用RATELIMITER的RwLock去操作的 所以还不如用dashmap算了 异步中混着同步锁特例
#[derive(Clone)]
pub struct RateLimiter {
	pub rules: DashMap<String, Rule>,                                  //pattern->rule
	pub key_pattern_buckets: DashMap<String, DashMap<String, Bucket>>, //key->pattern->bucket
}

impl Default for RateLimiter {
	fn default() -> Self {
		Self { rules: DashMap::new(), key_pattern_buckets: DashMap::new() }
	}
}

impl RateLimiter {
	// 确保limit和duration大于0
	pub fn add_rule(&mut self, pattern: String, limit: u32, interval: i64) -> anyhow::Result<()> {
		if limit == 0 || interval == 0 {
			return Err(anyhow::anyhow!("limit and duration must be greater than 0"));
		}
		self.rules.insert(pattern, Rule { limit, interval });
		Ok(())
	}

	//大部分api路径对于单个ip/uid来说是相同的规则 比如1req/s
	//显然key除了是uid/ip之外 还可以是api路径表示单个api的总限流 还可以用特殊的key/patter来表示global全局限流 但是这都无关紧要 因为这涉及到机器性能规模问题
	pub fn add_multi_pattern_with_same_rule(&mut self, pattern: &[String], limit: u32, interval: i64) -> anyhow::Result<()> {
		for pattern in pattern.iter() {
			self.add_rule(pattern.to_string(), limit, interval)?;
		}
		Ok(())
	}

	pub fn new_with_multi_pattern_with_same_rule(patterns: &[String], limit: u32, interval: i64) -> anyhow::Result<Self> {
		let mut rate_limiter = Self::default();
		rate_limiter.add_multi_pattern_with_same_rule(patterns, limit, interval)?;
		Ok(rate_limiter)
	}

	pub fn is_allowed(&mut self, key: String, pattern: String) -> bool {
		let key_pattern_buckets = self.key_pattern_buckets.entry(key.clone()).or_default();
		match self.rules.get(&pattern) {
			Some(rule) => {
				let mut bucket = key_pattern_buckets.entry(pattern.clone()).or_insert_with(|| Bucket::new(rule.limit, rule.interval));
				bucket.consume()
			}
			None => false,
		}
	}
}

pub fn is_allowed(rate_limiter: Arc<RateLimiter>, key: String, pattern: String) -> bool {
	let key_pattern_buckets = rate_limiter.key_pattern_buckets.entry(key.clone()).or_default();
	match rate_limiter.rules.get(&pattern) {
		Some(rule) => {
			let mut bucket = key_pattern_buckets.entry(pattern.clone()).or_insert_with(|| Bucket::new(rule.limit, rule.interval));
			bucket.consume()
		}
		None => false,
	}
}

//能直接传arc 因为内部dashmap是线程安全的
pub async fn rate_limit_gc(rate_limiter: Arc<RateLimiter>) {
	let mut tokio_interval = tokio::time::interval(Duration::from_secs(CHECK_INTERVAL));
	tokio_interval.tick().await;

	loop {
		tokio_interval.tick().await;

		let now = Utc::now().timestamp_millis();
		let mut delete_keys = Vec::new();
		let mut checked_count = 0;

		for entry in rate_limiter.key_pattern_buckets.iter() {
			// 随机抽取
			if fastrand::bool() {
				continue;
			}
			// 超过检查数量就不检查了 避免全部检查
			if checked_count > CHECK_NUM {
				break;
			}

			if entry.value().iter().all(|pattern_buckets| pattern_buckets.last_access_time + EXPIRE_TIME <= now) {
				delete_keys.push(entry.key().clone());
			}
			checked_count += 1;
		}

		// 删除所有delete key
		//rate_limiter.key_pattern_buckets.retain(|key, _| !delete_keys.contains(key)); dashmap上retain效率偏低
		for key in &delete_keys {
			rate_limiter.key_pattern_buckets.remove(key);
		}
		info!("rate limit gc deleted {} keys", delete_keys.len());
	}
}
