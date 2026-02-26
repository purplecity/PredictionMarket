use {
	crate::{calculator, config::get_config, db, init::get_shutdown_receiver},
	chrono::{Duration, Utc},
	common::model::Season,
	tracing::{error, info},
};

/// 启动定时调度器
/// 每天 UTC 指定时间执行积分计算
pub async fn start_scheduler() -> anyhow::Result<()> {
	let config = get_config();
	let target_hour = config.scheduler.utc_hour;
	let target_minute = config.scheduler.utc_minute;

	info!("Scheduler started, will run daily at UTC {:02}:{:02}", target_hour, target_minute);

	let mut shutdown_receiver = get_shutdown_receiver();

	loop {
		// 计算下次执行时间
		let now = Utc::now();
		let next_run = calculate_next_run_time(now, target_hour, target_minute);
		let wait_duration = (next_run - now).to_std().unwrap_or(std::time::Duration::from_secs(60));

		info!("Next calculation scheduled at: {} (in {:?})", next_run, wait_duration);

		tokio::select! {
			_ = tokio::time::sleep(wait_duration) => {
				info!("Starting scheduled points calculation");
				if let Err(e) = run_calculation().await {
					error!("Scheduled calculation failed: {}", e);
				}
			}
			_ = shutdown_receiver.recv() => {
				info!("Scheduler received shutdown signal");
				break;
			}
		}
	}

	Ok(())
}

/// 计算下次执行时间
fn calculate_next_run_time(now: chrono::DateTime<Utc>, target_hour: u32, target_minute: u32) -> chrono::DateTime<Utc> {
	let today_target = now.date_naive().and_hms_opt(target_hour, target_minute, 0).expect("valid hour and minute").and_utc();

	if now < today_target {
		// 今天的目标时间还没到
		today_target
	} else {
		// 今天已过，等待明天
		today_target + Duration::days(1)
	}
}

/// 执行积分计算
async fn run_calculation() -> anyhow::Result<()> {
	// 获取活跃赛季
	let season = db::get_active_season().await?;

	match season {
		Some(s) => {
			let now = Utc::now();

			// 检查是否在赛季时间范围内
			if now < s.start_time {
				info!("Season {} hasn't started yet (starts at {})", s.name, s.start_time);
				return Ok(());
			}

			if now >= s.end_time {
				info!("Season {} has ended (ended at {})", s.name, s.end_time);
				return Ok(());
			}

			info!("Running calculation for season: {} ({} to {})", s.name, s.start_time, s.end_time);
			calculator::calculate_season_points(s.start_time, s.end_time).await?;

			Ok(())
		}
		None => {
			info!("No active season found, skipping calculation");
			Ok(())
		}
	}
}

/// 启动赛季结束任务
/// 睡眠直到赛季结束后执行最终统计并迁移数据到历史记录表
pub async fn start_season_end_task() -> anyhow::Result<()> {
	let mut shutdown_receiver = get_shutdown_receiver();

	loop {
		// 获取活跃赛季
		let season = db::get_active_season().await?;

		match season {
			Some(s) => {
				let now = Utc::now();

				if now >= s.end_time {
					// 赛季已结束，可能已手动处理或已处理过，直接退出
					info!("Season {} has already ended (ended at {}), skipping", s.name, s.end_time);
					return Ok(());
				}

				// 计算需要睡眠的时间
				let wait_duration = (s.end_time - now).to_std().unwrap_or(std::time::Duration::from_secs(60));
				info!("Season end task: will run final calculation for season {} at {} (in {:?})", s.name, s.end_time, wait_duration);

				tokio::select! {
					_ = tokio::time::sleep(wait_duration) => {
						info!("Season {} end time reached, running final calculation", s.name);
						if let Err(e) = run_season_end_processing(&s).await {
							error!("Season end processing failed: {}", e);
						}
					}
					_ = shutdown_receiver.recv() => {
						info!("Season end task received shutdown signal");
						break;
					}
				}
			}
			None => {
				info!("Season end task: no active season found, waiting...");
				// 没有活跃赛季，等待一段时间后重试
				tokio::select! {
					_ = tokio::time::sleep(std::time::Duration::from_secs(3600)) => {
						// 每小时检查一次是否有新赛季
					}
					_ = shutdown_receiver.recv() => {
						info!("Season end task received shutdown signal");
						break;
					}
				}
			}
		}
	}

	Ok(())
}

/// 执行赛季结束处理：最终统计 + 迁移数据到历史记录表
async fn run_season_end_processing(season: &Season) -> anyhow::Result<()> {
	info!("Starting season end processing for season: {} (id={})", season.name, season.id);

	// 1. 执行最终积分计算（包含所有状态的订单）
	calculator::calculate_season_points(season.start_time, season.end_time).await?;
	info!("Final points calculation completed for season {}", season.id);

	// 2. 获取所有用户的积分数据
	let all_users = db::get_all_user_points().await?;
	info!("Found {} users to migrate to season history", all_users.len());

	// 3. 逐个写入赛季历史记录表
	let mut success_count = 0;
	for user in &all_users {
		if let Err(e) = db::insert_season_history(season.id, user).await {
			error!("Failed to insert season history for user {}: {}", user.user_id, e);
		} else {
			success_count += 1;
		}
	}
	info!("Migrated {} users to season history for season {}", success_count, season.id);

	// 4. 将赛季设为非活跃
	db::deactivate_season(season.id).await?;

	info!("Season end processing completed for season {}", season.id);
	Ok(())
}
