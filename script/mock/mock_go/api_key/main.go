package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"

	"github.com/redis/go-redis/v9"
)

const (
	// Redis Stream 配置
	API_KEY_STREAM  = "api_key_stream"
	API_KEY_MSG_KEY = "api_key_key"

	// Redis 配置 (common_mq)
	REDIS_HOST     = "35.200.1.149:6379"
	REDIS_PASSWORD = "mZDUu0M43KmvMo1ehuiz"
	REDIS_DB       = 0 // engine_input_mq 使用 DB 6
)

// ApiKeyEventAdd 添加 API Key 事件
type ApiKeyEventAdd struct {
	Action  string `json:"action"`
	ApiKey  string `json:"api_key"`
	PrivyID string `json:"privy_id"`
}

// ApiKeyEventRemove 移除 API Key 事件
type ApiKeyEventRemove struct {
	Action string `json:"action"`
	ApiKey string `json:"api_key"`
}

// sendAddApiKey 发送添加 API Key 消息
func sendAddApiKey(ctx context.Context, rdb *redis.Client, apiKey, privyID string) error {
	event := ApiKeyEventAdd{
		Action:  "add",
		ApiKey:  apiKey,
		PrivyID: privyID,
	}

	msgBytes, err := json.Marshal(event)
	if err != nil {
		return fmt.Errorf("failed to marshal add event: %w", err)
	}

	log.Printf("Sending Add API Key: %s", string(msgBytes))

	err = rdb.XAdd(ctx, &redis.XAddArgs{
		Stream: API_KEY_STREAM,
		Values: map[string]interface{}{
			API_KEY_MSG_KEY: string(msgBytes),
		},
	}).Err()

	if err != nil {
		return fmt.Errorf("failed to publish add event: %w", err)
	}

	log.Printf("Successfully added API Key: %s -> %s", apiKey, privyID)
	return nil
}

// sendRemoveApiKey 发送移除 API Key 消息
func sendRemoveApiKey(ctx context.Context, rdb *redis.Client, apiKey string) error {
	event := ApiKeyEventRemove{
		Action: "remove",
		ApiKey: apiKey,
	}

	msgBytes, err := json.Marshal(event)
	if err != nil {
		return fmt.Errorf("failed to marshal remove event: %w", err)
	}

	log.Printf("Sending Remove API Key: %s", string(msgBytes))

	err = rdb.XAdd(ctx, &redis.XAddArgs{
		Stream: API_KEY_STREAM,
		Values: map[string]interface{}{
			API_KEY_MSG_KEY: string(msgBytes),
		},
	}).Err()

	if err != nil {
		return fmt.Errorf("failed to publish remove event: %w", err)
	}

	log.Printf("Successfully removed API Key: %s", apiKey)
	return nil
}

func printUsage() {
	fmt.Println("Usage:")
	fmt.Println("  go run main.go add <api_key> <privy_id>  - Add an API Key")
	fmt.Println("  go run main.go remove <api_key>         - Remove an API Key")
	fmt.Println()
	fmt.Println("Examples:")
	fmt.Println("  go run main.go add my-api-key-123 did:privy:abc123")
	fmt.Println("  go run main.go remove my-api-key-123")
}

func main() {
	if len(os.Args) < 2 {
		printUsage()
		os.Exit(1)
	}

	action := os.Args[1]

	ctx := context.Background()

	// 连接 Redis
	rdb := redis.NewClient(&redis.Options{
		Addr:     REDIS_HOST,
		Password: REDIS_PASSWORD,
		DB:       REDIS_DB,
	})
	defer rdb.Close()

	// 测试 Redis 连接
	if err := rdb.Ping(ctx).Err(); err != nil {
		log.Fatalf("Failed to connect to Redis: %v", err)
	}
	log.Println("Connected to Redis")

	switch action {
	case "add":
		if len(os.Args) < 4 {
			fmt.Println("Error: 'add' requires <api_key> and <privy_id>")
			printUsage()
			os.Exit(1)
		}
		apiKey := os.Args[2]
		privyID := os.Args[3]
		if err := sendAddApiKey(ctx, rdb, apiKey, privyID); err != nil {
			log.Fatalf("Failed to add API Key: %v", err)
		}

	case "remove":
		if len(os.Args) < 3 {
			fmt.Println("Error: 'remove' requires <api_key>")
			printUsage()
			os.Exit(1)
		}
		apiKey := os.Args[2]
		if err := sendRemoveApiKey(ctx, rdb, apiKey); err != nil {
			log.Fatalf("Failed to remove API Key: %v", err)
		}

	default:
		fmt.Printf("Error: Unknown action '%s'\n", action)
		printUsage()
		os.Exit(1)
	}
}
