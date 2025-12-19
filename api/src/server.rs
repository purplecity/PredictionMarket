use {
	crate::{
		handlers::{
			handle_cancel_all_orders, handle_cancel_order, handle_event_detail, handle_events, handle_image_sign, handle_place_order, handle_topics, handle_user_data, handle_user_image,
			handle_user_profile,
		},
		user_handlers::{
			handle_activity, handle_closed_positions, handle_depth, handle_event_balance, handle_open_orders, handle_order_history, handle_portfolio_value, handle_positions, handle_traded_volume,
		},
	},
	axum::{
		Router,
		extract::{ConnectInfo, Request, State},
		http::{HeaderName, StatusCode, header::AUTHORIZATION},
		middleware::{self, Next},
		response::Response,
		routing::{get, post},
	},
	common::rate_limit::{RateLimiter, is_allowed, rate_limit_gc},
	std::{net::SocketAddr, sync::Arc, time::Duration},
	tower_http::{
		compression::CompressionLayer,
		cors::{Any, CorsLayer},
		request_id::{MakeRequestUuid, PropagateRequestIdLayer, SetRequestIdLayer},
	},
	uuid::Uuid,
};

#[derive(Clone)]
pub struct AppState {
	rate_limiter: Arc<RateLimiter>,
}

impl Default for AppState {
	fn default() -> Self {
		Self { rate_limiter: Arc::new(RateLimiter::default()) }
	}
}

#[derive(Clone)]
pub struct ClientInfo {
	pub request_id: String,
	pub ip: String,
	pub privy_id: Option<String>,
}

//中间件
async fn extract_and_check(State(state): State<AppState>, connect_info: ConnectInfo<SocketAddr>, mut request: Request, next: Next) -> Result<Response, StatusCode> {
	// 从请求头中获取真实IP，优先级：X-Forwarded-For > X-Real-IP > ConnectInfo
	let ip = request
		.headers()
		.get("x-real-ip")
		.or_else(|| request.headers().get("X-Real-IP"))
		.and_then(|header| header.to_str().ok())
		.and_then(|value| value.split(',').next()) // 取第一个IP
		.map(|s| s.trim().to_string())
		.or_else(|| request.headers().get("x-forwarded-for").or_else(|| request.headers().get("X-Forwarded-For")).and_then(|header| header.to_str().ok()).map(|s| s.trim().to_string()))
		.unwrap_or_else(|| connect_info.ip().to_string());

	let uri_path = request.uri().path().to_string(); //子路由没有api前缀

	// exract
	let common_env = common::common_env::get_common_env();
	let pem_key = common_env.privy_pem_key.clone();
	let app_id = common_env.privy_app_id.clone();
	let privy_id = request.headers().get(AUTHORIZATION).and_then(|header| header.to_str().ok()).and_then(|token| common::privy_jwt::check_privy_jwt(token, &pem_key, &app_id).ok());

	let x_request_id = HeaderName::from_static("x-request-id");
	let request_id = request.headers().get(x_request_id).and_then(|header| header.to_str().ok()).map(|header| header.to_string()).unwrap_or_else(|| Uuid::new_v4().to_string());
	request.extensions_mut().insert(ClientInfo { request_id, ip: ip.clone(), privy_id: privy_id.clone() });

	// check
	let rate_limiter_clone = state.rate_limiter.clone();
	if !is_allowed(rate_limiter_clone.clone(), ip, uri_path.clone()) {
		return Err(StatusCode::TOO_MANY_REQUESTS);
	}

	if let Some(privy_id) = privy_id
		&& !is_allowed(rate_limiter_clone, privy_id.clone(), uri_path)
	{
		return Err(StatusCode::TOO_MANY_REQUESTS);
	}
	Ok(next.run(request).await)
}

pub fn init_app_state() -> anyhow::Result<AppState> {
	let rate_limiter = RateLimiter::new_with_multi_pattern_with_same_rule(&RATE_LIMIT_PATTERNS.iter().map(|s| s.to_string()).collect::<Vec<String>>(), 5, 1)?;
	let state = AppState { rate_limiter: Arc::new(rate_limiter) };
	let rate_limiter_clone = state.rate_limiter.clone();
	tokio::spawn(async move { rate_limit_gc(rate_limiter_clone).await });
	Ok(state)
}

const RATE_LIMIT_PATTERNS: &[&str] = &[
	"/hi",
	"/user_image",
	"/user_profile",
	"/place_order",
	"/cancel_order",
	"/cancel_all_orders",
	"/user_data",
	"/image_sign",
	"/topics",
	"/events",
	"/event_detail",
	"/portfolio_value",
	"/traded_volume",
	"/positions",
	"/closed_positions",
	"/activity",
	"/open_orders",
	"/order_history",
	"/depth",
	"/event_balance",
];

pub fn app() -> anyhow::Result<Router> {
	let state = init_app_state()?;
	let x_request_id = HeaderName::from_static("x-request-id");
	let sub_router = Router::new()
		.route("/hi", get(handle_hi))
		.route("/user_image", post(handle_user_image))
		.route("/user_profile", post(handle_user_profile))
		.route("/place_order", post(handle_place_order))
		.route("/cancel_order", post(handle_cancel_order))
		.route("/cancel_all_orders", post(handle_cancel_all_orders))
		.route("/user_data", get(handle_user_data))
		.route("/image_sign", get(handle_image_sign))
		.route("/topics", get(handle_topics))
		.route("/events", get(handle_events))
		.route("/event_detail", get(handle_event_detail))
		.route("/portfolio_value", get(handle_portfolio_value))
		.route("/traded_volume", get(handle_traded_volume))
		.route("/positions", get(handle_positions))
		.route("/closed_positions", get(handle_closed_positions))
		.route("/activity", get(handle_activity))
		.route("/open_orders", get(handle_open_orders))
		.route("/order_history", get(handle_order_history))
		.route("/depth", get(handle_depth))
		.route("/event_balance", get(handle_event_balance))
		.layer(PropagateRequestIdLayer::new(x_request_id.clone())) //将请求id从请求头中传递到响应头中
		.layer(CompressionLayer::new())
		.layer(middleware::from_fn_with_state(state.clone(), extract_and_check))
		//.layer(TraceLayer::new_for_http())
		.layer(SetRequestIdLayer::new(x_request_id, MakeRequestUuid)) //生成请求id 并放到请求头中
		.layer(CorsLayer::new().allow_methods(Any).allow_origin(Any).allow_credentials(false).allow_headers(Any).expose_headers(Any).max_age(Duration::from_secs(60) * 10))
		.with_state(state);

	Ok(Router::new().nest("/api", sub_router)) //最重要的是执行中间件之前提前404 把这行去掉换成sub_router然后访问根目录会发现打印了404日志 显然这种就算404也执行了中间件
}

async fn handle_hi() -> &'static str {
	"You will succeed."
}
