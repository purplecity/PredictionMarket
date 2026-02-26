use {
	common::logging::LoggingConfig,
	config::{Config, File},
	serde::{Deserialize, Serialize},
	tokio::sync::OnceCell,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatisticsConfig {
	pub logging: LoggingConfig,
	pub scheduler: SchedulerConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SchedulerConfig {
	/// UTC 时区小时 (0-23)，默认 0 表示 UTC 0:00
	pub utc_hour: u32,
	/// UTC 时区分钟 (0-59)，默认 0
	pub utc_minute: u32,
}

pub static CONFIG: OnceCell<StatisticsConfig> = OnceCell::const_new();

pub fn load_config(config_path: &str) -> anyhow::Result<()> {
	let run_mode = &common::common_env::get_common_env().run_mode;

	let config = Config::builder().add_source(File::with_name(&format!("{}/{}", config_path, run_mode)).required(true)).build()?;

	let statistics_config: StatisticsConfig = config.try_deserialize()?;
	println!("Configuration loaded for mode: {}", run_mode);
	println!("Configuration: {:?}", statistics_config);
	CONFIG.set(statistics_config)?;
	check_config()?;
	Ok(())
}

fn check_config() -> anyhow::Result<()> {
	let config = get_config();
	config.logging.check()?;

	// 检查调度配置
	if config.scheduler.utc_hour > 23 {
		anyhow::bail!("scheduler.utc_hour must be 0-23");
	}
	if config.scheduler.utc_minute > 59 {
		anyhow::bail!("scheduler.utc_minute must be 0-59");
	}

	Ok(())
}

pub fn get_config() -> &'static StatisticsConfig {
	CONFIG.get().expect("Config not loaded")
}
