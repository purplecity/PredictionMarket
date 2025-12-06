use {
	common::logging::LoggingConfig,
	config::{Config, File},
	serde::{Deserialize, Serialize},
	tokio::sync::OnceCell,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct MatchEngineConfig {
	//logging
	pub logging: LoggingConfig,
	pub engine_input_mq: EngineInputMqConfig,
	pub engine_output_mq: EngineOutputMqConfig,
	pub engine: EngineConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EngineInputMqConfig {
	pub order_input_consumer_count: usize,
	pub order_input_batch_size: usize,
	pub event_input_batch_size: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EngineOutputMqConfig {
	pub output_task_count: usize,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EngineConfig {
	pub engine_max_order_count: u64,
}

pub static CONFIG: OnceCell<MatchEngineConfig> = OnceCell::const_new();

pub fn load_config(config_path: &str) -> anyhow::Result<()> {
	let run_mode = &common::common_env::get_common_env().run_mode;
	let config = Config::builder().add_source(File::with_name(&format!("{}/{}", config_path, run_mode)).required(true)).build()?;
	let match_engine_config: MatchEngineConfig = config.try_deserialize()?;
	println!("Configuration loaded for mode: {}", run_mode);
	println!("Configuration: {:?}", match_engine_config);
	CONFIG.set(match_engine_config)?;
	check_config()?;
	Ok(())
}

fn check_config() -> anyhow::Result<()> {
	let config = get_config();
	config.logging.check()?;
	Ok(())
}

pub fn get_config() -> &'static MatchEngineConfig {
	CONFIG.get().expect("Config not loaded")
}
