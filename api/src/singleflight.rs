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
