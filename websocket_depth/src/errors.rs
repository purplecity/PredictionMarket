use thiserror::Error;

/// 订阅相关错误
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum SubscriptionError {
	#[error("Max subscriptions exceeded")]
	MaxSubscriptionsExceeded,

	#[error("Connection not found")]
	ConnectionNotFound,

	#[error("Subscription not found")]
	SubscriptionNotFound,

	#[error("Depth not available")]
	DepthNotAvailable,
}
