use {
	crate::{
		consts::{
			DEFAULT_INVITE_POINTS_RATIO, DEFAULT_INVITEE_BOOST, DEFAULT_TRADING_POINTS_RATIO, INVITE_RATIO_DIVISOR, MAX_POINTS_PER_PRICE_LEVEL, MIN_ORDER_DURATION_SECS, MIN_ORDER_USDC_AMOUNT,
			MIN_TRADING_VOLUME_USDC, TRADING_RATIO_DIVISOR,
		},
		db::{self, UserPointMetadata, UserPointsUpdate},
	},
	chrono::{DateTime, Utc},
	rust_decimal::Decimal,
	std::{collections::HashMap, str::FromStr},
	tracing::{error, info},
};

// ============================================================================
// Decimal checked arithmetic helpers
// ============================================================================

fn checked_add(a: Decimal, b: Decimal) -> anyhow::Result<Decimal> {
	a.checked_add(b).ok_or_else(|| anyhow::anyhow!("Decimal addition overflow: {} + {}", a, b))
}

fn checked_mul(a: Decimal, b: Decimal) -> anyhow::Result<Decimal> {
	a.checked_mul(b).ok_or_else(|| anyhow::anyhow!("Decimal multiplication overflow: {} * {}", a, b))
}

/// 计算赛季积分
pub async fn calculate_season_points(season_start: DateTime<Utc>, season_end: DateTime<Utc>) -> anyhow::Result<()> {
	info!("Starting points calculation for season: {} to {}", season_start, season_end);

	// 1. 获取所有用户邀请关系
	let user_inviters = db::get_all_user_inviter_info().await?;
	info!("Found {} users with invitation records", user_inviters.len());

	if user_inviters.is_empty() {
		info!("No users to process");
		return Ok(());
	}

	// 2. 获取所有用户的积分元数据
	let all_metadata = db::get_all_user_point_metadata().await?;
	let metadata_map: HashMap<i64, UserPointMetadata> = all_metadata.into_iter().map(|m| (m.user_id, m)).collect();
	info!("Loaded {} user point metadata records", metadata_map.len());

	// 3. 计算流动性积分
	let min_volume = Decimal::from_str(MIN_ORDER_USDC_AMOUNT)?;
	let liquidity_orders = db::get_liquidity_orders(season_start, season_end, MIN_ORDER_DURATION_SECS, min_volume).await?;
	let liquidity_points_map = calculate_liquidity_points(&liquidity_orders)?;
	info!("Calculated liquidity points for {} users", liquidity_points_map.len());

	// 4. 计算交易量和交易积分
	let trading_volumes = db::get_user_trading_volumes(season_start, season_end).await?;
	let min_trading_volume = Decimal::from_str(MIN_TRADING_VOLUME_USDC)?;
	let (trading_points_map, volume_map) = calculate_trading_points(&trading_volumes, &metadata_map, min_trading_volume);
	info!("Calculated trading points for {} users", trading_points_map.len());

	// 5. 计算 self_points, boost_points, invite_earned_points, total_points
	let updates = calculate_final_points(&user_inviters, &metadata_map, &liquidity_points_map, &trading_points_map, &volume_map)?;
	info!("Calculated final points for {} users", updates.len());

	// 6. 批量更新数据库
	db::batch_update_user_points(&updates).await?;

	info!("Points calculation completed successfully");
	Ok(())
}

/// 计算流动性积分
/// 按价格档位分组，每个档位最多 MAX_POINTS_PER_PRICE_LEVEL 积分
fn calculate_liquidity_points(orders: &[db::LiquidityOrderInfo]) -> anyhow::Result<HashMap<i64, i64>> {
	// 按用户分组，然后按价格档位分组
	let mut user_price_volumes: HashMap<i64, HashMap<String, Decimal>> = HashMap::new();

	for order in orders {
		let user_volumes = user_price_volumes.entry(order.user_id).or_default();
		// 使用价格字符串作为档位key
		let price_key = order.price.to_string();
		let current = *user_volumes.get(&price_key).unwrap_or(&Decimal::ZERO);
		let new_value = checked_add(current, order.volume)?;
		user_volumes.insert(price_key, new_value);
	}

	// 计算每个用户的流动性积分
	let mut result: HashMap<i64, i64> = HashMap::new();

	for (user_id, price_volumes) in user_price_volumes {
		let mut total_points: i64 = 0;

		for (_price, volume) in price_volumes {
			// 每个价格档位：积分 = volume（向下取整），最多 MAX_POINTS_PER_PRICE_LEVEL
			let volume_i64 = volume.floor().to_string().parse::<i64>().unwrap_or(0);
			let level_points = volume_i64.min(MAX_POINTS_PER_PRICE_LEVEL);
			total_points = total_points.saturating_add(level_points);
		}

		result.insert(user_id, total_points);
	}

	Ok(result)
}

/// 计算交易积分和累积交易量
fn calculate_trading_points(volumes: &[db::UserTradingVolume], metadata_map: &HashMap<i64, UserPointMetadata>, min_volume: Decimal) -> (HashMap<i64, i64>, HashMap<i64, Decimal>) {
	let mut trading_points: HashMap<i64, i64> = HashMap::new();
	let mut volume_map: HashMap<i64, Decimal> = HashMap::new();

	for vol in volumes {
		volume_map.insert(vol.user_id, vol.total_volume);

		// 如果累积交易量 >= MIN_TRADING_VOLUME_USDC
		if vol.total_volume >= min_volume {
			// 获取用户的交易积分比例（默认 1/10000 = 0.01%）
			let ratio = metadata_map.get(&vol.user_id).map(|m| m.trading_points_ratio).unwrap_or(DEFAULT_TRADING_POINTS_RATIO);

			// trading_points = accumulated_volume * trading_points_ratio / 10000
			let volume_i64 = vol.total_volume.floor().to_string().parse::<i64>().unwrap_or(0);
			let points = volume_i64 * (ratio as i64) / TRADING_RATIO_DIVISOR;
			trading_points.insert(vol.user_id, points);
		}
	}

	(trading_points, volume_map)
}

/// 计算最终积分（self_points, boost_points, invite_earned_points, total_points）
fn calculate_final_points(
	user_inviters: &[db::UserInviterInfo],
	metadata_map: &HashMap<i64, UserPointMetadata>,
	liquidity_points_map: &HashMap<i64, i64>,
	trading_points_map: &HashMap<i64, i64>,
	volume_map: &HashMap<i64, Decimal>,
) -> anyhow::Result<Vec<UserPointsUpdate>> {
	let default_boost = Decimal::from_str(DEFAULT_INVITEE_BOOST).expect("DEFAULT_INVITEE_BOOST is a valid decimal");

	// 先计算每个用户的 self_points
	let mut self_points_map: HashMap<i64, i64> = HashMap::new();
	for user_info in user_inviters {
		let liquidity = *liquidity_points_map.get(&user_info.user_id).unwrap_or(&0);
		let trading = *trading_points_map.get(&user_info.user_id).unwrap_or(&0);
		self_points_map.insert(user_info.user_id, liquidity.saturating_add(trading));
	}

	// 构建邀请人到被邀请人的映射
	let mut inviter_to_invitees: HashMap<i64, Vec<i64>> = HashMap::new();
	for user_info in user_inviters {
		if let Some(inviter_id) = user_info.inviter_id {
			inviter_to_invitees.entry(inviter_id).or_default().push(user_info.user_id);
		}
	}

	// 计算每个用户的最终积分
	let mut updates = Vec::new();

	for user_info in user_inviters {
		let liquidity_points = *liquidity_points_map.get(&user_info.user_id).unwrap_or(&0);
		let trading_points = *trading_points_map.get(&user_info.user_id).unwrap_or(&0);
		let self_points = *self_points_map.get(&user_info.user_id).unwrap_or(&0);

		// 计算 boost_points
		let boost_points = if user_info.inviter_id.is_some() {
			// 被邀请的用户：boost_points = self_points * invitee_boost
			let boost_multiplier = metadata_map.get(&user_info.user_id).map(|m| m.invitee_boost).unwrap_or(default_boost);
			let self_points_decimal = Decimal::from(self_points);
			let boosted = checked_mul(self_points_decimal, boost_multiplier)?;
			boosted.floor().to_string().parse::<i64>().unwrap_or(self_points)
		} else {
			// 自注册用户：boost_points = self_points
			self_points
		};

		// 计算 invite_earned_points（作为邀请人获得的积分）
		let invite_earned_points = if let Some(invitees) = inviter_to_invitees.get(&user_info.user_id) {
			// 邀请人的 invite_earned_points = SUM(被邀请人的 self_points) * invite_points_ratio / 100
			let invitees_self_points_sum: i64 = invitees.iter().filter_map(|invitee_id| self_points_map.get(invitee_id)).sum();

			let invite_ratio = metadata_map.get(&user_info.user_id).map(|m| m.invite_points_ratio).unwrap_or(DEFAULT_INVITE_POINTS_RATIO);
			invitees_self_points_sum * (invite_ratio as i64) / INVITE_RATIO_DIVISOR
		} else {
			0
		};

		// total_points = boost_points + invite_earned_points
		let total_points = boost_points.saturating_add(invite_earned_points);

		let accumulated_volume = *volume_map.get(&user_info.user_id).unwrap_or(&Decimal::ZERO);

		updates.push(UserPointsUpdate { user_id: user_info.user_id, liquidity_points, trading_points, self_points, boost_points, invite_earned_points, total_points, accumulated_volume });
	}

	Ok(updates)
}

/// 手动触发计算（用于测试或管理）
pub async fn trigger_calculation() -> anyhow::Result<()> {
	let season = db::get_active_season().await?;

	match season {
		Some(s) => {
			info!("Found active season: {} ({} to {})", s.name, s.start_time, s.end_time);
			calculate_season_points(s.start_time, s.end_time).await
		}
		None => {
			error!("No active season found");
			Ok(())
		}
	}
}
