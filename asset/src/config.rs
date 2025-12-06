use {
	common::logging::LoggingConfig,
	config::{Config, File},
	serde::{Deserialize, Serialize},
	tokio::sync::OnceCell,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct AssetConfig {
	pub logging: LoggingConfig,
	pub grpc: GrpcConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GrpcConfig {
	pub port: u16,
}

impl GrpcConfig {
	pub fn get_addr(&self) -> String {
		format!("0.0.0.0:{}", self.port)
	}
}

pub static CONFIG: OnceCell<AssetConfig> = OnceCell::const_new();

pub fn load_config(config_path: &str) -> anyhow::Result<()> {
	let run_mode = &common::common_env::get_common_env().run_mode;
	let config = Config::builder().add_source(File::with_name(&format!("{}/{}", config_path, run_mode)).required(true)).build()?;

	let asset_config: AssetConfig = config.try_deserialize()?;
	println!("Configuration loaded for mode: {}", run_mode);
	println!("Configuration: {:?}", asset_config);
	CONFIG.set(asset_config)?;
	check_config()?;
	Ok(())
}

fn check_config() -> anyhow::Result<()> {
	let config = get_config();
	config.logging.check()?;
	Ok(())
}

pub fn get_config() -> &'static AssetConfig {
	CONFIG.get().expect("Config not loaded")
}
