use {
	common::logging::LoggingConfig,
	config::{Config, File},
	serde::{Deserialize, Serialize},
	tokio::sync::OnceCell,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct EventConfig {
	pub logging: LoggingConfig,
}

pub static CONFIG: OnceCell<EventConfig> = OnceCell::const_new();

pub fn load_config(config_path: &str) -> anyhow::Result<()> {
	let run_mode = &common::common_env::get_common_env().run_mode;
	let config = Config::builder().add_source(File::with_name(&format!("{}/{}", config_path, run_mode)).required(true)).build()?;

	let event_config: EventConfig = config.try_deserialize()?;
	println!("Configuration loaded for mode: {}", run_mode);
	println!("Configuration: {:?}", event_config);
	CONFIG.set(event_config)?;
	check_config()?;
	Ok(())
}

fn check_config() -> anyhow::Result<()> {
	let config = get_config();
	config.logging.check()?;
	Ok(())
}

pub fn get_config() -> &'static EventConfig {
	CONFIG.get().expect("Config not loaded")
}
