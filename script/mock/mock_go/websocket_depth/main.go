package main

import (
	"encoding/json"
	"log"
	"net/url"
	"os"
	"os/signal"
	"time"

	"github.com/gorilla/websocket"
)

// WebSocket 配置
const (
	//WSHost = "127.0.0.1:5004" // websocket_depth 服务端口
	WSHost = "predictionmarket-websocket-depth-290128242879.asia-northeast1.run.app" // websocket_depth 服务端口
)

// 订阅消息
type DepthSubscribeMessage struct {
	Action   string `json:"action"`
	EventID  int64  `json:"event_id"`
	MarketID int16  `json:"market_id"`
}

// 取消订阅消息
type DepthUnsubscribeMessage struct {
	Action   string `json:"action"`
	EventID  int64  `json:"event_id"`
	MarketID int16  `json:"market_id"`
}

func main() {
	interrupt := make(chan os.Signal, 1)
	signal.Notify(interrupt, os.Interrupt)

	u := url.URL{Scheme: "wss", Host: WSHost, Path: "/depth"}
	log.Printf("🔗 Connecting to %s", u.String())

	c, _, err := websocket.DefaultDialer.Dial(u.String(), nil)
	if err != nil {
		log.Fatal("dial:", err)
	}
	defer c.Close()

	log.Println("✅ Connected to WebSocket Depth Server")

	done := make(chan struct{})

	// 读取消息协程
	go func() {
		defer close(done)
		for {
			_, message, err := c.ReadMessage()
			if err != nil {
				log.Println("read error:", err)
				return
			}

			// 解析并美化输出
			var data interface{}
			if err := json.Unmarshal(message, &data); err == nil {
				prettyData, _ := json.MarshalIndent(data, "", "  ")
				log.Printf("📊 Received depth data:\n%s\n", string(prettyData))
			} else {
				log.Printf("📊 Received: %s", message)
			}
		}
	}()

	// 订阅深度数据
	subscribe := DepthSubscribeMessage{
		Action:   "subscribe",
		EventID:  1,
		MarketID: 1,
	}

	subscribeData, _ := json.Marshal(subscribe)
	if err := c.WriteMessage(websocket.TextMessage, subscribeData); err != nil {
		log.Println("write error:", err)
		return
	}
	log.Printf("📨 Subscribed to depth: event_id=1, market_id=1")

	// 等待中断信号
	ticker := time.NewTicker(20 * time.Second)
	defer ticker.Stop()

	for {
		select {
		case <-done:
			return
		case <-ticker.C:
			// 定期发送心跳 - 发送文本消息"ping"
			if err := c.WriteMessage(websocket.TextMessage, []byte("ping")); err != nil {
				log.Println("ping error:", err)
				return
			}
		case <-interrupt:
			log.Println("🛑 Interrupt received, closing connection...")

			// 取消订阅
			unsubscribe := DepthUnsubscribeMessage{
				Action:   "unsubscribe",
				EventID:  1,
				MarketID: 1,
			}
			unsubscribeData, _ := json.Marshal(unsubscribe)
			c.WriteMessage(websocket.TextMessage, unsubscribeData)

			// 正常关闭连接
			err := c.WriteMessage(websocket.CloseMessage, websocket.FormatCloseMessage(websocket.CloseNormalClosure, ""))
			if err != nil {
				log.Println("write close:", err)
				return
			}
			select {
			case <-done:
			case <-time.After(time.Second):
			}
			return
		}
	}
}
