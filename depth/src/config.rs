use {common::logging::LoggingConfig, serde::Deserialize, tokio::sync::OnceCell};

static CONFIG: OnceCell<Config> = OnceCell::const_new();

#[derive(Debug, Deserialize)]
pub struct Config {
	pub logging: LoggingConfig,
	pub consumer: ConsumerConfig,
}

#[derive(Debug, Deserialize)]
pub struct ConsumerConfig {
	pub batch_size: usize,
}

pub fn load_config(config_path: &str) -> anyhow::Result<()> {
	let run_mode = &common::common_env::get_common_env().run_mode;

	let config_file = format!("{}/{}.toml", config_path, run_mode);
	let config_str = std::fs::read_to_string(&config_file)?;
	let config: Config = toml::from_str(&config_str)?;

	CONFIG.set(config).map_err(|_| anyhow::anyhow!("Config already initialized"))?;
	check_config()?;
	Ok(())
}

fn check_config() -> anyhow::Result<()> {
	let config = get_config();
	config.logging.check()?;
	Ok(())
}

pub fn get_config() -> &'static Config {
	CONFIG.get().expect("Config not initialized")
}
