use {common::common_env::get_common_env, serde::Deserialize, std::time::Duration};

#[derive(Deserialize, Debug)]
struct OnlineResponse {
	code: i32,
	data: i32,
	msg: String,
}

/// 获取直播间在线人数
///
/// # 参数
/// * `room_id` - 直播间 ID（room_id）
///
/// # 返回
/// 返回在线人数，如果出错则返回错误信息
pub async fn get_online_member_num(room_id: &str) -> Result<i32, String> {
	// 从环境变量获取 Chat Service 基础 URL
	let chat_service_base_url = &get_common_env().chat_service_base_url;

	// 构建请求 URL，使用 room_id 作为查询参数
	let request_url = format!("{}/api/v1/chat/internal/online?room_id={}", chat_service_base_url, room_id);

	// 创建 HTTP 客户端
	let client = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().map_err(|e| format!("创建 HTTP 客户端失败: {}", e))?;

	// 发送 GET 请求
	let response = client.get(&request_url).send().await.map_err(|e| format!("发送请求失败: {}", e))?;

	// 检查响应状态
	if !response.status().is_success() {
		return Err(format!("HTTP 请求失败，状态码: {}", response.status()));
	}

	// 解析响应
	let response_body: OnlineResponse = response.json().await.map_err(|e| format!("解析响应失败: {}", e))?;

	// 检查错误码
	if response_body.code != 0 {
		return Err(format!("API 返回错误: code={}, msg={}", response_body.code, response_body.msg));
	}

	// 返回在线人数
	Ok(response_body.data)
}
