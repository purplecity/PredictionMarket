use {
	common::logging::LoggingConfig,
	config::{Config, File},
	serde::{Deserialize, Serialize},
	tokio::sync::OnceCell,
};

pub static CONFIG: OnceCell<OnchainMsgConfig> = OnceCell::const_new();

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OnchainMsgConfig {
	pub logging: LoggingConfig,
	pub trade_response_mq: ConsumerMqConfig,
	pub onchain_action_mq: ConsumerMqConfig,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ConsumerMqConfig {
	pub consumer_count: usize,
	pub batch_size: usize,
}

pub fn load_config(config_path: &str) -> anyhow::Result<()> {
	let run_mode = &common::common_env::get_common_env().run_mode;

	let config = Config::builder().add_source(File::with_name(&format!("{}/{}", config_path, run_mode)).required(true)).build()?;

	let onchain_msg_config: OnchainMsgConfig = config.try_deserialize()?;
	println!("Configuration loaded for mode: {}", run_mode);
	println!("Configuration: {:?}", onchain_msg_config);
	CONFIG.set(onchain_msg_config)?;
	check_config()?;
	Ok(())
}

fn check_config() -> anyhow::Result<()> {
	let config = get_config();
	config.logging.check()?;
	Ok(())
}

pub fn get_config() -> &'static OnchainMsgConfig {
	CONFIG.get().expect("Config not loaded")
}
