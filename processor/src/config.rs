use {
	common::logging::LoggingConfig,
	config::{Config, File},
	serde::{Deserialize, Serialize},
	tokio::sync::OnceCell,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessorConfig {
	pub logging: LoggingConfig,
	pub processor_mq: ProcessorMqConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProcessorMqConfig {
	pub consumer_count: usize,
	pub batch_size: usize,
}

pub static CONFIG: OnceCell<ProcessorConfig> = OnceCell::const_new();

pub fn load_config(config_path: &str) -> anyhow::Result<()> {
	let run_mode = &common::common_env::get_common_env().run_mode;

	let config = Config::builder().add_source(File::with_name(&format!("{}/{}", config_path, run_mode)).required(true)).build()?;

	let processor_config: ProcessorConfig = config.try_deserialize()?;
	println!("Configuration loaded for mode: {}", run_mode);
	println!("Configuration: {:?}", processor_config);
	CONFIG.set(processor_config)?;
	check_config()?;
	Ok(())
}

fn check_config() -> anyhow::Result<()> {
	let config = get_config();
	config.logging.check()?;
	Ok(())
}

pub fn get_config() -> &'static ProcessorConfig {
	CONFIG.get().expect("Config not loaded")
}
