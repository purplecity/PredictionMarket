use {
	crate::{
		api_error::InternalError,
		api_types::{
			ActivityRequest, ActivityResponse, ApiResponse, ClosedPositionsRequest, ClosedPositionsResponse, DepthBook, DepthRequest, DepthResponse, EventBalanceRequest, EventBalanceResponse,
			OpenOrdersRequest, OpenOrdersResponse, OrderHistoryRequest, OrderHistoryResponse, PortfolioValueRequest, PortfolioValueResponse, PositionRequest, PositionsResponse,
			SingleActivityResponse, SingleClosedPositionResponse, SingleOpenOrderResponse, SingleOrderHistoryResponse, SinglePositionResponse, TradedVolumeRequest, TradedVolumeResponse,
		},
		cache, db,
		handlers::get_user_id_from_auth,
		server::ClientInfo,
		singleflight::{get_activity_group, get_closed_positions_group, get_depth_group, get_open_orders_group, get_portfolio_value_group, get_positions_group, get_traded_volume_group},
	},
	axum::{
		extract::{Extension, Query},
		response::Json,
	},
	common::{
		consts::USDC_TOKEN_ID,
		depth_types::CacheMarketPriceInfo,
		engine_types::{OrderStatus, OrderType},
		key::{DEPTH_CACHE_KEY, PRICE_CACHE_KEY, market_field},
		model::{AssetHistoryType, Events, OperationHistory, Orders, Positions},
		redis_pool,
		websocket_types::WebSocketDepth,
	},
	redis::AsyncCommands,
	rust_decimal::Decimal,
	std::{collections::HashMap, str::FromStr},
	tracing,
};

const PAGE_SIZE: i16 = 1000;

/// 计算价格
/// 如果 |best_bid - best_ask| < 0.1，使用 latest_trade_price
/// 否则使用 (best_bid + best_ask) / 2
fn calculate_chance(best_bid: &str, best_ask: &str, latest_trade_price: &str) -> String {
	let bid = Decimal::from_str(best_bid).unwrap_or(Decimal::ZERO);
	let ask = Decimal::from_str(best_ask).unwrap_or(Decimal::ZERO);

	// 如果 best_bid 或 best_ask 有一个是 0，直接使用最新成交价
	if bid.is_zero() || ask.is_zero() {
		if latest_trade_price.is_empty() {
			return "0".to_string();
		}
		return latest_trade_price.to_string();
	}

	let diff = (bid - ask).abs();
	let threshold = Decimal::new(1, 1); // 0.1

	if diff < threshold {
		// 使用 latest_trade_price
		if latest_trade_price.is_empty() {
			// 如果没有成交价，回退到中间价
			((bid + ask) / Decimal::TWO).normalize().to_string()
		} else {
			latest_trade_price.to_string()
		}
	} else {
		// 使用中间价
		((bid + ask) / Decimal::TWO).normalize().to_string()
	}
}

/// 获取用户投资组合价值（公开接口）
/// GET /portfolio_value?uid=xxx
pub async fn handle_portfolio_value(Query(params): Query<PortfolioValueRequest>) -> Result<Json<ApiResponse<PortfolioValueResponse>>, InternalError> {
	let user_id = params.uid;
	let group = get_portfolio_value_group().await;
	let key = user_id.to_string();

	let result = group
		.work(&key, async move {
			let read_pool = db::get_db_read_pool()?;

			// 查询未 redeemed 的持仓
			let positions: Vec<Positions> =
				sqlx::query_as("SELECT * FROM positions WHERE user_id = $1 AND (redeemed IS NULL OR redeemed = false)").bind(user_id).fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?;

			let mut cash = Decimal::ZERO;
			let mut total_value = Decimal::ZERO;

			// 分离 USDC 和其他 token
			let mut non_usdc_positions: Vec<&Positions> = Vec::new();
			for pos in &positions {
				if pos.token_id == USDC_TOKEN_ID {
					cash = pos.balance.checked_add(pos.frozen_balance).unwrap_or(pos.balance);
				} else {
					// 过滤掉 balance + frozen_balance <= 0 的持仓
					if let Some(sum) = pos.balance.checked_add(pos.frozen_balance)
						&& sum.le(&Decimal::ZERO)
					{
						continue;
					}
					non_usdc_positions.push(pos);
				}
			}

			if !non_usdc_positions.is_empty() {
				// 收集需要查询的 event_ids
				let event_ids: Vec<i64> = non_usdc_positions.iter().filter_map(|p| p.event_id).collect::<std::collections::HashSet<_>>().into_iter().collect();

				// 批量查询 events
				let events: Vec<Events> = if !event_ids.is_empty() {
					sqlx::query_as("SELECT * FROM events WHERE id = ANY($1)").bind(&event_ids).fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?
				} else {
					Vec::new()
				};

				// 构建 event_id -> Event 映射
				let event_map: std::collections::HashMap<i64, &Events> = events.iter().map(|e| (e.id, e)).collect();

				// 分离 closed 和未 closed 的 positions
				let mut closed_positions: Vec<&Positions> = Vec::new();
				let mut open_positions: Vec<&Positions> = Vec::new();

				for pos in &non_usdc_positions {
					if let Some(event_id) = pos.event_id
						&& let Some(event) = event_map.get(&event_id)
						&& let Some(market_id) = pos.market_id
						&& let Some(market) = event.markets.get(&market_id.to_string())
					{
						if market.closed {
							closed_positions.push(pos);
						} else {
							open_positions.push(pos);
						}
					}
				}

				// 处理 closed positions
				for pos in closed_positions {
					if let (Some(event_id), Some(market_id)) = (pos.event_id, pos.market_id)
						&& let Some(event) = event_map.get(&event_id)
						&& let Some(market) = event.markets.get(&market_id.to_string())
					{
						// 如果 token_id 是赢家，价值 = balance + frozen_balance
						if pos.token_id == market.win_outcome_token_id
							&& let Some(sum) = pos.balance.checked_add(pos.frozen_balance)
						{
							total_value = total_value.checked_add(sum).unwrap_or(total_value);
						}
						// 否则价值为 0
					}
				}

				// 处理 open positions - 需要查询价格缓存
				if !open_positions.is_empty() {
					let mut conn = redis_pool::get_cache_redis_connection().await.map_err(|e| anyhow::anyhow!(e))?;

					// 收集需要查询的 price fields
					let price_fields: Vec<String> = open_positions
						.iter()
						.filter_map(|p| if let (Some(event_id), Some(market_id)) = (p.event_id, p.market_id) { Some(market_field(event_id, market_id)) } else { None })
						.collect::<std::collections::HashSet<_>>()
						.into_iter()
						.collect();

					// 批量获取价格信息
					let price_values: Vec<Option<String>> =
						if !price_fields.is_empty() { conn.hget(PRICE_CACHE_KEY, &price_fields).await.unwrap_or_else(|_| vec![None; price_fields.len()]) } else { Vec::new() };

					// 构建 field -> price_info 映射
					let mut price_map: HashMap<String, CacheMarketPriceInfo> = HashMap::new();
					for (i, field) in price_fields.iter().enumerate() {
						if let Some(Some(json_str)) = price_values.get(i)
							&& let Ok(info) = serde_json::from_str::<CacheMarketPriceInfo>(json_str)
						{
							price_map.insert(field.clone(), info);
						}
					}

					// 计算 open positions 价值
					for pos in open_positions {
						if let (Some(event_id), Some(market_id)) = (pos.event_id, pos.market_id) {
							let field = market_field(event_id, market_id);
							if let Some(price_info) = price_map.get(&field)
								&& let Some(token_price_info) = price_info.prices.get(&pos.token_id)
							{
								// 使用 calculate_chance 计算价格
								let price_str = calculate_chance(&token_price_info.best_bid, &token_price_info.best_ask, &token_price_info.latest_trade_price);
								if let Ok(price) = Decimal::from_str(&price_str)
									&& let Some(position_value) = pos.balance.checked_add(pos.frozen_balance).and_then(|sum| price.checked_mul(sum))
								{
									total_value = total_value.checked_add(position_value).unwrap_or(total_value);
								}
							}
							// 如果没有价格缓存，价值为 0
						}
					}
				}
			}

			// 总价值 = cash + token 价值
			total_value = total_value.checked_add(cash).unwrap_or(total_value);

			// normalize before returning（如果小于等于 0，则设为 0）
			let value_normalized = if total_value.le(&Decimal::ZERO) { Decimal::ZERO } else { total_value.normalize() };

			Ok(value_normalized)
		})
		.await;

	match result {
		Ok(value) => Ok(Json(ApiResponse::success(PortfolioValueResponse { value }))),
		Err(Some(e)) => {
			tracing::error!("Failed to get portfolio value for user {}: {}", user_id, e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight portfolio_value leader dropped for user {}", user_id);
			Err(InternalError)
		}
	}
}

/// 获取用户总交易量（公开接口）
/// GET /traded_volume?uid=xxx
pub async fn handle_traded_volume(Query(params): Query<TradedVolumeRequest>) -> Result<Json<ApiResponse<TradedVolumeResponse>>, InternalError> {
	let user_id = params.uid;
	let group = get_traded_volume_group().await;
	let key = user_id.to_string();

	let result = group
		.work(&key, async move {
			let read_pool = db::get_db_read_pool()?;

			// 查询用户作为 taker 的总交易量 (trade_volume)
			let taker_volume: Option<Decimal> =
				sqlx::query_scalar("SELECT COALESCE(SUM(usdc_amount), 0) FROM trades WHERE user_id = $1 AND taker = true").bind(user_id).fetch_one(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?;

			// 查询用户作为 maker 的总交易量 (usdc_amount)
			let maker_volume: Option<Decimal> = sqlx::query_scalar("SELECT COALESCE(SUM(usdc_amount), 0) FROM trades WHERE user_id = $1 AND taker = false")
				.bind(user_id)
				.fetch_one(&read_pool)
				.await
				.map_err(|e| anyhow::anyhow!(e))?;

			// 总交易量 = taker_volume + maker_volume (自己不能和自己成交，所以直接相加)
			let total_volume = taker_volume.unwrap_or(Decimal::ZERO).checked_add(maker_volume.unwrap_or(Decimal::ZERO)).unwrap_or(Decimal::ZERO);
			Ok(total_volume.normalize())
		})
		.await;

	match result {
		Ok(volume) => Ok(Json(ApiResponse::success(TradedVolumeResponse { volume }))),
		Err(Some(e)) => {
			tracing::error!("Failed to get traded volume for user {}: {}", user_id, e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight traded_volume leader dropped for user {}", user_id);
			Err(InternalError)
		}
	}
}

/// 获取用户持仓列表（公开接口）
/// GET /positions?uid=xxx
pub async fn handle_positions(Query(params): Query<PositionRequest>) -> Result<Json<ApiResponse<PositionsResponse>>, InternalError> {
	let user_id = params.uid;
	// 构建 singleflight key: uid:event_id:market_id:question:page
	let question_key = params.question.as_deref().unwrap_or("");
	let key = format!("{}:{}:{}:{}:{}", user_id, params.event_id.unwrap_or(0), params.market_id.unwrap_or(0), question_key, params.page);

	let group = get_positions_group().await;
	let params_clone = params.clone();

	let result = group
		.work(&key, async move {
			let read_pool = db::get_db_read_pool()?;
			let (count_sql, query_sql, has_event_filter, has_market_filter, has_question_filter) = build_position_query(&params_clone);

			// 查询总数 - 使用动态绑定
			let total: i64 =
				bind_position_count_query(&count_sql, user_id, &params_clone, has_event_filter, has_market_filter, has_question_filter).fetch_one(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?;

			let total = total as i16;

			// 如果没有数据，直接返回空
			if total == 0 {
				return Ok(PositionsResponse { positions: vec![], total: 0, has_more: false });
			}

			// 超过 1000 条：只支持 avg_price 排序，不计算动态值
			if total > PAGE_SIZE {
				return handle_large_positions_internal(&read_pool, user_id, &params_clone, &query_sql, has_event_filter, has_market_filter, has_question_filter, total).await;
			}

			// 小于等于 1000 条：获取所有记录，动态计算后排序
			handle_small_positions_internal(&read_pool, user_id, &params_clone, &query_sql, has_event_filter, has_market_filter, has_question_filter, total).await
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Failed to get positions for user {}: {}", user_id, e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight positions leader dropped for user {}", user_id);
			Err(InternalError)
		}
	}
}

/// 动态绑定持仓查询参数 (用于 count 查询)
fn bind_position_count_query<'a>(
	sql: &'a str,
	user_id: i64,
	params: &'a PositionRequest,
	has_event: bool,
	has_market: bool,
	has_question: bool,
) -> sqlx::query::QueryScalar<'a, sqlx::Postgres, i64, sqlx::postgres::PgArguments> {
	let mut query = sqlx::query_scalar(sql).bind(user_id);
	if has_event {
		query = query.bind(params.event_id.expect("event_id checked"));
	}
	if has_market {
		query = query.bind(params.market_id.expect("market_id checked"));
	}
	if has_question {
		query = query.bind(params.question.as_ref().expect("question checked"));
	}
	query
}

/// 动态绑定持仓查询参数 (用于 data 查询)
fn bind_position_query<'a>(
	sql: &'a str,
	user_id: i64,
	params: &'a PositionRequest,
	has_event: bool,
	has_market: bool,
	has_question: bool,
) -> sqlx::query::QueryAs<'a, sqlx::Postgres, Positions, sqlx::postgres::PgArguments> {
	let mut query = sqlx::query_as(sql).bind(user_id);
	if has_event {
		query = query.bind(params.event_id.expect("event_id checked"));
	}
	if has_market {
		query = query.bind(params.market_id.expect("market_id checked"));
	}
	if has_question {
		query = query.bind(params.question.as_ref().expect("question checked"));
	}
	query
}

/// 构建持仓查询 SQL
fn build_position_query(params: &PositionRequest) -> (String, String, bool, bool, bool) {
	let has_event_filter = params.event_id.is_some();
	let has_market_filter = params.market_id.is_some();
	let has_question_filter = params.question.as_ref().is_some_and(|q| !q.is_empty());

	// 使用 USDC_TOKEN_ID 常量构建 SQL
	let base_where = format!("user_id = $1 AND token_id != '{}' AND (redeemed IS NULL OR redeemed = false)", USDC_TOKEN_ID);

	// 动态构建 where 子句，参数按顺序: user_id ($1), event_id, market_id, question
	let mut next_param = 2;
	let mut where_clause = base_where;

	if has_event_filter {
		where_clause = format!("{} AND event_id = ${}", where_clause, next_param);
		next_param += 1;
	}
	if has_market_filter {
		where_clause = format!("{} AND market_id = ${}", where_clause, next_param);
		next_param += 1;
	}
	if has_question_filter {
		where_clause = format!("{} AND question LIKE '%' || ${} || '%'", where_clause, next_param);
	}

	let count_sql = format!("SELECT COUNT(*) FROM positions WHERE {}", where_clause);
	let query_sql = format!("SELECT * FROM positions WHERE {}", where_clause);

	(count_sql, query_sql, has_event_filter, has_market_filter, has_question_filter)
}

/// 处理大量数据（>1000）：只支持 avg_price 排序（内部函数，返回 Result）
async fn handle_large_positions_internal(
	read_pool: &sqlx::PgPool,
	user_id: i64,
	params: &PositionRequest,
	query_sql: &str,
	has_event_filter: bool,
	has_market_filter: bool,
	has_question_filter: bool,
	total: i16,
) -> Result<PositionsResponse, anyhow::Error> {
	// 只支持 avg_price 排序
	let order = if params.avg_price == Some(false) { "ASC" } else { "DESC" };
	let offset = (params.page.max(1) - 1) as i64 * PAGE_SIZE as i64;

	let full_sql = format!("{} ORDER BY avg_price {} NULLS LAST LIMIT {} OFFSET {}", query_sql, order, PAGE_SIZE, offset);

	let positions: Vec<Positions> = bind_position_query(&full_sql, user_id, params, has_event_filter, has_market_filter, has_question_filter).fetch_all(read_pool).await?;

	// 获取 event 信息（从缓存中批量获取）
	let event_ids: Vec<i64> = positions.iter().filter_map(|p| p.event_id).collect::<std::collections::HashSet<_>>().into_iter().collect();
	let events_map = if !event_ids.is_empty() { cache::get_events(&event_ids).await? } else { HashMap::new() };

	// 构建响应（不计算动态值）
	let result: Vec<SinglePositionResponse> = positions
		.iter()
		.filter_map(|pos| {
			let (event_id, market_id) = (pos.event_id?, pos.market_id?);
			let event = events_map.get(&event_id)?;
			let market = event.markets.get(&market_id.to_string())?;
			let quantity = pos.balance.checked_add(pos.frozen_balance).unwrap_or(pos.balance);

			// 过滤掉 quantity 为 0 的持仓
			if quantity.le(&Decimal::ZERO) {
				return None;
			}

			Some(SinglePositionResponse {
				event_id,
				market_id,
				event_title: event.title.clone(),
				market_title: market.title.clone(),
				market_question: market.question.clone(),
				event_image: event.image.clone(),
				market_image: market.image.clone(),
				outcome_name: pos.outcome_name.clone().unwrap_or_default(),
				condition_id: market.condition_id.clone(),
				token_id: pos.token_id.clone(),
				avg_price: pos.avg_price.unwrap_or(Decimal::ZERO).normalize(),
				quantity: quantity.normalize(),
				value: Decimal::ZERO,             // 大数据量不计算
				profit_value: Decimal::ZERO,      // 大数据量不计算
				profit_percentage: Decimal::ZERO, // 大数据量不计算
				current_price: Decimal::ZERO,     // 大数据量不计算
			})
		})
		.collect();

	let has_more = (params.page as i64 * PAGE_SIZE as i64) < total as i64;
	Ok(PositionsResponse { positions: result, total, has_more })
}

/// 处理小量数据（<=1000）：获取所有记录，动态计算后排序（内部函数）
async fn handle_small_positions_internal(
	read_pool: &sqlx::PgPool,
	user_id: i64,
	params: &PositionRequest,
	query_sql: &str,
	has_event_filter: bool,
	has_market_filter: bool,
	has_question_filter: bool,
	total: i16,
) -> Result<PositionsResponse, anyhow::Error> {
	// 获取所有记录
	let positions: Vec<Positions> = bind_position_query(query_sql, user_id, params, has_event_filter, has_market_filter, has_question_filter).fetch_all(read_pool).await?;

	if positions.is_empty() {
		return Ok(PositionsResponse { positions: vec![], total: 0, has_more: false });
	}

	// 收集 event_ids 和 market fields
	let event_ids: Vec<i64> = positions.iter().filter_map(|p| p.event_id).collect::<std::collections::HashSet<_>>().into_iter().collect();
	let price_fields: Vec<String> =
		positions.iter().filter_map(|p| if let (Some(event_id), Some(market_id)) = (p.event_id, p.market_id) { Some(market_field(event_id, market_id)) } else { None }).collect();

	// 并行获取 event 信息和价格缓存
	let read_pool_clone = read_pool.clone();
	let event_ids_clone = event_ids.clone();
	let price_fields_clone = price_fields.clone();

	let (events_result, prices_result) = tokio::join!(
		// Task 1: 获取 event 信息
		async move {
			if event_ids_clone.is_empty() {
				Ok::<Vec<Events>, sqlx::Error>(Vec::new())
			} else {
				sqlx::query_as::<_, Events>("SELECT * FROM events WHERE id = ANY($1)").bind(&event_ids_clone).fetch_all(&read_pool_clone).await
			}
		},
		// Task 2: 获取价格缓存
		async move {
			if price_fields_clone.is_empty() {
				return Ok::<HashMap<String, CacheMarketPriceInfo>, anyhow::Error>(HashMap::new());
			}
			let mut conn = redis_pool::get_cache_redis_connection().await?;
			let price_values: Vec<Option<String>> = conn.hget(PRICE_CACHE_KEY, &price_fields_clone).await.unwrap_or_else(|_| vec![None; price_fields_clone.len()]);

			let mut price_map: HashMap<String, CacheMarketPriceInfo> = HashMap::new();
			for (i, field) in price_fields_clone.iter().enumerate() {
				if let Some(Some(json_str)) = price_values.get(i)
					&& let Ok(info) = serde_json::from_str::<CacheMarketPriceInfo>(json_str)
				{
					price_map.insert(field.clone(), info);
				}
			}
			Ok(price_map)
		}
	);

	let events = events_result?;
	let price_map = prices_result?;
	let event_map: HashMap<i64, &Events> = events.iter().map(|e| (e.id, e)).collect();

	// 直接构造 SinglePositionResponse
	let mut results: Vec<SinglePositionResponse> = Vec::with_capacity(positions.len());

	for pos in positions {
		let (event_id, market_id) = match (pos.event_id, pos.market_id) {
			(Some(e), Some(m)) => (e, m),
			_ => continue,
		};

		let event = match event_map.get(&event_id) {
			Some(e) => *e,
			None => continue,
		};

		let market = match event.markets.get(&market_id.to_string()) {
			Some(m) => m,
			None => continue,
		};

		let quantity = pos.balance.checked_add(pos.frozen_balance).unwrap_or(pos.balance);

		// 过滤掉 quantity 为 0 的持仓
		if quantity.le(&Decimal::ZERO) {
			continue;
		}

		let avg_price = pos.avg_price.unwrap_or(Decimal::ZERO);

		// 计算当前价格 (Decimal)
		let current_price_decimal = if market.closed {
			// closed market: 赢家价格为 1，输家为 0
			if pos.token_id == market.win_outcome_token_id { Decimal::ONE } else { Decimal::ZERO }
		} else {
			// open market: 从价格缓存获取
			let field = market_field(event_id, market_id);
			if let Some(price_info) = price_map.get(&field)
				&& let Some(token_price) = price_info.prices.get(&pos.token_id)
			{
				let price_str = calculate_chance(&token_price.best_bid, &token_price.best_ask, &token_price.latest_trade_price);
				Decimal::from_str(&price_str).unwrap_or(Decimal::ZERO)
			} else {
				Decimal::ZERO
			}
		};

		let value = current_price_decimal.checked_mul(quantity).unwrap_or(Decimal::ZERO);
		let cost = avg_price.checked_mul(quantity).unwrap_or(Decimal::ZERO);
		let profit_value = value.checked_sub(cost).unwrap_or(Decimal::ZERO);
		let profit_percentage = if cost > Decimal::ZERO { profit_value.checked_div(cost).unwrap_or(Decimal::ZERO).checked_mul(Decimal::ONE_HUNDRED).unwrap_or(Decimal::ZERO) } else { Decimal::ZERO };

		results.push(SinglePositionResponse {
			event_id,
			market_id,
			event_title: event.title.clone(),
			market_title: market.title.clone(),
			market_question: market.question.clone(),
			event_image: event.image.clone(),
			market_image: market.image.clone(),
			outcome_name: pos.outcome_name.unwrap_or_default(),
			condition_id: market.condition_id.clone(),
			token_id: pos.token_id,
			avg_price: avg_price.normalize(),
			quantity: quantity.normalize(),
			value: value.normalize(),
			profit_value: profit_value.normalize(),
			profit_percentage: profit_percentage.normalize(),
			current_price: current_price_decimal.normalize(),
		});
	}

	// 根据参数排序（只支持一个排序参数）
	sort_positions(&mut results, params);

	let has_more = (params.page as i64 * PAGE_SIZE as i64) < total as i64;
	Ok(PositionsResponse { positions: results, total, has_more })
}

/// 根据参数排序持仓（只支持一个排序参数，默认 DESC，无参数时按 profit_value DESC）
fn sort_positions(results: &mut [SinglePositionResponse], params: &PositionRequest) {
	// 按优先级检查排序参数，true=DESC（默认），false=ASC
	if let Some(desc) = params.value {
		results.sort_by(|a, b| {
			let cmp = a.value.cmp(&b.value);
			if desc { cmp.reverse() } else { cmp }
		});
	} else if let Some(desc) = params.quantity {
		results.sort_by(|a, b| {
			let cmp = a.quantity.cmp(&b.quantity);
			if desc { cmp.reverse() } else { cmp }
		});
	} else if let Some(desc) = params.avg_price {
		results.sort_by(|a, b| {
			let cmp = a.avg_price.cmp(&b.avg_price);
			if desc { cmp.reverse() } else { cmp }
		});
	} else if let Some(desc) = params.profit_value {
		results.sort_by(|a, b| {
			let cmp = a.profit_value.cmp(&b.profit_value);
			if desc { cmp.reverse() } else { cmp }
		});
	} else if let Some(desc) = params.profit_percentage {
		results.sort_by(|a, b| {
			let cmp = a.profit_percentage.cmp(&b.profit_percentage);
			if desc { cmp.reverse() } else { cmp }
		});
	} else {
		// 默认按 profit_value DESC 排序
		results.sort_by(|a, b| b.profit_value.cmp(&a.profit_value));
	}
}

// ============== 已平仓持仓处理 ==============

/// 获取用户已平仓持仓列表（公开接口）
/// GET /closed_positions?uid=xxx
pub async fn handle_closed_positions(Query(params): Query<ClosedPositionsRequest>) -> Result<Json<ApiResponse<ClosedPositionsResponse>>, InternalError> {
	let user_id = params.uid;
	// 构建 singleflight key: uid:event_id:market_id:question:page
	let question_key = params.question.as_deref().unwrap_or("");
	let key = format!("{}:{}:{}:{}:{}", user_id, params.event_id.unwrap_or(0), params.market_id.unwrap_or(0), question_key, params.page);

	let group = get_closed_positions_group().await;
	let params_clone = params.clone();

	let result = group
		.work(&key, async move {
			let read_pool = db::get_db_read_pool()?;
			let (count_sql, query_sql, has_event_filter, has_market_filter, has_question_filter) = build_closed_position_query(&params_clone);

			// 查询总数 - 使用动态绑定
			let total: i64 = bind_closed_position_count_query(&count_sql, user_id, &params_clone, has_event_filter, has_market_filter, has_question_filter)
				.fetch_one(&read_pool)
				.await
				.map_err(|e| anyhow::anyhow!(e))?;

			let total = total as i16;

			// 如果没有数据，直接返回空
			if total == 0 {
				return Ok(ClosedPositionsResponse { positions: vec![], total: 0, has_more: false });
			}

			// 超过 1000 条：只支持 avg_price 和 redeem_timestamp 排序，按分页返回
			if total > PAGE_SIZE {
				return handle_large_closed_positions_internal(&read_pool, user_id, &params_clone, &query_sql, has_event_filter, has_market_filter, has_question_filter, total).await;
			}

			// 小于等于 1000 条：获取所有记录，动态计算后排序
			handle_small_closed_positions_internal(&read_pool, user_id, &params_clone, &query_sql, has_event_filter, has_market_filter, has_question_filter, total).await
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Failed to get closed positions for user {}: {}", user_id, e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight closed_positions leader dropped for user {}", user_id);
			Err(InternalError)
		}
	}
}

/// 动态绑定已平仓持仓查询参数 (用于 count 查询)
fn bind_closed_position_count_query<'a>(
	sql: &'a str,
	user_id: i64,
	params: &'a ClosedPositionsRequest,
	has_event: bool,
	has_market: bool,
	has_question: bool,
) -> sqlx::query::QueryScalar<'a, sqlx::Postgres, i64, sqlx::postgres::PgArguments> {
	let mut query = sqlx::query_scalar(sql).bind(user_id);
	if has_event {
		query = query.bind(params.event_id.expect("event_id checked"));
	}
	if has_market {
		query = query.bind(params.market_id.expect("market_id checked"));
	}
	if has_question {
		query = query.bind(params.question.as_ref().expect("question checked"));
	}
	query
}

/// 动态绑定已平仓持仓查询参数 (用于 data 查询)
fn bind_closed_position_query<'a>(
	sql: &'a str,
	user_id: i64,
	params: &'a ClosedPositionsRequest,
	has_event: bool,
	has_market: bool,
	has_question: bool,
) -> sqlx::query::QueryAs<'a, sqlx::Postgres, Positions, sqlx::postgres::PgArguments> {
	let mut query = sqlx::query_as(sql).bind(user_id);
	if has_event {
		query = query.bind(params.event_id.expect("event_id checked"));
	}
	if has_market {
		query = query.bind(params.market_id.expect("market_id checked"));
	}
	if has_question {
		query = query.bind(params.question.as_ref().expect("question checked"));
	}
	query
}

/// 构建已平仓持仓查询 SQL（redeemed = true）
fn build_closed_position_query(params: &ClosedPositionsRequest) -> (String, String, bool, bool, bool) {
	let has_event_filter = params.event_id.is_some();
	let has_market_filter = params.market_id.is_some();
	let has_question_filter = params.question.as_ref().is_some_and(|q| !q.is_empty());

	// 使用 USDC_TOKEN_ID 常量构建 SQL，过滤 redeemed = true
	let base_where = format!("user_id = $1 AND token_id != '{}' AND redeemed = true", USDC_TOKEN_ID);

	// 动态构建 where 子句，参数按顺序: user_id ($1), event_id, market_id, question
	let mut next_param = 2;
	let mut where_clause = base_where;

	if has_event_filter {
		where_clause = format!("{} AND event_id = ${}", where_clause, next_param);
		next_param += 1;
	}
	if has_market_filter {
		where_clause = format!("{} AND market_id = ${}", where_clause, next_param);
		next_param += 1;
	}
	if has_question_filter {
		where_clause = format!("{} AND question LIKE '%' || ${} || '%'", where_clause, next_param);
	}

	let count_sql = format!("SELECT COUNT(*) FROM positions WHERE {}", where_clause);
	let query_sql = format!("SELECT * FROM positions WHERE {}", where_clause);

	(count_sql, query_sql, has_event_filter, has_market_filter, has_question_filter)
}

/// 处理大量已平仓数据（>1000）：只支持 avg_price 和 redeem_timestamp 排序（内部函数）
async fn handle_large_closed_positions_internal(
	read_pool: &sqlx::PgPool,
	user_id: i64,
	params: &ClosedPositionsRequest,
	query_sql: &str,
	has_event_filter: bool,
	has_market_filter: bool,
	has_question_filter: bool,
	total: i16,
) -> Result<ClosedPositionsResponse, anyhow::Error> {
	// 确定排序字段和顺序（只支持 avg_price 和 redeem_timestamp，默认 redeem_timestamp DESC）
	let (order_field, order) = if let Some(desc) = params.avg_price {
		("avg_price", if desc { "DESC" } else { "ASC" })
	} else {
		// 默认或指定 redeem_timestamp 排序
		let order = if params.redeem_timestamp == Some(false) { "ASC" } else { "DESC" };
		("redeemed_timestamp", order)
	};

	let offset = (params.page.max(1) - 1) as i64 * PAGE_SIZE as i64;
	let full_sql = format!("{} ORDER BY {} {} NULLS LAST LIMIT {} OFFSET {}", query_sql, order_field, order, PAGE_SIZE, offset);

	let positions: Vec<Positions> = bind_closed_position_query(&full_sql, user_id, params, has_event_filter, has_market_filter, has_question_filter).fetch_all(read_pool).await?;

	// 获取 event 信息
	let event_ids: Vec<i64> = positions.iter().filter_map(|p| p.event_id).collect::<std::collections::HashSet<_>>().into_iter().collect();
	let events: Vec<Events> = if !event_ids.is_empty() { sqlx::query_as("SELECT * FROM events WHERE id = ANY($1)").bind(&event_ids).fetch_all(read_pool).await? } else { Vec::new() };
	let event_map: HashMap<i64, &Events> = events.iter().map(|e| (e.id, e)).collect();

	// 构建响应
	let result: Vec<SingleClosedPositionResponse> = positions
		.iter()
		.filter_map(|pos| {
			let (event_id, market_id) = (pos.event_id?, pos.market_id?);
			let event = event_map.get(&event_id)?;
			let market = event.markets.get(&market_id.to_string())?;
			let quantity = pos.balance.checked_add(pos.frozen_balance).unwrap_or(pos.balance);
			let win = pos.token_id == market.win_outcome_token_id;
			let value = pos.usdc_cost.unwrap_or(Decimal::ZERO);
			let payout = pos.payout.unwrap_or(Decimal::ZERO);
			let redeem_timestamp = pos.redeemed_timestamp.map(|t| t.timestamp()).unwrap_or(0);

			// 计算 profit: payout - value (usdc_cost)
			let profit_value = payout.checked_sub(value).unwrap_or(Decimal::ZERO);
			let profit_percentage =
				if value > Decimal::ZERO { profit_value.checked_div(value).unwrap_or(Decimal::ZERO).checked_mul(Decimal::ONE_HUNDRED).unwrap_or(Decimal::ZERO) } else { Decimal::ZERO };

			Some(SingleClosedPositionResponse {
				event_id,
				market_id,
				event_title: event.title.clone(),
				market_title: market.title.clone(),
				market_question: market.question.clone(),
				event_image: event.image.clone(),
				market_image: market.image.clone(),
				outcome_name: pos.outcome_name.clone().unwrap_or_default(),
				token_id: pos.token_id.clone(),
				avg_price: pos.avg_price.unwrap_or(Decimal::ZERO).normalize(),
				quantity: quantity.normalize(),
				value: value.normalize(),
				payout: payout.normalize(),
				profit_value: profit_value.normalize(),
				profit_percentage: profit_percentage.normalize(),
				redeem_timestamp,
				win,
			})
		})
		.collect();

	let has_more = (params.page as i64 * PAGE_SIZE as i64) < total as i64;
	Ok(ClosedPositionsResponse { positions: result, total, has_more })
}

/// 处理小量已平仓数据（<=1000）：获取所有记录，动态计算后排序（内部函数）
async fn handle_small_closed_positions_internal(
	read_pool: &sqlx::PgPool,
	user_id: i64,
	params: &ClosedPositionsRequest,
	query_sql: &str,
	has_event_filter: bool,
	has_market_filter: bool,
	has_question_filter: bool,
	total: i16,
) -> Result<ClosedPositionsResponse, anyhow::Error> {
	// 获取所有记录
	let positions: Vec<Positions> = bind_closed_position_query(query_sql, user_id, params, has_event_filter, has_market_filter, has_question_filter).fetch_all(read_pool).await?;

	if positions.is_empty() {
		return Ok(ClosedPositionsResponse { positions: vec![], total: 0, has_more: false });
	}

	// 获取 event 信息
	let event_ids: Vec<i64> = positions.iter().filter_map(|p| p.event_id).collect::<std::collections::HashSet<_>>().into_iter().collect();
	let events: Vec<Events> = if !event_ids.is_empty() { sqlx::query_as("SELECT * FROM events WHERE id = ANY($1)").bind(&event_ids).fetch_all(read_pool).await? } else { Vec::new() };
	let event_map: HashMap<i64, &Events> = events.iter().map(|e| (e.id, e)).collect();

	// 直接构造 SingleClosedPositionResponse
	let mut results: Vec<SingleClosedPositionResponse> = Vec::with_capacity(positions.len());

	for pos in positions {
		let (event_id, market_id) = match (pos.event_id, pos.market_id) {
			(Some(e), Some(m)) => (e, m),
			_ => continue,
		};

		let event = match event_map.get(&event_id) {
			Some(e) => *e,
			None => continue,
		};

		let market = match event.markets.get(&market_id.to_string()) {
			Some(m) => m,
			None => continue,
		};

		let quantity = pos.balance.checked_add(pos.frozen_balance).unwrap_or(pos.balance);
		let avg_price = pos.avg_price.unwrap_or(Decimal::ZERO);
		let win = pos.token_id == market.win_outcome_token_id;
		let value = pos.usdc_cost.unwrap_or(Decimal::ZERO);
		let payout = pos.payout.unwrap_or(Decimal::ZERO);
		let redeem_timestamp = pos.redeemed_timestamp.map(|t| t.timestamp()).unwrap_or(0);

		// 计算 profit: payout - value (usdc_cost)
		let profit_value = payout.checked_sub(value).unwrap_or(Decimal::ZERO);
		let profit_percentage = if value > Decimal::ZERO { profit_value.checked_div(value).unwrap_or(Decimal::ZERO).checked_mul(Decimal::ONE_HUNDRED).unwrap_or(Decimal::ZERO) } else { Decimal::ZERO };

		results.push(SingleClosedPositionResponse {
			event_id,
			market_id,
			event_title: event.title.clone(),
			market_title: market.title.clone(),
			market_question: market.question.clone(),
			event_image: event.image.clone(),
			market_image: market.image.clone(),
			outcome_name: pos.outcome_name.unwrap_or_default(),
			token_id: pos.token_id,
			avg_price: avg_price.normalize(),
			quantity: quantity.normalize(),
			value: value.normalize(),
			payout: payout.normalize(),
			profit_value: profit_value.normalize(),
			profit_percentage: profit_percentage.normalize(),
			redeem_timestamp,
			win,
		});
	}

	// 根据参数排序
	sort_closed_positions(&mut results, params);

	let has_more = (params.page as i64 * PAGE_SIZE as i64) < total as i64;
	Ok(ClosedPositionsResponse { positions: results, total, has_more })
}

/// 根据参数排序已平仓持仓（支持 4 个排序参数，默认 DESC，无参数时按 redeem_timestamp DESC）
fn sort_closed_positions(results: &mut [SingleClosedPositionResponse], params: &ClosedPositionsRequest) {
	// 按优先级检查排序参数，true=DESC（默认），false=ASC
	if let Some(desc) = params.avg_price {
		results.sort_by(|a, b| {
			let cmp = a.avg_price.cmp(&b.avg_price);
			if desc { cmp.reverse() } else { cmp }
		});
	} else if let Some(desc) = params.profit_value {
		results.sort_by(|a, b| {
			let cmp = a.profit_value.cmp(&b.profit_value);
			if desc { cmp.reverse() } else { cmp }
		});
	} else if let Some(desc) = params.profit_percentage {
		results.sort_by(|a, b| {
			let cmp = a.profit_percentage.cmp(&b.profit_percentage);
			if desc { cmp.reverse() } else { cmp }
		});
	} else if let Some(desc) = params.redeem_timestamp {
		results.sort_by(|a, b| {
			let cmp = a.redeem_timestamp.cmp(&b.redeem_timestamp);
			if desc { cmp.reverse() } else { cmp }
		});
	} else {
		// 默认按 redeem_timestamp DESC 排序
		results.sort_by(|a, b| b.redeem_timestamp.cmp(&a.redeem_timestamp));
	}
}

// ============== 活动历史处理 ==============

/// 获取用户活动历史（公开接口）
/// GET /activity?uid=xxx
pub async fn handle_activity(Query(params): Query<ActivityRequest>) -> Result<Json<ApiResponse<ActivityResponse>>, InternalError> {
	let user_id = params.uid;
	// 构建 singleflight key: uid:event_id:market_id:page:page_size
	let key = format!("{}:{}:{}:{}:{}", user_id, params.event_id.unwrap_or(0), params.market_id.unwrap_or(0), params.page, params.page_size);

	let group = get_activity_group().await;
	let params_clone = params.clone();

	let result = group
		.work(&key, async move {
			let read_pool = db::get_db_read_pool()?;
			let history_types = vec![AssetHistoryType::Split, AssetHistoryType::Merge, AssetHistoryType::Redeem, AssetHistoryType::OnChainBuySuccess, AssetHistoryType::OnChainSellSuccess];

			let has_event_filter = params_clone.event_id.is_some();
			let has_market_filter = params_clone.market_id.is_some();
			let page_size = params_clone.page_size.clamp(1, 100) as i64;
			let offset = (params_clone.page.max(1) - 1) as i64 * page_size;

			// 查询总数
			let count_sql = match (has_event_filter, has_market_filter) {
				(true, true) => "SELECT COUNT(*) FROM operation_history WHERE user_id = $1 AND history_type = ANY($2) AND event_id = $3 AND market_id = $4",
				(true, false) => "SELECT COUNT(*) FROM operation_history WHERE user_id = $1 AND history_type = ANY($2) AND event_id = $3",
				(false, true) => "SELECT COUNT(*) FROM operation_history WHERE user_id = $1 AND history_type = ANY($2) AND market_id = $3",
				(false, false) => "SELECT COUNT(*) FROM operation_history WHERE user_id = $1 AND history_type = ANY($2)",
			};

			let total: i64 = match (has_event_filter, has_market_filter) {
				(true, true) => {
					sqlx::query_scalar(count_sql)
						.bind(user_id)
						.bind(&history_types)
						.bind(params_clone.event_id.expect("event_id checked"))
						.bind(params_clone.market_id.expect("market_id checked"))
						.fetch_one(&read_pool)
						.await
						.map_err(|e| anyhow::anyhow!(e))?
				}
				(true, false) => {
					sqlx::query_scalar(count_sql)
						.bind(user_id)
						.bind(&history_types)
						.bind(params_clone.event_id.expect("event_id checked"))
						.fetch_one(&read_pool)
						.await
						.map_err(|e| anyhow::anyhow!(e))?
				}
				(false, true) => {
					sqlx::query_scalar(count_sql)
						.bind(user_id)
						.bind(&history_types)
						.bind(params_clone.market_id.expect("market_id checked"))
						.fetch_one(&read_pool)
						.await
						.map_err(|e| anyhow::anyhow!(e))?
				}
				(false, false) => sqlx::query_scalar(count_sql).bind(user_id).bind(&history_types).fetch_one(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?,
			};

			if total == 0 {
				return Ok(ActivityResponse { activities: vec![], total: 0, has_more: false });
			}

			// 查询数据
			let query_sql = match (has_event_filter, has_market_filter) {
				(true, true) => {
					format!(
						"SELECT * FROM operation_history WHERE user_id = $1 AND history_type = ANY($2) AND event_id = $3 AND market_id = $4 ORDER BY created_at DESC LIMIT {} OFFSET {}",
						page_size, offset
					)
				}
				(true, false) => {
					format!("SELECT * FROM operation_history WHERE user_id = $1 AND history_type = ANY($2) AND event_id = $3 ORDER BY created_at DESC LIMIT {} OFFSET {}", page_size, offset)
				}
				(false, true) => {
					format!("SELECT * FROM operation_history WHERE user_id = $1 AND history_type = ANY($2) AND market_id = $3 ORDER BY created_at DESC LIMIT {} OFFSET {}", page_size, offset)
				}
				(false, false) => format!("SELECT * FROM operation_history WHERE user_id = $1 AND history_type = ANY($2) ORDER BY created_at DESC LIMIT {} OFFSET {}", page_size, offset),
			};

			let records: Vec<OperationHistory> = match (has_event_filter, has_market_filter) {
				(true, true) => {
					sqlx::query_as(&query_sql)
						.bind(user_id)
						.bind(&history_types)
						.bind(params_clone.event_id.expect("event_id checked"))
						.bind(params_clone.market_id.expect("market_id checked"))
						.fetch_all(&read_pool)
						.await
						.map_err(|e| anyhow::anyhow!(e))?
				}
				(true, false) => {
					sqlx::query_as(&query_sql).bind(user_id).bind(&history_types).bind(params_clone.event_id.expect("event_id checked")).fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?
				}
				(false, true) => {
					sqlx::query_as(&query_sql)
						.bind(user_id)
						.bind(&history_types)
						.bind(params_clone.market_id.expect("market_id checked"))
						.fetch_all(&read_pool)
						.await
						.map_err(|e| anyhow::anyhow!(e))?
				}
				(false, false) => sqlx::query_as(&query_sql).bind(user_id).bind(&history_types).fetch_all(&read_pool).await.map_err(|e| anyhow::anyhow!(e))?,
			};

			// 获取 event 信息（从缓存中批量获取）
			let event_ids: Vec<i64> = records.iter().map(|r| r.event_id).collect::<std::collections::HashSet<_>>().into_iter().collect();
			let events_map = if !event_ids.is_empty() { cache::get_events(&event_ids).await? } else { HashMap::new() };

			// 构建响应
			let activities: Vec<SingleActivityResponse> = records
				.iter()
				.filter_map(|r| {
					let event = events_map.get(&r.event_id)?;
					let market = event.markets.get(&r.market_id.to_string())?;
					let types = match r.history_type {
						AssetHistoryType::Split => "split",
						AssetHistoryType::Merge => "merge",
						AssetHistoryType::Redeem => "redeem",
						AssetHistoryType::OnChainBuySuccess => "buy",
						AssetHistoryType::OnChainSellSuccess => "sell",
						_ => return None,
					};
					Some(SingleActivityResponse {
						event_id: r.event_id,
						market_id: r.market_id,
						event_title: event.title.clone(),
						market_title: market.title.clone(),
						market_question: market.question.clone(),
						event_image: event.image.clone(),
						market_image: market.image.clone(),
						types: types.to_string(),
						timestamp: r.created_at.timestamp(),
						outcome_name: r.outcome_name.clone(),
						price: r.price.map(|p| p.normalize()),
						quantity: r.quantity.unwrap_or(Decimal::ZERO).normalize(),
						value: r.value.unwrap_or(Decimal::ZERO).normalize(),
						tx_hash: r.tx_hash.clone(),
					})
				})
				.collect();

			let has_more = (params_clone.page as i64 * page_size) < total;
			Ok(ActivityResponse { activities, total: total as i16, has_more })
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Failed to get activity for user {}: {}", user_id, e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight activity leader dropped for user {}", user_id);
			Err(InternalError)
		}
	}
}

// ============== 未完成订单处理 ==============

/// 获取用户未完成订单（公开接口，使用 uid 参数）
pub async fn handle_open_orders(Query(params): Query<OpenOrdersRequest>) -> Result<Json<ApiResponse<OpenOrdersResponse>>, InternalError> {
	let user_id = params.uid;
	let key = format!("{}:{}:{}:{}:{}", user_id, params.event_id.unwrap_or(0), params.market_id.unwrap_or(0), params.page, params.page_size);
	let group = get_open_orders_group().await;

	let event_id = params.event_id;
	let market_id = params.market_id;
	let page = params.page;
	let page_size = params.page_size;

	let result = group.work(&key, async move { handle_open_orders_internal(user_id, event_id, market_id, page, page_size).await }).await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Failed to get open orders for user {}: {}", user_id, e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight open orders leader dropped for user {}", user_id);
			Err(InternalError)
		}
	}
}

async fn handle_open_orders_internal(user_id: i64, event_id: Option<i64>, market_id: Option<i16>, page: i16, page_size: i16) -> Result<OpenOrdersResponse, anyhow::Error> {
	// status 为 new 或 partially_filled
	let read_pool = db::get_db_read_pool()?;
	let statuses = vec![OrderStatus::New, OrderStatus::PartiallyFilled];
	let has_event_filter = event_id.is_some();
	let has_market_filter = market_id.is_some();
	let page_size_i64 = page_size.clamp(1, 100) as i64;
	let offset = (page.max(1) - 1) as i64 * page_size_i64;

	// 查询总数
	let count_sql = match (has_event_filter, has_market_filter) {
		(true, true) => "SELECT COUNT(*) FROM orders WHERE user_id = $1 AND status = ANY($2) AND event_id = $3 AND market_id = $4 AND order_type = $5",
		(true, false) => "SELECT COUNT(*) FROM orders WHERE user_id = $1 AND status = ANY($2) AND event_id = $3 AND order_type = $4",
		(false, true) => "SELECT COUNT(*) FROM orders WHERE user_id = $1 AND status = ANY($2) AND market_id = $3 AND order_type = $4",
		(false, false) => "SELECT COUNT(*) FROM orders WHERE user_id = $1 AND status = ANY($2) AND order_type = $3",
	};

	let total: i64 = match (has_event_filter, has_market_filter) {
		(true, true) => {
			sqlx::query_scalar(count_sql)
				.bind(user_id)
				.bind(&statuses)
				.bind(event_id.expect("event_id checked"))
				.bind(market_id.expect("market_id checked"))
				.bind(OrderType::Limit)
				.fetch_one(&read_pool)
				.await?
		}
		(true, false) => sqlx::query_scalar(count_sql).bind(user_id).bind(&statuses).bind(event_id.expect("event_id checked")).bind(OrderType::Limit).fetch_one(&read_pool).await?,
		(false, true) => sqlx::query_scalar(count_sql).bind(user_id).bind(&statuses).bind(market_id.expect("market_id checked")).bind(OrderType::Limit).fetch_one(&read_pool).await?,
		(false, false) => sqlx::query_scalar(count_sql).bind(user_id).bind(&statuses).bind(OrderType::Limit).fetch_one(&read_pool).await?,
	};

	if total == 0 {
		return Ok(OpenOrdersResponse { orders: vec![], total: 0, has_more: false });
	}

	// 查询数据
	let query_sql = match (has_event_filter, has_market_filter) {
		(true, true) => {
			format!(
				"SELECT * FROM orders WHERE user_id = $1 AND status = ANY($2) AND event_id = $3 AND market_id = $4 AND order_type = $5 ORDER BY created_at DESC LIMIT {} OFFSET {}",
				page_size_i64, offset
			)
		}
		(true, false) => {
			format!("SELECT * FROM orders WHERE user_id = $1 AND status = ANY($2) AND event_id = $3 AND order_type = $4 ORDER BY created_at DESC LIMIT {} OFFSET {}", page_size_i64, offset)
		}
		(false, true) => {
			format!("SELECT * FROM orders WHERE user_id = $1 AND status = ANY($2) AND market_id = $3 AND order_type = $4 ORDER BY created_at DESC LIMIT {} OFFSET {}", page_size_i64, offset)
		}
		(false, false) => format!("SELECT * FROM orders WHERE user_id = $1 AND status = ANY($2) AND order_type = $3 ORDER BY created_at DESC LIMIT {} OFFSET {}", page_size_i64, offset),
	};

	let orders: Vec<Orders> = match (has_event_filter, has_market_filter) {
		(true, true) => {
			sqlx::query_as(&query_sql)
				.bind(user_id)
				.bind(&statuses)
				.bind(event_id.expect("event_id checked"))
				.bind(market_id.expect("market_id checked"))
				.bind(OrderType::Limit)
				.fetch_all(&read_pool)
				.await?
		}
		(true, false) => sqlx::query_as(&query_sql).bind(user_id).bind(&statuses).bind(event_id.expect("event_id checked")).bind(OrderType::Limit).fetch_all(&read_pool).await?,
		(false, true) => sqlx::query_as(&query_sql).bind(user_id).bind(&statuses).bind(market_id.expect("market_id checked")).bind(OrderType::Limit).fetch_all(&read_pool).await?,
		(false, false) => sqlx::query_as(&query_sql).bind(user_id).bind(&statuses).bind(OrderType::Limit).fetch_all(&read_pool).await?,
	};

	// 获取 event 信息（从缓存中批量获取）
	let event_ids: Vec<i64> = orders.iter().map(|o| o.event_id).collect::<std::collections::HashSet<_>>().into_iter().collect();
	let events_map = if !event_ids.is_empty() { cache::get_events(&event_ids).await? } else { HashMap::new() };

	// 构建响应
	let result: Vec<SingleOpenOrderResponse> = orders
		.iter()
		.filter_map(|o| {
			let event = events_map.get(&o.event_id)?;
			let market = event.markets.get(&o.market_id.to_string())?;
			Some(SingleOpenOrderResponse {
				event_id: o.event_id,
				market_id: o.market_id,
				event_title: event.title.clone(),
				market_title: market.title.clone(),
				market_question: market.question.clone(),
				event_image: event.image.clone(),
				market_image: market.image.clone(),
				order_id: o.id.to_string(),
				side: format!("{:?}", o.order_side).to_lowercase(),
				outcome_name: o.outcome.clone(),
				price: o.price.normalize(),
				quantity: o.quantity.normalize(),
				filled_quantity: o.filled_quantity.normalize(),
				volume: o.volume.normalize(),
				created_at: o.created_at.timestamp(),
			})
		})
		.collect();

	let has_more = (page as i64 * page_size_i64) < total;
	Ok(OpenOrdersResponse { orders: result, total: total as i16, has_more })
}

// ============== 深度数据处理 ==============

/// 获取市场深度数据（公开接口，不需要鉴权）
/// 使用 singleflight 确保并发请求共享同一个数据库查询
pub async fn handle_depth(Query(params): Query<DepthRequest>) -> Result<Json<ApiResponse<DepthResponse>>, InternalError> {
	let singleflight_key = format!("{}_{}", params.event_id, params.market_id);

	let result = get_depth_group()
		.await
		.work(&singleflight_key, async {
			let field = market_field(params.event_id, params.market_id);

			// 从 Redis 缓存获取深度数据
			let mut conn = redis_pool::get_cache_redis_connection().await.map_err(|e| anyhow::anyhow!(e))?;
			let depth_json: Option<String> = conn.hget(DEPTH_CACHE_KEY, &field).await.map_err(|e| anyhow::anyhow!(e))?;

			let depth_info = match depth_json {
				Some(json_str) => serde_json::from_str::<WebSocketDepth>(&json_str).map_err(|e| anyhow::anyhow!(e))?,
				None => {
					// 没有缓存数据，返回空响应
					return Ok(DepthResponse { update_id: 0, timestamp: 0, depths: HashMap::new() });
				}
			};

			// 构建响应
			let mut depths: HashMap<String, DepthBook> = HashMap::new();
			for (token_id, token_depth) in depth_info.depths {
				depths.insert(token_id, DepthBook { latest_trade_price: token_depth.latest_trade_price, bids: token_depth.bids, asks: token_depth.asks });
			}

			Ok(DepthResponse { update_id: depth_info.update_id, timestamp: depth_info.timestamp, depths })
		})
		.await;

	match result {
		Ok(response) => Ok(Json(ApiResponse::success(response))),
		Err(Some(e)) => {
			tracing::error!("Singleflight depth leader failed for event_id={}, market_id={}: {}", params.event_id, params.market_id, e);
			Err(InternalError)
		}
		Err(None) => {
			tracing::error!("Singleflight depth leader dropped for event_id={}, market_id={}", params.event_id, params.market_id);
			Err(InternalError)
		}
	}
}

/// GET /event_balance - 获取用户在指定事件的余额（可选指定 market_id）
pub async fn handle_event_balance(Query(params): Query<EventBalanceRequest>, Extension(client_info): Extension<ClientInfo>) -> Result<Json<ApiResponse<EventBalanceResponse>>, InternalError> {
	// 鉴权
	let auth_result = match get_user_id_from_auth(&client_info, &client_info.request_id).await {
		Ok(result) => result,
		Err(e) => return Ok(Json(ApiResponse::error(e.to_api_error_code()))),
	};
	let user_id = auth_result.user_id;
	let privy_id = auth_result.privy_id;

	let event_id = params.event_id;

	// 获取数据库连接
	let read_pool = db::get_db_read_pool().map_err(|e| {
		tracing::error!("request_id={}, privy_id={} - Failed to get db read pool: {}", client_info.request_id, privy_id, e);
		InternalError
	})?;

	// 根据是否有 market_id 决定查询范围
	let all_token_ids = if let Some(market_id) = params.market_id {
		// 查询特定市场的 token_ids
		let market = cache::get_market(event_id, market_id).await.map_err(|e| {
			tracing::error!("request_id={}, privy_id={} - Failed to get market from cache: event_id={}, market_id={}, error={}", client_info.request_id, privy_id, event_id, market_id, e);
			InternalError
		})?;
		let mut token_ids = market.outcome_token_ids.clone();
		token_ids.push(USDC_TOKEN_ID.to_string());
		token_ids
	} else {
		// 查询整个事件的所有 token_ids
		let event = cache::get_event(event_id).await.map_err(|e| {
			tracing::error!("request_id={}, privy_id={} - Failed to get event from cache: event_id={}, error={}", client_info.request_id, privy_id, event_id, e);
			InternalError
		})?;
		let mut token_ids: Vec<String> = event.markets.values().flat_map(|market| market.outcome_token_ids.clone()).collect();
		token_ids.push(USDC_TOKEN_ID.to_string());
		token_ids
	};

	// 一次性查询用户的 USDC 余额和所有 token 余额
	let positions: Vec<Positions> =
		sqlx::query_as("SELECT * FROM positions WHERE user_id = $1 AND token_id = ANY($2)").bind(user_id).bind(&all_token_ids).fetch_all(&read_pool).await.map_err(|e| {
			tracing::error!("request_id={}, privy_id={} - Failed to query positions: {}", client_info.request_id, privy_id, e);
			InternalError
		})?;

	// 构建 token_id -> balance 的映射
	let position_map: HashMap<String, Decimal> = positions.iter().map(|p| (p.token_id.clone(), p.balance)).collect();

	// 获取 USDC 余额（如果小于等于 0，则设为 0）
	let cash = position_map.get(USDC_TOKEN_ID).copied().unwrap_or(Decimal::ZERO);
	let cash = if cash.le(&Decimal::ZERO) { Decimal::ZERO } else { cash };

	// 构建各 token 余额（不包括 USDC）
	let mut token_available: HashMap<String, String> = HashMap::new();
	for token_id in &all_token_ids {
		if token_id == USDC_TOKEN_ID {
			continue; // 跳过 USDC
		}
		let balance = position_map.get(token_id).copied().unwrap_or(Decimal::ZERO);
		// 如果余额小于等于 0，则设为 0
		let balance = if balance.le(&Decimal::ZERO) { Decimal::ZERO } else { balance };
		token_available.insert(token_id.clone(), balance.normalize().to_string());
	}

	Ok(Json(ApiResponse::success(EventBalanceResponse { token_available, cash_available: cash.normalize() })))
}

/// 获取用户委托历史（所有订单）
pub async fn handle_order_history(Extension(client_info): Extension<ClientInfo>, Query(params): Query<OrderHistoryRequest>) -> Result<Json<ApiResponse<OrderHistoryResponse>>, InternalError> {
	// 鉴权
	let auth_result = match get_user_id_from_auth(&client_info, &client_info.request_id).await {
		Ok(result) => result,
		Err(e) => return Ok(Json(ApiResponse::error(e.to_api_error_code()))),
	};
	let user_id = auth_result.user_id;
	let privy_id = auth_result.privy_id;

	// 获取数据库连接池
	let read_pool = db::get_db_read_pool().map_err(|e| {
		tracing::error!("request_id={}, privy_id={} - Failed to get db read pool: {}", client_info.request_id, privy_id, e);
		InternalError
	})?;

	let has_event_filter = params.event_id.is_some();
	let has_market_filter = params.market_id.is_some();
	let page_size = params.page_size.clamp(1, 100) as i64;
	let offset = (params.page.max(1) - 1) as i64 * page_size;

	// 查询总数
	let count_sql = match (has_event_filter, has_market_filter) {
		(true, true) => "SELECT COUNT(*) FROM orders WHERE user_id = $1 AND event_id = $2 AND market_id = $3",
		(true, false) => "SELECT COUNT(*) FROM orders WHERE user_id = $1 AND event_id = $2",
		(false, true) => "SELECT COUNT(*) FROM orders WHERE user_id = $1 AND market_id = $2",
		(false, false) => "SELECT COUNT(*) FROM orders WHERE user_id = $1",
	};

	let total: i64 = match (has_event_filter, has_market_filter) {
		(true, true) => {
			sqlx::query_scalar(count_sql).bind(user_id).bind(params.event_id.expect("event_id checked")).bind(params.market_id.expect("market_id checked")).fetch_one(&read_pool).await.map_err(
				|e| {
					tracing::error!("request_id={}, privy_id={} - Failed to query order count: {}", client_info.request_id, privy_id, e);
					InternalError
				},
			)?
		}
		(true, false) => {
			sqlx::query_scalar(count_sql).bind(user_id).bind(params.event_id.expect("event_id checked")).fetch_one(&read_pool).await.map_err(|e| {
				tracing::error!("request_id={}, privy_id={} - Failed to query order count: {}", client_info.request_id, privy_id, e);
				InternalError
			})?
		}
		(false, true) => {
			sqlx::query_scalar(count_sql).bind(user_id).bind(params.market_id.expect("market_id checked")).fetch_one(&read_pool).await.map_err(|e| {
				tracing::error!("request_id={}, privy_id={} - Failed to query order count: {}", client_info.request_id, privy_id, e);
				InternalError
			})?
		}
		(false, false) => {
			sqlx::query_scalar(count_sql).bind(user_id).fetch_one(&read_pool).await.map_err(|e| {
				tracing::error!("request_id={}, privy_id={} - Failed to query order count: {}", client_info.request_id, privy_id, e);
				InternalError
			})?
		}
	};

	if total == 0 {
		return Ok(Json(ApiResponse::success(OrderHistoryResponse { order_history: vec![], total: 0, has_more: false })));
	}

	// 查询数据（按创建时间倒序）
	let query_sql = match (has_event_filter, has_market_filter) {
		(true, true) => format!("SELECT * FROM orders WHERE user_id = $1 AND event_id = $2 AND market_id = $3 ORDER BY created_at DESC LIMIT {} OFFSET {}", page_size, offset),
		(true, false) => format!("SELECT * FROM orders WHERE user_id = $1 AND event_id = $2 ORDER BY created_at DESC LIMIT {} OFFSET {}", page_size, offset),
		(false, true) => format!("SELECT * FROM orders WHERE user_id = $1 AND market_id = $2 ORDER BY created_at DESC LIMIT {} OFFSET {}", page_size, offset),
		(false, false) => format!("SELECT * FROM orders WHERE user_id = $1 ORDER BY created_at DESC LIMIT {} OFFSET {}", page_size, offset),
	};

	let records: Vec<Orders> = match (has_event_filter, has_market_filter) {
		(true, true) => {
			sqlx::query_as(&query_sql).bind(user_id).bind(params.event_id.expect("event_id checked")).bind(params.market_id.expect("market_id checked")).fetch_all(&read_pool).await.map_err(|e| {
				tracing::error!("request_id={}, privy_id={} - Failed to query orders: {}", client_info.request_id, privy_id, e);
				InternalError
			})?
		}
		(true, false) => {
			sqlx::query_as(&query_sql).bind(user_id).bind(params.event_id.expect("event_id checked")).fetch_all(&read_pool).await.map_err(|e| {
				tracing::error!("request_id={}, privy_id={} - Failed to query orders: {}", client_info.request_id, privy_id, e);
				InternalError
			})?
		}
		(false, true) => {
			sqlx::query_as(&query_sql).bind(user_id).bind(params.market_id.expect("market_id checked")).fetch_all(&read_pool).await.map_err(|e| {
				tracing::error!("request_id={}, privy_id={} - Failed to query orders: {}", client_info.request_id, privy_id, e);
				InternalError
			})?
		}
		(false, false) => {
			sqlx::query_as(&query_sql).bind(user_id).fetch_all(&read_pool).await.map_err(|e| {
				tracing::error!("request_id={}, privy_id={} - Failed to query orders: {}", client_info.request_id, privy_id, e);
				InternalError
			})?
		}
	};

	// 获取 event 信息（从缓存中批量获取）
	let event_ids: Vec<i64> = records.iter().map(|r| r.event_id).collect::<std::collections::HashSet<_>>().into_iter().collect();
	let events_map = if !event_ids.is_empty() {
		cache::get_events(&event_ids).await.map_err(|e| {
			tracing::error!("request_id={}, privy_id={} - Failed to get events from cache: {}", client_info.request_id, privy_id, e);
			InternalError
		})?
	} else {
		HashMap::new()
	};

	// 构建响应
	let order_history: Vec<SingleOrderHistoryResponse> = records
		.iter()
		.filter_map(|r| {
			let event = events_map.get(&r.event_id)?;
			let market = event.markets.get(&r.market_id.to_string())?;

			// 根据订单类型选择对应的字段
			let (price, quantity, volume, filled_quantity, cancelled_quantity) = match r.order_type {
				common::engine_types::OrderType::Market => {
					// 市价单：使用 market_* 字段
					let market_price = if r.market_filled_quantity > rust_decimal::Decimal::ZERO {
						// 计算平均价格：market_volume / market_filled_quantity
						r.market_filled_volume.checked_div(r.market_filled_quantity).unwrap_or(r.market_price).trunc_with_scale(4).normalize()
					} else {
						// 如果没有成交，使用提交时的市价单价格
						r.market_price.normalize()
					};
					(market_price, r.market_quantity.normalize(), r.market_volume.normalize(), r.market_filled_quantity.normalize(), r.market_cancelled_quantity.normalize())
				}
				common::engine_types::OrderType::Limit => {
					// 限价单：使用普通字段
					(r.price.normalize(), r.quantity.normalize(), r.volume.normalize(), r.filled_quantity.normalize(), r.cancelled_quantity.normalize())
				}
			};

			Some(SingleOrderHistoryResponse {
				order_id: r.id.to_string(),
				event_title: event.title.clone(),
				event_image: event.image.clone(),
				market_title: market.title.clone(),
				market_question: market.question.clone(),
				market_image: market.image.clone(),
				token_id: r.token_id.clone(),
				outcome: r.outcome.clone(),
				order_side: r.order_side,
				order_type: r.order_type,
				price,
				quantity,
				volume,
				filled_quantity,
				cancelled_quantity,
				status: r.status,
				created_at: r.created_at.timestamp(),
				updated_at: r.updated_at.timestamp(),
			})
		})
		.collect();

	let has_more = (params.page as i64 * page_size) < total;
	Ok(Json(ApiResponse::success(OrderHistoryResponse { order_history, total: total as i16, has_more })))
}
