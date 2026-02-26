use {crate::cache, common::event_types::ApiMQEventCreate, futures_util::TryStreamExt, std::collections::HashMap, tracing::info};

/// 从数据库加载所有市场信息到内存缓存
pub async fn load_events() -> anyhow::Result<()> {
	let read_pool = crate::db::get_db_read_pool()?;

	info!("Loading events from database using stream...");

	let mut count = 0;
	// 使用流式查询，逐个处理市场记录
	let mut stream = sqlx::query_as::<_, common::model::Events>("SELECT * FROM events").fetch(&read_pool);

	// 逐个处理每条记录
	while let Some(event) = stream.try_next().await? {
		let api_event = convert_event_to_api_mq_event_create(event);
		cache::insert_event(api_event).await;
		count += 1;
	}

	info!("Successfully loaded {} events into cache", count);
	Ok(())
}

/// 将数据库 Events 模型转换为 ApiMQEventCreate
fn convert_event_to_api_mq_event_create(event: common::model::Events) -> ApiMQEventCreate {
	// 将 markets 从 Json<HashMap<String, EventMarket>> 转换为 HashMap<String, ApiMQEventMarket>
	let mut api_markets = HashMap::new();
	for (market_id, market) in event.markets.0.iter() {
		// 构建 outcome_info: token_id -> outcome_name 的映射
		let mut outcome_info = HashMap::new();
		for (idx, token_id) in market.token_ids.iter().enumerate() {
			if let Some(outcome_name) = market.outcomes.get(idx) {
				outcome_info.insert(token_id.clone(), outcome_name.clone());
			}
		}

		let api_market = common::event_types::ApiMQEventMarket {
			parent_collection_id: market.parent_collection_id.clone(),
			market_id: market.id,
			condition_id: market.condition_id.clone(),
			market_identifier: market.market_identifier.clone(),
			question: market.question.clone(),
			slug: market.slug.clone(),
			title: market.title.clone(),
			image: market.image.clone(),
			outcome_info,
			outcome_names: market.outcomes.clone(),
			outcome_token_ids: market.token_ids.clone(),
		};
		api_markets.insert(market_id.clone(), api_market);
	}

	ApiMQEventCreate {
		event_id: event.id,
		event_identifier: event.event_identifier,
		slug: event.slug,
		title: event.title,
		description: event.description,
		image: event.image,
		end_date: event.end_date,
		topic: event.topic,
		markets: api_markets,
		created_at: event.created_at,
	}
}
