use {
	serde::{Deserialize, Serialize},
	std::{io, path::Path},
	tokio::sync::OnceCell,
	tracing::info,
	tracing_appender::{
		non_blocking::WorkerGuard,
		rolling::{RollingFileAppender, Rotation},
	},
};

// 保持 guard 存活，确保日志缓冲区被刷新到文件
static LOG_GUARD: OnceCell<Box<WorkerGuard>> = OnceCell::const_new();

/// 日志配置结构体（用于从 TOML 配置文件反序列化）
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LoggingConfig {
	pub level: String,
	pub file: Option<String>,
	pub console: bool,
	pub rotation_max_files: usize,
}

impl LoggingConfig {
	/// 检查配置是否有效
	pub fn check(&self) -> anyhow::Result<()> {
		if self.level.is_empty() {
			return Err(anyhow::anyhow!("Logging level is empty"));
		}
		if self.file.is_none() && !self.console {
			return Err(anyhow::anyhow!("Logging file and console are both empty"));
		}
		Ok(())
	}
}

// 当前还没有找到同时往终端和文件打印日志 所以搞成2选一

pub fn init_console_logging(level: &str) -> anyhow::Result<()> {
	tracing_subscriber::fmt().with_env_filter(level).with_writer(io::stdout).with_file(true).with_target(true).with_line_number(true).with_ansi(false).init();

	info!("Console logging system initialized");
	Ok(())
}

pub fn init_file_logging(level: &str, file_path: &str, rotation_max_files: usize) -> anyhow::Result<()> {
	let path = Path::new(file_path);
	let parent = path.parent().expect("Failed to get parent directory");
	if !parent.as_os_str().is_empty() {
		std::fs::create_dir_all(parent)?;
	}
	let file_name = path.file_name().expect("Failed to get file name").to_str().expect("Failed to convert file name to string");
	let file_appender = RollingFileAppender::builder()
		.rotation(Rotation::DAILY)
		.max_log_files(rotation_max_files)
		.filename_prefix(file_name)
		//.filename_suffix("log")
		.build(parent)
		.expect("initializing rolling file appender failed");
	let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
	// 将 guard 存储在静态变量中，确保它在程序运行期间一直存活
	LOG_GUARD.set(Box::new(guard))?;
	tracing_subscriber::fmt().with_env_filter(level).with_writer(non_blocking).with_file(true).with_target(true).with_line_number(true).with_ansi(false).init();

	info!("File logging system initialized");
	Ok(())
}

// 同时打印 但是有一个尴尬的行为 就是如果日志级别是debug那么大于debug的才打印 而且第一次打印的时候是没有往文件打印的 后续打印会有 原因未知 简单点还是2选一吧
// pub fn init_logging(level: u16, file_path: &str, rotation_max_files: usize) -> anyhow::Result<()> {
// 	let path = Path::new(file_path);
// 	let parent = path.parent().expect("Failed to get parent directory");
// 	if !parent.as_os_str().is_empty() {
// 		std::fs::create_dir_all(parent)?;
// 	}
// 	let file_name = path.file_name().expect("Failed to get file name").to_str().expect("Failed to convert file name to string");
// 	let file_appender = RollingFileAppender::builder()
// 		.rotation(Rotation::DAILY)
// 		.max_log_files(rotation_max_files)
// 		.filename_prefix(file_name)
// 		//.filename_suffix("log")
// 		.build(parent)
// 		.expect("initializing rolling file appender failed");
// 	let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
// 	// 将 guard 存储在静态变量中，确保它在程序运行期间一直存活
// 	LOG_GUARD.set(Box::new(guard))?;
// 	let file_log = tracing_subscriber::fmt::layer()
// 		.with_writer(non_blocking)
// 		.with_file(true)
// 		.with_target(true)
// 		.with_line_number(true)
// 		.with_ansi(false)
// 		.with_filter(filter::LevelFilter::from_level(Level::from_str(&level.to_string())?))
// 		.boxed();
// 	let stdout_log = tracing_subscriber::fmt::layer()
// 		.with_writer(io::stdout)
// 		.with_file(true)
// 		.with_target(true)
// 		.with_line_number(true)
// 		.with_ansi(true)
// 		.with_filter(filter::LevelFilter::from_level(Level::from_str(&level.to_string())?))
// 		.boxed();
// 	let mut layers = Vec::new();
// 	layers.push(stdout_log);
// 	layers.push(file_log);
// 	tracing_subscriber::registry().with(layers).init();
// 	Ok(())
// }
