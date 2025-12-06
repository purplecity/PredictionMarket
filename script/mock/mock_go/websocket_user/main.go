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
	//WSHost = "127.0.0.1:5005" // websocket_user 服务端口
	WSHost = "predictionmarket-websocket-user-290128242879.asia-northeast1.run.app" // websocket_user 服务端口

	// Privy JWT Token - 替换为你的实际token用于鉴权
	PrivyToken = "YOUR_PRIVY_JWT_TOKEN_HERE"
)

// 鉴权消息
type AuthMessage struct {
	Auth string `json:"auth"`
}

func main() {
	interrupt := make(chan os.Signal, 1)
	signal.Notify(interrupt, os.Interrupt)

	// u := url.URL{Scheme: "ws", Host: WSHost, Path: "/user"}
	u := url.URL{Scheme: "wss", Host: WSHost, Path: "/user"}
	log.Printf("🔗 Connecting to %s", u.String())

	c, _, err := websocket.DefaultDialer.Dial(u.String(), nil)
	if err != nil {
		log.Fatal("dial:", err)
	}
	defer c.Close()

	log.Println("✅ Connected to WebSocket User Server")

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
				log.Printf("👤 Received user data:\n%s\n", string(prettyData))
			} else {
				log.Printf("👤 Received: %s", message)
			}
		}
	}()

	// 先发送鉴权消息
	authMsg := AuthMessage{
		Auth: PrivyToken,
	}

	authData, _ := json.Marshal(authMsg)
	if err := c.WriteMessage(websocket.TextMessage, authData); err != nil {
		log.Println("auth write error:", err)
		return
	}
	log.Printf("🔐 Sent authentication with Privy token")

	// 等待中断信号
	ticker := time.NewTicker(30 * time.Second)
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
