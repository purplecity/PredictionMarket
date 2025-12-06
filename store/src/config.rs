use {
	config::{Config, File},
	serde::{Deserialize, Serialize},
	tokio::sync::OnceCell,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StoreConfig {
	pub logging: common::logging::LoggingConfig,
	pub engine_output_mq: EngineOutputMqConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EngineOutputMqConfig {
	pub batch_size: usize,
}

pub static CONFIG: OnceCell<StoreConfig> = OnceCell::const_new();

pub fn load_config(config_path: &str) -> anyhow::Result<()> {
	let run_mode = &common::common_env::get_common_env().run_mode;
	let config = Config::builder().add_source(File::with_name(&format!("{}/{}", config_path, run_mode)).required(true)).build()?;

	let store_config: StoreConfig = config.try_deserialize()?;
	println!("Configuration loaded for mode: {}", run_mode);
	println!("Configuration: {:?}", store_config);
	CONFIG.set(store_config)?;
	check_config()?;
	Ok(())
}

fn check_config() -> anyhow::Result<()> {
	let config = get_config();
	config.logging.check()?;
	Ok(())
}

pub fn get_config() -> &'static StoreConfig {
	CONFIG.get().expect("Config not loaded")
}
