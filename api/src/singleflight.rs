use {
	crate::api_types::{DepthResponse, EventDetailResponse, EventsResponse},
	anyhow::Error,
	async_singleflight::Group,
	tokio::sync::OnceCell,
};

//Group中Hashmap中的key-value都是send 外面加了Mutex
//T: Send，Mutex<T> 就是 Send + Sync！
//所以Group是send+sync的

/// Topics 查询的 singleflight group
/// 返回类型: Vec<String>
pub static TOPICS_GROUP: OnceCell<Group<String, Vec<String>, Error>> = OnceCell::const_new();

/// 获取 topics group 的便捷函数
/// 拿到的是相同的引用 Group:Send+Sync
pub async fn get_topics_group() -> &'static Group<String, Vec<String>, Error> {
	TOPICS_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Events 查询的 singleflight group
/// Key 根据查询参数生成
pub static EVENTS_GROUP: OnceCell<Group<String, EventsResponse, Error>> = OnceCell::const_new();

pub async fn get_events_group() -> &'static Group<String, EventsResponse, Error> {
	EVENTS_GROUP.get_or_init(|| async { Group::new() }).await
}

/// EventDetail 查询的 singleflight group
/// Key: event_id
pub static EVENT_DETAIL_GROUP: OnceCell<Group<String, EventDetailResponse, Error>> = OnceCell::const_new();

pub async fn get_event_detail_group() -> &'static Group<String, EventDetailResponse, Error> {
	EVENT_DETAIL_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Depth 查询的 singleflight group
/// Key: event_id|market_id
pub static DEPTH_GROUP: OnceCell<Group<String, DepthResponse, Error>> = OnceCell::const_new();

pub async fn get_depth_group() -> &'static Group<String, DepthResponse, Error> {
	DEPTH_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Market 验证的 singleflight group
/// Key: event_id|market_id
/// 用于下单时避免重复查询缓存和数据库
pub static MARKET_VALIDATION_GROUP: OnceCell<Group<String, common::event_types::ApiMQEventMarket, crate::cache::CacheError>> = OnceCell::const_new();

pub async fn get_market_validation_group() -> &'static Group<String, common::event_types::ApiMQEventMarket, crate::cache::CacheError> {
	MARKET_VALIDATION_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Streamer Viewer Count 查询的 singleflight group
/// Key: streamer_id
pub static STREAMER_VIEWER_COUNT_GROUP: OnceCell<Group<String, i32, Error>> = OnceCell::const_new();

pub async fn get_streamer_viewer_count_group() -> &'static Group<String, i32, Error> {
	STREAMER_VIEWER_COUNT_GROUP.get_or_init(|| async { Group::new() }).await
}

/// 积分排行榜前200名 singleflight group
/// Key: "points:{season_id}" (按赛季区分)
pub static LEADERBOARD_POINTS_GROUP: OnceCell<Group<String, Vec<crate::api_types::LeaderboardPointsItem>, Error>> = OnceCell::const_new();

pub async fn get_leaderboard_points_group() -> &'static Group<String, Vec<crate::api_types::LeaderboardPointsItem>, Error> {
	LEADERBOARD_POINTS_GROUP.get_or_init(|| async { Group::new() }).await
}

/// 交易量排行榜前200名 singleflight group
/// Key: "volume:{season_id}" (按赛季区分)
pub static LEADERBOARD_VOLUME_GROUP: OnceCell<Group<String, Vec<crate::api_types::LeaderboardVolumeItem>, Error>> = OnceCell::const_new();

pub async fn get_leaderboard_volume_group() -> &'static Group<String, Vec<crate::api_types::LeaderboardVolumeItem>, Error> {
	LEADERBOARD_VOLUME_GROUP.get_or_init(|| async { Group::new() }).await
}

/// 当前赛季信息 singleflight group
/// Key: "current" (固定key)
pub static CURRENT_SEASON_GROUP: OnceCell<Group<String, Option<crate::api_types::CurrentSeasonResponse>, Error>> = OnceCell::const_new();

pub async fn get_current_season_group() -> &'static Group<String, Option<crate::api_types::CurrentSeasonResponse>, Error> {
	CURRENT_SEASON_GROUP.get_or_init(|| async { Group::new() }).await
}

/// 所有赛季列表 singleflight group
/// Key: "list" (固定key)
pub static SEASONS_LIST_GROUP: OnceCell<Group<String, Vec<crate::api_types::SeasonItem>, Error>> = OnceCell::const_new();

pub async fn get_seasons_list_group() -> &'static Group<String, Vec<crate::api_types::SeasonItem>, Error> {
	SEASONS_LIST_GROUP.get_or_init(|| async { Group::new() }).await
}

/// 当前赛季ID singleflight group
/// Key: "current_id" (固定key)
pub static CURRENT_SEASON_ID_GROUP: OnceCell<Group<String, Option<i32>, Error>> = OnceCell::const_new();

pub async fn get_current_season_id_group() -> &'static Group<String, Option<i32>, Error> {
	CURRENT_SEASON_ID_GROUP.get_or_init(|| async { Group::new() }).await
}

/// 赛季是否存在 singleflight group
/// Key: season_id
pub static SEASON_EXISTS_GROUP: OnceCell<Group<String, bool, Error>> = OnceCell::const_new();

pub async fn get_season_exists_group() -> &'static Group<String, bool, Error> {
	SEASON_EXISTS_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Lives 查询的 singleflight group
/// Key: 根据查询参数生成
pub static LIVES_GROUP: OnceCell<Group<String, crate::api_types::LivesResponse, Error>> = OnceCell::const_new();

pub async fn get_lives_group() -> &'static Group<String, crate::api_types::LivesResponse, Error> {
	LIVES_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Event Streamers 查询的 singleflight group
/// Key: event_id
pub static EVENT_STREAMERS_GROUP: OnceCell<Group<String, crate::api_types::EventStreamersResponse, Error>> = OnceCell::const_new();

pub async fn get_event_streamers_group() -> &'static Group<String, crate::api_types::EventStreamersResponse, Error> {
	EVENT_STREAMERS_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Streamer Detail 查询的 singleflight group
/// Key: streamer_uid
pub static STREAMER_DETAIL_GROUP: OnceCell<Group<String, crate::api_types::StreamerDetailResponse, Error>> = OnceCell::const_new();

pub async fn get_streamer_detail_group() -> &'static Group<String, crate::api_types::StreamerDetailResponse, Error> {
	STREAMER_DETAIL_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Balance 查询的 singleflight group
/// Key: user_id
pub static BALANCE_GROUP: OnceCell<Group<String, rust_decimal::Decimal, Error>> = OnceCell::const_new();

pub async fn get_balance_group() -> &'static Group<String, rust_decimal::Decimal, Error> {
	BALANCE_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Portfolio Value 查询的 singleflight group
/// Key: user_id
pub static PORTFOLIO_VALUE_GROUP: OnceCell<Group<String, rust_decimal::Decimal, Error>> = OnceCell::const_new();

pub async fn get_portfolio_value_group() -> &'static Group<String, rust_decimal::Decimal, Error> {
	PORTFOLIO_VALUE_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Traded Volume 查询的 singleflight group
/// Key: user_id
pub static TRADED_VOLUME_GROUP: OnceCell<Group<String, rust_decimal::Decimal, Error>> = OnceCell::const_new();

pub async fn get_traded_volume_group() -> &'static Group<String, rust_decimal::Decimal, Error> {
	TRADED_VOLUME_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Positions 查询的 singleflight group
/// Key: user_id:event_id:market_id:page (event_id/market_id 可为空)
pub static POSITIONS_GROUP: OnceCell<Group<String, crate::api_types::PositionsResponse, Error>> = OnceCell::const_new();

pub async fn get_positions_group() -> &'static Group<String, crate::api_types::PositionsResponse, Error> {
	POSITIONS_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Closed Positions 查询的 singleflight group
/// Key: user_id:event_id:market_id:page
pub static CLOSED_POSITIONS_GROUP: OnceCell<Group<String, crate::api_types::ClosedPositionsResponse, Error>> = OnceCell::const_new();

pub async fn get_closed_positions_group() -> &'static Group<String, crate::api_types::ClosedPositionsResponse, Error> {
	CLOSED_POSITIONS_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Activity 查询的 singleflight group
/// Key: user_id:event_id:market_id:page:page_size
pub static ACTIVITY_GROUP: OnceCell<Group<String, crate::api_types::ActivityResponse, Error>> = OnceCell::const_new();

pub async fn get_activity_group() -> &'static Group<String, crate::api_types::ActivityResponse, Error> {
	ACTIVITY_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Open Orders 查询的 singleflight group
/// Key: user_id:event_id:market_id:page:page_size
pub static OPEN_ORDERS_GROUP: OnceCell<Group<String, crate::api_types::OpenOrdersResponse, Error>> = OnceCell::const_new();

pub async fn get_open_orders_group() -> &'static Group<String, crate::api_types::OpenOrdersResponse, Error> {
	OPEN_ORDERS_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Points 查询的 singleflight group
/// Key: user_id
pub static POINTS_GROUP: OnceCell<Group<String, crate::api_types::PointsResponse, Error>> = OnceCell::const_new();

pub async fn get_points_group() -> &'static Group<String, crate::api_types::PointsResponse, Error> {
	POINTS_GROUP.get_or_init(|| async { Group::new() }).await
}

/// Simple Info 查询的 singleflight group
/// Key: user_id
pub static SIMPLE_INFO_GROUP: OnceCell<Group<String, crate::api_types::SimpleInfoResponse, Error>> = OnceCell::const_new();

pub async fn get_simple_info_group() -> &'static Group<String, crate::api_types::SimpleInfoResponse, Error> {
	SIMPLE_INFO_GROUP.get_or_init(|| async { Group::new() }).await
}

/// User Points Volumes 查询的 singleflight group
/// Key: user_id:season_id
pub static USER_POINTS_VOLUMES_GROUP: OnceCell<Group<String, crate::api_types::UserPointsVolumesResponse, Error>> = OnceCell::const_new();

pub async fn get_user_points_volumes_group() -> &'static Group<String, crate::api_types::UserPointsVolumesResponse, Error> {
	USER_POINTS_VOLUMES_GROUP.get_or_init(|| async { Group::new() }).await
}
