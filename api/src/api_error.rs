/// API 错误码定义
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiErrorCode {
	/// 成功
	Success = 0,
	/// 认证失败
	AuthFailed = 2001,
	/// Privy 没有设置地址
	PrivyNoAddress = 2002,
	/// 参数错误
	InvalidParameter = 2003,
	/// 用户不存在
	UserNotFound = 2004,
	/// Event 不存在或已关闭
	EventNotFoundOrClosed = 2005,
	/// Market 不存在或已关闭
	MarketNotFoundOrClosed = 2006,
	/// Token ID 不存在
	TokenIdNotFound = 2007,
	/// 自定义错误
	CustomerError = 2997,
	/// 内部错误
	InternalError = 2998,
	/// 未知错误
	UnknownError = 2999,
}

/// 内部错误类型 - 用于表示服务器内部错误
#[derive(Debug)]
pub struct InternalError;

impl axum::response::IntoResponse for InternalError {
	fn into_response(self) -> axum::response::Response {
		let body = crate::api_types::ApiResponse::<()>::error(ApiErrorCode::InternalError);
		(axum::http::StatusCode::OK, axum::Json(body)).into_response()
	}
}

// 数据库错误/内部错误等返回内部错误码而不是http 500错误

impl ApiErrorCode {
	/// 获取错误消息
	pub fn message(&self) -> &'static str {
		match self {
			ApiErrorCode::Success => "success",
			ApiErrorCode::AuthFailed => "Authentication failed",
			ApiErrorCode::PrivyNoAddress => "Privy address not set",
			ApiErrorCode::InvalidParameter => "Invalid parameter",
			ApiErrorCode::UserNotFound => "User not found",
			ApiErrorCode::EventNotFoundOrClosed => "Event not found or closed",
			ApiErrorCode::MarketNotFoundOrClosed => "Market not found or closed",
			ApiErrorCode::TokenIdNotFound => "Token ID not found",
			ApiErrorCode::CustomerError => "Customer error",
			ApiErrorCode::InternalError => "Internal error",
			ApiErrorCode::UnknownError => "Unknown error",
		}
	}

	/// 转换为 i32
	pub fn as_i32(&self) -> i32 {
		*self as i32
	}
}
