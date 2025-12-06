package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"time"

	"github.com/redis/go-redis/v9"
)

// Redis 配置
const (
	RedisAddr       = "127.0.0.1:8889"
	RedisPassword   = "123456"
	RedisDB         = 0 // COMMON_MQ DB
	TradeSendStream = "deepsense:onchain:service:send_request"
	TradeSendKey    = "send_request"
	TradeRespStream = "deepsense:onchain:service:send_reponse"
	TradeRespKey    = "send_response"
	ConsumerGroup   = "mock_trade_responder"
	ConsumerName    = "mock_consumer_1"
)

// TradeOnchainSendRequest 发送请求
type TradeOnchainSendRequest struct {
	MatchInfo       MatchOrderInfo   `json:"match_info"`
	TradeID         string           `json:"trade_id"`
	EventID         int64            `json:"event_id"`
	MarketID        int32            `json:"market_id"`
	TakerTradeInfo  TakerTradeInfo   `json:"taker_trade_info"`
	MakerTradeInfos []MakerTradeInfo `json:"maker_trade_infos"`
}

// TradeOnchainSendResponse 发送响应（去掉 match_info，加上 tx_hash 和 success）
type TradeOnchainSendResponse struct {
	TradeID         string           `json:"trade_id"`
	EventID         int64            `json:"event_id"`
	MarketID        int32            `json:"market_id"`
	TakerTradeInfo  TakerTradeInfo   `json:"taker_trade_info"`
	MakerTradeInfos []MakerTradeInfo `json:"maker_trade_infos"`
	TxHash          string           `json:"tx_hash"`
	Success         bool             `json:"success"`
}

type MatchOrderInfo struct {
	TakerOrder         SignatureOrderMsg   `json:"taker_order"`
	MakerOrder         []SignatureOrderMsg `json:"maker_order"`
	TakerFillAmount    string              `json:"taker_fill_amount"`
	TakerReceiveAmount string              `json:"taker_receive_amount"`
	MakerFillAmount    []string            `json:"maker_fill_amount"`
}

type SignatureOrderMsg struct {
	Expiration    string `json:"expiration"`
	FeeRateBps    string `json:"fee_rate_bps"`
	Maker         string `json:"maker"`
	MakerAmount   string `json:"maker_amount"`
	Nonce         string `json:"nonce"`
	Salt          int64  `json:"salt"`
	Side          string `json:"side"`
	Signature     string `json:"signature"`
	SignatureType int32  `json:"signature_type"`
	Signer        string `json:"signer"`
	Taker         string `json:"taker"`
	TakerAmount   string `json:"taker_amount"`
	TokenID       string `json:"token_id"`
}

type TakerTradeInfo struct {
	TakerSide            string `json:"taker_side"`
	TakerUserID          int64  `json:"taker_user_id"`
	TakerUsdcAmount      string `json:"taker_usdc_amount"`
	TakerTokenAmount     string `json:"taker_token_amount"`
	TakerTokenID         string `json:"taker_token_id"`
	TakerOrderID         string `json:"taker_order_id"`
	TakerUnfreezeAmount  string `json:"taker_unfreeze_amount"`
	RealTakerUsdcAmount  string `json:"real_taker_usdc_amount"`
	RealTakerTokenAmount string `json:"real_taker_token_amount"`
	TakerPrivyUserID     string `json:"taker_privy_user_id"`
	TakerOutcomeName     string `json:"taker_outcome_name"`
}

type MakerTradeInfo struct {
	MakerSide            string `json:"maker_side"`
	MakerUserID          int64  `json:"maker_user_id"`
	MakerUsdcAmount      string `json:"maker_usdc_amount"`
	MakerTokenAmount     string `json:"maker_token_amount"`
	MakerTokenID         string `json:"maker_token_id"`
	MakerOrderID         string `json:"maker_order_id"`
	MakerPrice           string `json:"maker_price"`
	RealMakerUsdcAmount  string `json:"real_maker_usdc_amount"`
	RealMakerTokenAmount string `json:"real_maker_token_amount"`
	MakerPrivyUserID     string `json:"maker_privy_user_id"`
	MakerOutcomeName     string `json:"maker_outcome_name"`
}

func main() {
	ctx := context.Background()

	// 连接 Redis
	rdb := redis.NewClient(&redis.Options{
		Addr:     RedisAddr,
		Password: RedisPassword,
		DB:       RedisDB,
	})

	// 测试连接
	if err := rdb.Ping(ctx).Err(); err != nil {
		log.Fatalf("Failed to connect to Redis: %v", err)
	}
	log.Println("✅ Connected to Redis")

	// 创建消费者组（如果不存在）
	err := rdb.XGroupCreateMkStream(ctx, TradeSendStream, ConsumerGroup, "0").Err()
	if err != nil && err.Error() != "BUSYGROUP Consumer Group name already exists" {
		log.Printf("Warning: Failed to create consumer group: %v", err)
	}

	log.Printf("🚀 Trade Responder started, listening on stream: %s", TradeSendStream)

	// 消费消息
	for {
		streams, err := rdb.XReadGroup(ctx, &redis.XReadGroupArgs{
			Group:    ConsumerGroup,
			Consumer: ConsumerName,
			Streams:  []string{TradeSendStream, ">"},
			Count:    10,
			Block:    2 * time.Second,
		}).Result()

		if err != nil {
			if err == redis.Nil {
				// 没有新消息
				continue
			}
			log.Printf("Error reading from stream: %v", err)
			time.Sleep(1 * time.Second)
			continue
		}

		for _, stream := range streams {
			for _, message := range stream.Messages {
				handleMessage(ctx, rdb, message)
			}
		}
	}
}

func handleMessage(ctx context.Context, rdb *redis.Client, message redis.XMessage) {
	// 提取消息内容
	data, ok := message.Values[TradeSendKey].(string)
	if !ok {
		log.Printf("❌ Invalid message format: %v", message.Values)
		return
	}

	// 反序列化请求
	var req TradeOnchainSendRequest
	if err := json.Unmarshal([]byte(data), &req); err != nil {
		log.Printf("❌ Failed to unmarshal request: %v", err)
		return
	}

	log.Printf("📨 Received trade request: trade_id=%s, event_id=%d, market_id=%d",
		req.TradeID, req.EventID, req.MarketID)
	log.Printf("📨 Received trade full request info: %+v\n", req)

	// 构建响应（模拟成功）
	resp := TradeOnchainSendResponse{
		TradeID:         req.TradeID,
		EventID:         req.EventID,
		MarketID:        req.MarketID,
		TakerTradeInfo:  req.TakerTradeInfo,
		MakerTradeInfos: req.MakerTradeInfos,
		TxHash:          fmt.Sprintf("0x%x", time.Now().UnixNano()), // 随机生成 tx_hash
		Success:         true,
	}

	// 序列化响应
	respData, err := json.Marshal(resp)
	if err != nil {
		log.Printf("❌ Failed to marshal response: %v", err)
		return
	}

	// 推送响应到 TRADE_RESPONSE_STREAM
	_, err = rdb.XAdd(ctx, &redis.XAddArgs{
		Stream: TradeRespStream,
		Values: map[string]interface{}{
			TradeRespKey: string(respData),
		},
	}).Result()

	if err != nil {
		log.Printf("❌ Failed to send response: %v", err)
		return
	}

	log.Printf("✅ Sent trade response: trade_id=%s, tx_hash=%s, success=%v",
		resp.TradeID, resp.TxHash, resp.Success)

	// ACK 消息
	if err := rdb.XAck(ctx, TradeSendStream, ConsumerGroup, message.ID).Err(); err != nil {
		log.Printf("⚠️  Failed to ACK message: %v", err)
	}
}
