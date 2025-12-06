use {
	anyhow::Result,
	reqwest::Client,
	serde::{Deserialize, Serialize},
	std::time::Duration,
	tokio::sync::OnceCell,
};

pub static CLIENT: OnceCell<Client> = OnceCell::const_new();
pub fn init_client() -> anyhow::Result<()> {
	let client = Client::builder()
		.timeout(Duration::from_secs(30)) // 30秒超时
		.connect_timeout(Duration::from_secs(10)) // 10秒连接超时
		.build()?;

	CLIENT.set(client).map_err(|_| anyhow::anyhow!("Client already initialized"))?;
	Ok(())
}

pub fn get_client() -> &'static Client {
	CLIENT.get().expect("Client not initialized")
}

const INTERNAL_IP_INFO_PATH: &str = "/internal/ip_info";
const INTERNAL_PRIVY_USER_INFO_PATH: &str = "/internal/privy_user_info";
const INTERNAL_GOOGLE_IMAGE_SIGN_PATH: &str = "/internal/google_image_sign";

/// 内部服务统一返回格式
/// 对应Go代码中的 Response 格式：{code, msg, data}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InternalServiceResponse<T> {
	#[serde(rename = "code")]
	pub code: i32,
	#[serde(rename = "msg")]
	pub msg: String,
	#[serde(rename = "data")]
	pub data: Option<T>,
}

impl<T> InternalServiceResponse<T> {
	/// 检查响应是否成功（code == 0）
	pub fn is_success(&self) -> bool {
		self.code == 0
	}

	/// 将响应转换为Result，如果失败则返回错误
	pub fn into_result(self) -> Result<T> {
		if self.is_success() { self.data.ok_or_else(|| anyhow::anyhow!("Response data is None")) } else { Err(anyhow::anyhow!("Internal service error: code={}, msg={}", self.code, self.msg)) }
	}
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReqGetIpInfo {
	#[serde(rename = "ip")]
	pub ip: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResGetIpInfo {
	#[serde(rename = "region")]
	pub region: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReqGetPrivyUserInfo {
	#[serde(rename = "privy_id")]
	pub privy_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResGetPrivyUserInfo {
	#[serde(rename = "privy_id")]
	pub privy_id: String,
	#[serde(rename = "privy_email")]
	pub privy_email: String,
	#[serde(rename = "privy_x")]
	pub privy_x: String,
	#[serde(rename = "privy_evm_address")]
	pub privy_evm_address: String,
	#[serde(rename = "privy_x_profile_image")]
	pub privy_x_profile_image: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReqGetGoogleImageSign {
	#[serde(rename = "uid")]
	pub uid: String,
	#[serde(rename = "bucket_type")]
	pub bucket_type: String,
	#[serde(rename = "image_type")]
	pub image_type: String,
}

/// SignURLData defines the complete response data for the generate_sign_url API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResGetGoogleImageSign {
	#[serde(rename = "sign_url")]
	pub sign_url: Option<SignURLResponse>,
	#[serde(rename = "info")]
	pub info: Option<SignURLInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignURLResponse {
	#[serde(rename = "url")]
	pub url: String,
	#[serde(rename = "expires_at")]
	pub expires_at: i64,
}

/// SignURLInfo defines additional information about the signed URL
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignURLInfo {
	#[serde(rename = "object_name")]
	pub object_name: String,
	#[serde(rename = "bucket")]
	pub bucket: String,
	#[serde(rename = "content_type")]
	pub content_type: String,
	#[serde(rename = "max_size")]
	pub max_size: String,
	#[serde(rename = "public_url")]
	pub public_url: String,
}

/// 获取IP区域信息
pub async fn get_ip_info(req: ReqGetIpInfo) -> Result<ResGetIpInfo> {
	let internal_service_host = &common::common_env::get_common_env().internal_service_host;
	let url = format!("{}{}", internal_service_host, INTERNAL_IP_INFO_PATH);

	let client = get_client();
	let response = client.get(&url).query(&req).send().await?;

	if !response.status().is_success() {
		return Err(anyhow::anyhow!("Failed to get ip info: status={}, body={}", response.status(), response.text().await.unwrap_or_default()));
	}

	let result: InternalServiceResponse<ResGetIpInfo> = response.json().await?;
	result.into_result()
}

/// 获取Privy用户信息
pub async fn get_privy_user_info(req: ReqGetPrivyUserInfo) -> Result<ResGetPrivyUserInfo> {
	let internal_service_host = &common::common_env::get_common_env().internal_service_host;
	let url = format!("{}{}", internal_service_host, INTERNAL_PRIVY_USER_INFO_PATH);

	let client = get_client();
	let response = client.get(&url).query(&req).send().await?;

	if !response.status().is_success() {
		return Err(anyhow::anyhow!("Failed to get privy user info: status={}, body={}", response.status(), response.text().await.unwrap_or_default()));
	}

	let result: InternalServiceResponse<ResGetPrivyUserInfo> = response.json().await?;
	result.into_result()
}

/// 获取Google签名URL
pub async fn get_google_image_sign(req: ReqGetGoogleImageSign) -> Result<ResGetGoogleImageSign> {
	let internal_service_host = &common::common_env::get_common_env().internal_service_host;
	let url = format!("{}{}", internal_service_host, INTERNAL_GOOGLE_IMAGE_SIGN_PATH);

	let client = get_client();
	let response = client.get(&url).query(&req).send().await?;

	if !response.status().is_success() {
		return Err(anyhow::anyhow!("Failed to get google image sign: status={}, body={}", response.status(), response.text().await.unwrap_or_default()));
	}

	let result: InternalServiceResponse<ResGetGoogleImageSign> = response.json().await?;
	result.into_result()
}
