use {proto::asset_service_client::AssetServiceClient, tokio::sync::OnceCell, tonic::transport::Channel, tracing::info};

static ASSET_CLIENT: OnceCell<AssetServiceClient<Channel>> = OnceCell::const_new();

/// 初始化 Asset RPC 客户端
pub async fn init_asset_client() -> anyhow::Result<()> {
	let env = crate::common_env::get_common_env();
	let addr = env.asset_rpc_url.clone();

	info!("Connecting to asset RPC service at: {}", addr);
	let client = AssetServiceClient::connect(addr).await?;
	info!("Connected to asset RPC service");

	ASSET_CLIENT.set(client).map_err(|_| anyhow::anyhow!("Asset client already initialized"))?;
	Ok(())
}

/// 获取 Asset RPC 客户端
fn get_asset_client() -> AssetServiceClient<Channel> {
	ASSET_CLIENT.get().expect("Asset client not initialized").clone()
}

// ============================================================================
// API Service Methods
// ============================================================================

/// 调用 CreateOrder RPC
pub async fn create_order(request: proto::CreateOrderRequest) -> anyhow::Result<proto::CreateOrderResponse> {
	let mut client = get_asset_client();
	let response = client.create_order(request).await?;
	Ok(response.into_inner())
}

// ============================================================================
// Processor Service Methods
// ============================================================================

/// 调用 OrderRejected RPC
pub async fn order_rejected(request: proto::OrderRejectedRequest) -> anyhow::Result<proto::Response> {
	let mut client = get_asset_client();
	let response = client.order_rejected(request).await?;
	Ok(response.into_inner())
}

/// 调用 CancelOrder RPC
pub async fn cancel_order(request: proto::CancelOrderRequest) -> anyhow::Result<proto::ResponseWithUpdateId> {
	let mut client = get_asset_client();
	let response = client.cancel_order(request).await?;
	Ok(response.into_inner())
}

/// 调用 Trade RPC
pub async fn trade(request: proto::TradeRequest) -> anyhow::Result<proto::TradeResponse> {
	let mut client = get_asset_client();
	let response = client.trade(request).await?;
	Ok(response.into_inner())
}

// ============================================================================
// OnchainMsg Service Methods
// ============================================================================

/// 调用 Deposit RPC
pub async fn deposit(request: proto::DepositRequest) -> anyhow::Result<proto::Response> {
	let mut client = get_asset_client();
	let response = client.deposit(request).await?;
	Ok(response.into_inner())
}

/// 调用 Withdraw RPC
pub async fn withdraw(request: proto::WithdrawRequest) -> anyhow::Result<proto::Response> {
	let mut client = get_asset_client();
	let response = client.withdraw(request).await?;
	Ok(response.into_inner())
}

/// 调用 Split RPC
pub async fn split(request: proto::SplitRequest) -> anyhow::Result<proto::Response> {
	let mut client = get_asset_client();
	let response = client.split(request).await?;
	Ok(response.into_inner())
}

/// 调用 Merge RPC
pub async fn merge(request: proto::MergeRequest) -> anyhow::Result<proto::Response> {
	let mut client = get_asset_client();
	let response = client.merge(request).await?;
	Ok(response.into_inner())
}

/// 调用 Redeem RPC
pub async fn redeem(request: proto::RedeemRequest) -> anyhow::Result<proto::Response> {
	let mut client = get_asset_client();
	let response = client.redeem(request).await?;
	Ok(response.into_inner())
}

/// 调用 TradeOnchainSendResult RPC
pub async fn trade_onchain_send_result(request: proto::TradeOnchainSendResultRequest) -> anyhow::Result<proto::Response> {
	let mut client = get_asset_client();
	let response = client.trade_onchain_send_result(request).await?;
	Ok(response.into_inner())
}
