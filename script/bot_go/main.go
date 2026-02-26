package main

import (
	"bytes"
	"crypto/ecdsa"
	"database/sql"
	"encoding/json"
	"fmt"
	"io"
	"log"
	"math/rand"
	"net/http"
	"os"
	"time"

	"bot_go/eip712"

	"github.com/ethereum/go-ethereum/common/hexutil"
	"github.com/ethereum/go-ethereum/crypto"
	_ "github.com/lib/pq"
	"github.com/shopspring/decimal"
)

// 配置常量
const (
	// API 地址
	PrivyNonceURL = "https://auth.privy.io/api/v1/siwe/init"
	PrivyAuthURL  = "https://auth.privy.io/api/v1/siwe/authenticate"
	APIBaseURL    = "https://predictionmarket-api-290128242879.asia-northeast1.run.app/api" // 预测市场 API 地址

	// Privy 请求头
	PrivyAppID  = "cmi5m5vdz006lks0cbixho6k0"
	PrivyClient = "react-auth:3.6.1"
	PrivyOrigin = "https://deepsense-website-290128242879.asia-northeast1.run.app"

	// 数据库配置
	DBHost     = "34.146.110.159"
	DBPort     = 5432
	DBUser     = "postgres"
	DBPassword = "0gZUDGsz1sFy0avm2VHd!"
	DBName     = "deepsense"

	// 链 ID
	ChainID = 97

	// 定时执行间隔
	IntervalMinutes = 30

	// 下单金额 (USDC)
	OrderUSDC = 2.0
)

// 账户信息
var (
	// 账户1: 吃单账号 (user_id=16)
	Account1PrivateKey           = "3f060945b644e0f3d1b9db8481dcdc62c7f8cd6628c8c271c983f0db6e279653"
	Account1Address              = "0x62924ea9188Ad1228eEa76931B595c781b72b664"
	Account1FetchTokenPrivateKey = "b0be8b6d672323dbbd54c5130c70b4a4384560104b7f19e9b9c7bbc674b10e51"
	Account1FetchTokenPublicKey  = "0xC130e75851A2cF13D3BdB0D76471F9f30Cab136A"
	Account1ApiKey               = "cmio6moiu00s1jx0b7oaro1kt"
	// 账户2: 挂单账号 (user_id=26)
	Account2PrivateKey           = "78fb9ba7c9796c3c22067862f3841d4051ec198b92e1ce84c81772ec6e0dfa72"
	Account2Address              = "0xF3D4d60F7562e505383d992E33e8E3cf5e79A7de"
	Account2FetchTokenPrivateKey = "3698259e1c6623f313e59e30d194045efb1cd94f0d7fea85e423fc0ee4c13282"
	Account2FetchTokenPublicKey  = "0x3407C5690e06c2A477C821F20D568Ce3c1692D9b"
	Account2ApiKey               = "cmj2ivxmb00owl40cvtmuz2j7"
)

// NonceResponse Privy nonce 响应
type NonceResponse struct {
	Nonce     string `json:"nonce"`
	Address   string `json:"address"`
	ExpiresAt string `json:"expires_at"`
}

// AuthResponse Privy 认证响应
type AuthResponse struct {
	User          any    `json:"user"`
	Token         string `json:"token"`
	IdentityToken string `json:"identity_token"`
}

// Event 数据库中的事件
type Event struct {
	ID      int64
	Title   string
	Markets map[string]Market
}

// Market 市场信息
type Market struct {
	ID       int16
	Title    string
	TokenIDs []string
	Outcomes []string
	Closed   bool
}

// DepthResponse API 深度响应
type DepthResponse struct {
	Code int    `json:"code"`
	Msg  string `json:"msg"`
	Data struct {
		UpdateID  uint64               `json:"update_id"`
		Timestamp int64                `json:"timestamp"`
		Depths    map[string]DepthBook `json:"depths"`
	} `json:"data"`
}

// DepthBook 深度订单簿
type DepthBook struct {
	LatestTradePrice string           `json:"latest_trade_price"`
	Bids             []PriceLevelInfo `json:"bids"`
	Asks             []PriceLevelInfo `json:"asks"`
}

// PriceLevelInfo 价格档位信息
type PriceLevelInfo struct {
	Price    string `json:"price"`
	Quantity string `json:"quantity"`
}

// PlaceOrderRequest 下单请求
type PlaceOrderRequest struct {
	Expiration    string `json:"expiration"`
	FeeRateBps    string `json:"feeRateBps"`
	Maker         string `json:"maker"`
	MakerAmount   string `json:"makerAmount"`
	Nonce         string `json:"nonce"`
	Salt          int64  `json:"salt"`
	Side          string `json:"side"`
	Signature     string `json:"signature"`
	SignatureType int    `json:"signatureType"`
	Signer        string `json:"signer"`
	Taker         string `json:"taker"`
	TakerAmount   string `json:"takerAmount"`
	TokenId       string `json:"tokenId"`
	EventID       int64  `json:"event_id"`
	MarketID      int16  `json:"market_id"`
	Price         string `json:"price"`
	OrderType     string `json:"order_type"`
}

// PlaceOrderResponse 下单响应
type PlaceOrderResponse struct {
	Code int    `json:"code"`
	Msg  string `json:"msg"`
	Data string `json:"data"`
}

// PersonalSign 使用以太坊私钥签名消息
func PersonalSign(message string, privateKey *ecdsa.PrivateKey) (string, error) {
	fullMessage := fmt.Sprintf("\x19Ethereum Signed Message:\n%d%s", len(message), message)
	hash := crypto.Keccak256Hash([]byte(fullMessage))
	signatureBytes, err := crypto.Sign(hash.Bytes(), privateKey)
	if err != nil {
		return "", err
	}
	signatureBytes[64] += 27
	return hexutil.Encode(signatureBytes), nil
}

// GetPrivyNonce 获取 Privy nonce
func GetPrivyNonce(address string) (*NonceResponse, error) {
	payload := map[string]string{"address": address}
	jsonData, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}

	req, err := http.NewRequest("POST", PrivyNonceURL, bytes.NewBuffer(jsonData))
	if err != nil {
		return nil, err
	}

	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", PrivyOrigin)
	req.Header.Set("Referer", PrivyOrigin+"/")
	req.Header.Set("privy-app-id", PrivyAppID)
	req.Header.Set("privy-client", PrivyClient)

	client := &http.Client{Timeout: 30 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}

	if resp.StatusCode != 200 {
		return nil, fmt.Errorf("nonce request failed: %s", string(body))
	}

	var nonceResp NonceResponse
	if err := json.Unmarshal(body, &nonceResp); err != nil {
		return nil, err
	}

	return &nonceResp, nil
}

// GetPrivyToken 获取 Privy token
func GetPrivyToken(address, privateKeyHex, nonce string) (*AuthResponse, error) {
	// 构建 SIWE 消息
	issuedAt := time.Now().UTC().Format("2006-01-02T15:04:05.000Z")
	message := fmt.Sprintf("deepsense-website-290128242879.asia-northeast1.run.app wants you to sign in with your Ethereum account:\n%s\n\nBy signing, you are proving you own this wallet and logging in. This does not initiate a transaction or cost any fees.\n\nURI: https://deepsense-website-290128242879.asia-northeast1.run.app\nVersion: 1\nChain ID: %d\nNonce: %s\nIssued At: %s\nResources:\n- https://privy.io", address, ChainID, nonce, issuedAt)

	log.Println("message: ", message)
	// 签名
	privKey, err := crypto.HexToECDSA(privateKeyHex)
	if err != nil {
		return nil, fmt.Errorf("invalid private key: %v", err)
	}

	signature, err := PersonalSign(message, privKey)
	if err != nil {
		return nil, fmt.Errorf("sign failed: %v", err)
	}

	// 构建请求
	payload := map[string]any{
		"message":          message,
		"signature":        signature,
		"walletClientType": "metamask",
		"connectorType":    "injected",
		"mode":             "login-or-sign-up",
		//"chainId":          fmt.Sprintf("eip155:%d", ChainID),
	}
	jsonData, err := json.Marshal(payload)
	if err != nil {
		return nil, err
	}

	req, err := http.NewRequest("POST", PrivyAuthURL, bytes.NewBuffer(jsonData))
	if err != nil {
		return nil, err
	}

	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Origin", PrivyOrigin)
	req.Header.Set("Referer", PrivyOrigin+"/")
	req.Header.Set("privy-app-id", PrivyAppID)
	req.Header.Set("privy-client", PrivyClient)
	// req.Header.Set("privy-ca-id", "24f5d304-8f84-41c7-bf34-638a957152b7")
	// req.Header.Set("User-Agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/143.0.0.0 Safari/537.36")

	client := &http.Client{Timeout: 30 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}

	if resp.StatusCode != 200 {
		return nil, fmt.Errorf("auth request failed: %s", string(body))
	}

	var authResp AuthResponse
	if err := json.Unmarshal(body, &authResp); err != nil {
		return nil, err
	}

	return &authResp, nil
}

// Authenticate 完整的认证流程
func Authenticate(address, privateKey string) (string, error) {
	log.Printf("Authenticating %s...", address)

	// 1. 获取 nonce
	nonceResp, err := GetPrivyNonce(address)
	if err != nil {
		return "", fmt.Errorf("get nonce failed: %v", err)
	}
	log.Printf("Got nonce: %s", nonceResp.Nonce)

	// 2. 获取 token
	authResp, err := GetPrivyToken(address, privateKey, nonceResp.Nonce)
	if err != nil {
		return "", fmt.Errorf("get token failed: %v", err)
	}
	log.Printf("Got identity_token for %s", address)

	return authResp.IdentityToken, nil
}

// GetActiveEvents 从数据库获取活跃事件（未关闭、未解决、未过期）
func GetActiveEvents(db *sql.DB) ([]Event, error) {
	query := `
		SELECT id, title, markets
		FROM events
		WHERE closed = false AND resolved = false AND (end_date IS NULL OR end_date > NOW())
		ORDER BY id
	`
	rows, err := db.Query(query)
	if err != nil {
		return nil, err
	}
	defer rows.Close()

	var events []Event
	for rows.Next() {
		var e Event
		var marketsJSON string
		if err := rows.Scan(&e.ID, &e.Title, &marketsJSON); err != nil {
			return nil, err
		}

		// 解析 markets JSON
		var marketsMap map[string]struct {
			ID       int16    `json:"id"`
			Title    string   `json:"title"`
			TokenIDs []string `json:"token_ids"`
			Outcomes []string `json:"outcomes"`
			Closed   bool     `json:"closed"`
		}
		if err := json.Unmarshal([]byte(marketsJSON), &marketsMap); err != nil {
			log.Printf("Failed to parse markets for event %d: %v", e.ID, err)
			continue
		}

		e.Markets = make(map[string]Market)
		for key, m := range marketsMap {
			e.Markets[key] = Market{
				ID:       m.ID,
				Title:    m.Title,
				TokenIDs: m.TokenIDs,
				Outcomes: m.Outcomes,
				Closed:   m.Closed,
			}
		}

		events = append(events, e)
	}

	return events, nil
}

// GetDepth 获取市场深度
func GetDepth(eventID int64, marketID int16) (*DepthResponse, error) {
	url := fmt.Sprintf("%s/depth?event_id=%d&market_id=%d", APIBaseURL, eventID, marketID)

	resp, err := http.Get(url)
	if err != nil {
		return nil, err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return nil, err
	}

	// 检查 HTTP 状态码
	if resp.StatusCode != 200 {
		return nil, fmt.Errorf("HTTP %d: %s", resp.StatusCode, string(body[:min(len(body), 200)]))
	}

	var depthResp DepthResponse
	if err := json.Unmarshal(body, &depthResp); err != nil {
		return nil, err
	}

	if depthResp.Code != 0 {
		return nil, fmt.Errorf("depth API error: %s", depthResp.Msg)
	}

	return &depthResp, nil
}

// SignOrderLocal 使用本地 eip712 模块签名订单
func SignOrderLocal(privateKey string, order *eip712.OrderInput) (string, error) {
	return eip712.SignOrderInput(privateKey, ChainID, order)
}

// CancelAllOrders 取消所有未完成订单
func CancelAllOrders(apiKey string) error {
	req, err := http.NewRequest("POST", APIBaseURL+"/cancel_all_orders", nil)
	if err != nil {
		return err
	}

	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("x-api-key", apiKey)

	client := &http.Client{Timeout: 30 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}

	var result struct {
		Code int    `json:"code"`
		Msg  string `json:"msg"`
	}
	if err := json.Unmarshal(body, &result); err != nil {
		return fmt.Errorf("parse response failed: %v, body: %s", err, string(body))
	}

	if result.Code != 0 {
		return fmt.Errorf("cancel all orders failed: %s", result.Msg)
	}

	log.Printf("All orders cancelled successfully")
	return nil
}

// PlaceOrder 下单
func PlaceOrder(apiKey string, order *PlaceOrderRequest) error {
	jsonData, err := json.Marshal(order)
	if err != nil {
		return err
	}

	req, err := http.NewRequest("POST", APIBaseURL+"/place_order", bytes.NewBuffer(jsonData))
	if err != nil {
		return err
	}

	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("x-api-key", apiKey)

	client := &http.Client{Timeout: 30 * time.Second}
	resp, err := client.Do(req)
	if err != nil {
		return err
	}
	defer resp.Body.Close()

	body, err := io.ReadAll(resp.Body)
	if err != nil {
		return err
	}

	var orderResp PlaceOrderResponse
	if err := json.Unmarshal(body, &orderResp); err != nil {
		return fmt.Errorf("parse response failed: %v, body: %s", err, string(body))
	}

	if orderResp.Code != 0 {
		return fmt.Errorf("place order failed: %s", orderResp.Msg)
	}

	log.Printf("Order placed successfully, order_id=%s", orderResp.Data)
	return nil
}

// CreateBuyOrder 创建买单
func CreateBuyOrder(privateKey, address, tokenID string, price decimal.Decimal, shares int64, eventID int64, marketID int16) (*PlaceOrderRequest, error) {
	salt := time.Now().Unix()

	// 10^18
	unit := decimal.NewFromInt(10).Pow(decimal.NewFromInt(18))

	// takerAmount = shares * 10^18
	takerAmount := decimal.NewFromInt(shares).Mul(unit).String()

	// makerAmount = (shares * price) * 10^18
	makerAmount := decimal.NewFromInt(shares).Mul(price).Mul(unit).String()

	// 构建签名订单
	orderInput := &eip712.OrderInput{
		Salt:          fmt.Sprintf("%d", salt),
		Maker:         address,
		Signer:        address,
		Taker:         "0x0000000000000000000000000000000000000000",
		TokenId:       tokenID,
		MakerAmount:   makerAmount,
		TakerAmount:   takerAmount,
		Expiration:    "0",
		Nonce:         "0",
		FeeRateBps:    "0",
		Side:          0, // buy
		SignatureType: 0,
	}

	// 使用本地 eip712 模块签名
	signature, err := SignOrderLocal(privateKey, orderInput)
	if err != nil {
		return nil, fmt.Errorf("sign order failed: %v", err)
	}

	// 构建下单请求
	orderReq := &PlaceOrderRequest{
		Expiration:    "0",
		FeeRateBps:    "0",
		Maker:         address,
		MakerAmount:   makerAmount,
		Nonce:         "0",
		Salt:          salt,
		Side:          "buy",
		Signature:     signature,
		SignatureType: 0,
		Signer:        address,
		Taker:         "0x0000000000000000000000000000000000000000",
		TakerAmount:   takerAmount,
		TokenId:       tokenID,
		EventID:       eventID,
		MarketID:      marketID,
		Price:         price.String(),
		OrderType:     "limit",
	}

	return orderReq, nil
}

// ProcessMarket 处理单个市场
func ProcessMarket(event Event, market Market) error {
	log.Printf("Processing event %d (%s), market %d (%s)", event.ID, event.Title, market.ID, market.Title)

	if market.Closed {
		log.Printf("Market %d is closed, skipping", market.ID)
		return nil
	}

	if len(market.TokenIDs) < 2 {
		log.Printf("Market %d has less than 2 tokens, skipping", market.ID)
		return nil
	}

	token0ID := market.TokenIDs[0] // Yes/第一个结果
	token1ID := market.TokenIDs[1] // No/第二个结果

	// 获取深度
	depth, err := GetDepth(event.ID, market.ID)
	if err != nil {
		return fmt.Errorf("get depth failed: %v", err)
	}

	// 检查 token_1 的买1价
	var price decimal.Decimal
	token1Depth, ok := depth.Data.Depths[token1ID]
	if ok && len(token1Depth.Bids) > 0 {
		// 有买1价，使用该价格
		var err error
		price, err = decimal.NewFromString(token1Depth.Bids[0].Price)
		if err != nil {
			log.Printf("Failed to parse bid price: %v", err)
			price = decimal.NewFromFloat(0.3 + rand.Float64()*0.2) // 0.3-0.5
		}
	} else {
		// 没有买1价，随机生成 0.3-0.5
		price = decimal.NewFromFloat(0.3 + rand.Float64()*0.2).Truncate(4)
		log.Printf("No bids found for token_1, using random price: %s", price.String())
	}

	// 计算 shares: 2美金除以价格然后截断
	shares := decimal.NewFromFloat(OrderUSDC).Div(price).IntPart()
	if shares <= 0 {
		shares = 1
	}

	// 相反价格 (1 - price)
	oppositePrice := decimal.NewFromInt(1).Sub(price)

	log.Printf("Token0: %s, Token1: %s", token0ID[:20]+"...", token1ID[:20]+"...")
	log.Printf("Price: %s, Opposite: %s, Shares: %d", price.String(), oppositePrice.String(), shares)

	// 账户2 挂 token_1 买单 (先挂单)
	order2, err := CreateBuyOrder(Account2PrivateKey, Account2Address, token1ID, price, shares, event.ID, market.ID)
	if err != nil {
		return fmt.Errorf("create order2 failed: %v", err)
	}

	log.Printf("Account2 placing order on token_1 at price %s...", price.String())
	if err := PlaceOrder(Account2ApiKey, order2); err != nil {
		log.Printf("Account2 place order failed: %v", err)
	} else {
		log.Printf("Account2 order placed successfully")
	}

	// 等待 6 秒
	log.Printf("Waiting 6 seconds...")
	time.Sleep(6 * time.Second)

	// 账户1 挂 token_0 买单 (吃单)
	order1, err := CreateBuyOrder(Account1PrivateKey, Account1Address, token0ID, oppositePrice, shares, event.ID, market.ID)
	if err != nil {
		return fmt.Errorf("create order1 failed: %v", err)
	}

	log.Printf("Account1 placing order on token_0 at price %s...", oppositePrice.String())
	if err := PlaceOrder(Account1ApiKey, order1); err != nil {
		log.Printf("Account1 place order failed: %v", err)
	} else {
		log.Printf("Account1 order placed successfully")
	}

	return nil
}

// RunBot 执行一次机器人任务
func RunBot(db *sql.DB) error {
	log.Println("======= Bot execution started =======")

	// 1. 认证两个账户
	// token1, err := Authenticate(Account1FetchTokenPublicKey, Account1FetchTokenPrivateKey)
	// if err != nil {
	// 	return fmt.Errorf("account1 auth failed: %v", err)
	// }

	// token2, err := Authenticate(Account2FetchTokenPublicKey, Account2FetchTokenPrivateKey)
	// if err != nil {
	// 	return fmt.Errorf("account2 auth failed: %v", err)
	// }

	// 2. 获取活跃事件
	events, err := GetActiveEvents(db)
	if err != nil {
		return fmt.Errorf("get events failed: %v", err)
	}

	log.Printf("Found %d active events", len(events))

	// 3. 处理每个事件的每个市场
	for _, event := range events {
		for _, market := range event.Markets {
			if err := ProcessMarket(event, market); err != nil {
				log.Printf("Process market failed: %v", err)
				// 继续处理下一个市场
			}
			// 每个市场之间稍微等待一下
			time.Sleep(1 * time.Second)
		}
	}

	log.Println("======= Bot execution completed =======")
	return nil
}

func start_bot() {
	// 设置日志文件
	logFile, err := os.OpenFile("bot.log", os.O_CREATE|os.O_WRONLY|os.O_APPEND, 0644)
	if err != nil {
		log.Fatalf("Failed to open log file: %v", err)
	}
	defer logFile.Close()

	// 同时输出到文件和控制台
	multiWriter := io.MultiWriter(os.Stdout, logFile)
	log.SetOutput(multiWriter)
	log.SetFlags(log.Ldate | log.Ltime | log.Lmicroseconds | log.Lshortfile)

	log.Println("Market Making Bot starting...")

	// 连接数据库
	connStr := fmt.Sprintf("host=%s port=%d user=%s password=%s dbname=%s sslmode=require",
		DBHost, DBPort, DBUser, DBPassword, DBName)
	db, err := sql.Open("postgres", connStr)
	if err != nil {
		log.Fatalf("Failed to connect to database: %v", err)
	}
	defer db.Close()

	if err := db.Ping(); err != nil {
		log.Fatalf("Failed to ping database: %v", err)
	}
	log.Println("Connected to database")

	// 立即执行一次
	if err := RunBot(db); err != nil {
		log.Printf("Bot execution failed: %v", err)
	}

	// 定时执行
	ticker := time.NewTicker(time.Duration(IntervalMinutes) * time.Minute)
	defer ticker.Stop()

	log.Printf("Bot will run every %d minutes", IntervalMinutes)

	for range ticker.C {
		if err := RunBot(db); err != nil {
			log.Printf("Bot execution failed: %v", err)
		}
	}
}

func main() {
	//start_bot()
	//CancelAllOrders("cmjrw9b3b0330la0d1qgu0gb1")
}
