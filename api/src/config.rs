use {
	common::logging::LoggingConfig,
	config::{Config, File},
	serde::{Deserialize, Serialize},
	tokio::sync::OnceCell,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ApiConfig {
	pub logging: LoggingConfig,
	pub server: ServerConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
	pub port: u16,
}

impl ServerConfig {
	pub fn get_addr(&self) -> String {
		format!("0.0.0.0:{}", self.port)
	}
}

pub static CONFIG: OnceCell<ApiConfig> = OnceCell::const_new();

pub fn load_config(config_path: &str) -> anyhow::Result<()> {
	let run_mode = &common::common_env::get_common_env().run_mode;

	let config = Config::builder().add_source(File::with_name(&format!("{}/{}", config_path, run_mode)).required(true)).build()?;

	let api_config: ApiConfig = config.try_deserialize()?;
	println!("Configuration loaded for mode: {}", run_mode);
	println!("Configuration: {:?}", api_config);
	CONFIG.set(api_config)?;
	check_config()?;
	Ok(())
}

fn check_config() -> anyhow::Result<()> {
	let config = get_config();
	config.logging.check()?;
	Ok(())
}

pub fn get_config() -> &'static ApiConfig {
	CONFIG.get().expect("Config not loaded")
}
