package main

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"sort"
	"strings"
	"time"

	"github.com/jackc/pgx/v4/pgxpool"
	"github.com/redis/go-redis/v9"
)

const (
	// Redis Stream 配置
	EVENT_INPUT_STREAM  = "event_input_stream"
	EVENT_INPUT_MSG_KEY = "event_input_key"

	// // PostgreSQL 配置
	// POSTGRES_HOST     = "127.0.0.1"
	// POSTGRES_PORT     = 5432
	// POSTGRES_USER     = "postgres"
	// POSTGRES_PASSWORD = "123456"
	// POSTGRES_DATABASE = "prediction_market"

	// // Redis 配置
	// REDIS_HOST     = "127.0.0.1:8889"
	// REDIS_PASSWORD = "123456"
	// REDIS_DB       = 6 // engine_input_mq 使用 DB 6
	// PostgreSQL 配置
	POSTGRES_HOST     = "34.146.110.159"
	POSTGRES_PORT     = 5432
	POSTGRES_USER     = "postgres"
	POSTGRES_PASSWORD = "0gZUDGsz1sFy0avm2VHd!"
	POSTGRES_DATABASE = "deepsense"

	// Redis 配置
	REDIS_HOST     = "35.200.1.149:6379"
	REDIS_PASSWORD = "mZDUu0M43KmvMo1ehuiz"
	REDIS_DB       = 0 // engine_input_mq 使用 DB 0
)

// Event 数据库表结构
type Event struct {
	ID              int64                  `json:"id"`
	EventIdentifier string                 `json:"event_identifier"`
	Slug            string                 `json:"slug"`
	Title           string                 `json:"title"`
	Description     string                 `json:"description"`
	Image           string                 `json:"image"`
	EndDate         *time.Time             `json:"end_date"`
	Topic           string                 `json:"topic"`
	Markets         map[string]EventMarket `json:"markets"`
	Closed          bool                   `json:"closed"`
	CreatedAt       time.Time              `json:"created_at"`
}

// EventMarket 数据库市场结构
type EventMarket struct {
	ParentCollectionID string   `json:"parent_collection_id"`
	ConditionID        string   `json:"condition_id"`
	ID                 int16    `json:"id"`
	MarketIdentifier   string   `json:"market_identifier"`
	Question           string   `json:"question"`
	Slug               string   `json:"slug"`
	Title              string   `json:"title"`
	Image              string   `json:"image"`
	Outcomes           []string `json:"outcomes"`
	TokenIDs           []string `json:"token_ids"`
}

// EngineMQEventCreate 用于发送给 match_engine
type EngineMQEventCreate struct {
	EventID int64                          `json:"event_id"`
	Markets map[string]EngineMQEventMarket `json:"markets"`
	EndDate *time.Time                     `json:"end_date,omitempty"`
}

// EngineMQEventMarket match_engine 需要的市场结构
type EngineMQEventMarket struct {
	MarketID int16    `json:"market_id"`
	Outcomes []string `json:"outcomes"`
	TokenIDs []string `json:"token_ids"`
}

// EventInputMessageCreate 用于 AddOneEvent
type EventInputMessageCreate struct {
	Types   string                         `json:"types"`
	EventID int64                          `json:"event_id"`
	Markets map[string]EngineMQEventMarket `json:"markets"`
	EndDate *time.Time                     `json:"end_date,omitempty"`
}

// EventInputMessageClose 用于 RemoveOneEvent
type EventInputMessageClose struct {
	Types   string `json:"types"`
	EventID int64  `json:"event_id"`
}

// sortOutcomesAndTokenIDs 排序 outcomes 和 token_ids
// 如果是 Yes/No，Yes 在前，No 在后；否则按字典序排序
func sortOutcomesAndTokenIDs(outcomes []string, tokenIDs []string) ([]string, []string) {
	if len(outcomes) != len(tokenIDs) {
		log.Printf("Warning: outcomes and tokenIDs length mismatch")
		return outcomes, tokenIDs
	}

	// 创建索引配对
	type pair struct {
		index   int
		outcome string
		tokenID string
	}
	pairs := make([]pair, len(outcomes))
	for i := range outcomes {
		pairs[i] = pair{index: i, outcome: outcomes[i], tokenID: tokenIDs[i]}
	}

	// 检查是否是 Yes/No
	isYesNo := len(pairs) == 2
	hasYes := false
	hasNo := false
	for _, p := range pairs {
		if strings.EqualFold(p.outcome, "Yes") {
			hasYes = true
		}
		if strings.EqualFold(p.outcome, "No") {
			hasNo = true
		}
	}
	isYesNo = isYesNo && hasYes && hasNo

	if isYesNo {
		// Yes 在前，No 在后
		sort.Slice(pairs, func(i, j int) bool {
			iIsYes := strings.EqualFold(pairs[i].outcome, "Yes")
			jIsYes := strings.EqualFold(pairs[j].outcome, "Yes")
			if iIsYes && !jIsYes {
				return true
			}
			if !iIsYes && jIsYes {
				return false
			}
			return pairs[i].outcome < pairs[j].outcome
		})
	} else {
		// 按字典序排序
		sort.Slice(pairs, func(i, j int) bool {
			return pairs[i].outcome < pairs[j].outcome
		})
	}

	// 提取排序后的结果
	sortedOutcomes := make([]string, len(pairs))
	sortedTokenIDs := make([]string, len(pairs))
	for i, p := range pairs {
		sortedOutcomes[i] = p.outcome
		sortedTokenIDs[i] = p.tokenID
	}

	return sortedOutcomes, sortedTokenIDs
}

// sendEventCreate 发送未关闭且未过期的事件创建消息到 match_engine
func sendEventCreate(ctx context.Context, pgPool *pgxpool.Pool, rdb *redis.Client) error {
	// 查询 closed=false 且未过期的事件
	query := `SELECT id, event_identifier, slug, title, description, image, end_date, topic, markets, closed, created_at
	          FROM events WHERE closed = false AND (end_date IS NULL OR end_date > NOW()) ORDER BY id`
	rows, err := pgPool.Query(ctx, query)
	if err != nil {
		return fmt.Errorf("failed to query events: %w", err)
	}
	defer rows.Close()

	eventCount := 0
	for rows.Next() {
		var event Event
		var marketsJSON []byte

		err := rows.Scan(
			&event.ID,
			&event.EventIdentifier,
			&event.Slug,
			&event.Title,
			&event.Description,
			&event.Image,
			&event.EndDate,
			&event.Topic,
			&marketsJSON,
			&event.Closed,
			&event.CreatedAt,
		)
		if err != nil {
			log.Printf("Failed to scan event: %v", err)
			continue
		}

		// 解析 markets JSON
		if err := json.Unmarshal(marketsJSON, &event.Markets); err != nil {
			log.Printf("Failed to unmarshal markets for event %d: %v", event.ID, err)
			continue
		}

		// 构建 EngineMQEventCreate
		engineMarkets := make(map[string]EngineMQEventMarket)
		for marketIDStr, market := range event.Markets {
			// 排序 outcomes 和 token_ids
			sortedOutcomes, sortedTokenIDs := sortOutcomesAndTokenIDs(market.Outcomes, market.TokenIDs)

			engineMarket := EngineMQEventMarket{
				MarketID: market.ID,
				Outcomes: sortedOutcomes,
				TokenIDs: sortedTokenIDs,
			}
			engineMarkets[marketIDStr] = engineMarket
		}

		// 构建 EventInputMessageCreate (展平结构)
		eventMsg := EventInputMessageCreate{
			Types:   "AddOneEvent",
			EventID: event.ID,
			Markets: engineMarkets,
			EndDate: event.EndDate,
		}

		// 序列化为 JSON
		msgBytes, err := json.Marshal(eventMsg)
		if err != nil {
			log.Printf("Failed to marshal event message for event %d: %v", event.ID, err)
			continue
		}

		// 发送到 Redis Stream
		err = rdb.XAdd(ctx, &redis.XAddArgs{
			Stream: EVENT_INPUT_STREAM,
			Values: map[string]interface{}{
				EVENT_INPUT_MSG_KEY: string(msgBytes),
			},
		}).Err()

		if err != nil {
			log.Printf("Failed to publish event %d to Redis: %v", event.ID, err)
			continue
		}

		eventCount++
		log.Printf("Published AddOneEvent: event_id=%d (%s)", event.ID, event.EventIdentifier)
	}

	if err := rows.Err(); err != nil {
		return fmt.Errorf("error iterating events: %w", err)
	}

	log.Printf("✅ Successfully published %d AddOneEvent messages to match_engine", eventCount)
	return nil
}

// sendEventClose 发送指定事件的关闭消息到 match_engine
func sendEventClose(ctx context.Context, rdb *redis.Client, eventID int64) error {
	// 构建 EventInputMessageClose (展平结构)
	eventMsg := EventInputMessageClose{
		Types:   "RemoveOneEvent",
		EventID: eventID,
	}

	// 序列化为 JSON
	msgBytes, err := json.Marshal(eventMsg)
	if err != nil {
		return fmt.Errorf("failed to marshal close message for event %d: %w", eventID, err)
	}

	// 打印 JSON 用于调试
	log.Printf("RemoveOneEvent JSON: %s", string(msgBytes))

	// 发送到 Redis Stream
	err = rdb.XAdd(ctx, &redis.XAddArgs{
		Stream: EVENT_INPUT_STREAM,
		Values: map[string]interface{}{
			EVENT_INPUT_MSG_KEY: string(msgBytes),
		},
	}).Err()

	if err != nil {
		return fmt.Errorf("failed to publish close event %d to Redis: %w", eventID, err)
	}

	log.Printf("✅ Published RemoveOneEvent: event_id=%d", eventID)
	return nil
}

func main() {
	ctx := context.Background()

	// 连接 PostgreSQL
	//dsn := fmt.Sprintf("postgres://%s:%s@%s:%d/%s",
	dsn := fmt.Sprintf("postgres://%s:%s@%s:%d/%s?sslmode=require",
		POSTGRES_USER, POSTGRES_PASSWORD, POSTGRES_HOST, POSTGRES_PORT, POSTGRES_DATABASE)
	pgPool, err := pgxpool.Connect(ctx, dsn)
	if err != nil {
		log.Fatalf("Failed to connect to PostgreSQL: %v", err)
	}
	defer pgPool.Close()
	log.Println("Connected to PostgreSQL")

	// 连接 Redis
	rdb := redis.NewClient(&redis.Options{
		Addr:     REDIS_HOST,
		Password: REDIS_PASSWORD,
		DB:       REDIS_DB,
	})
	defer rdb.Close()

	// 测试 Redis 连接
	// if err := rdb.Ping(ctx).Err(); err != nil {
	// 	log.Fatalf("Failed to connect to Redis: %v", err)
	// }
	// log.Println("Connected to Redis")

	// 发送事件创建消息
	log.Println("\n=== Sending Event Create Messages ===")
	if err := sendEventCreate(ctx, pgPool, rdb); err != nil {
		log.Fatalf("Failed to send event create messages: %v", err)
	}

	log.Println("\n✅ Event create messages sent successfully")
	// log.Println("\n💡 To close an event, call: sendEventClose(ctx, rdb, event_id)")
	// sendEventClose(ctx, rdb, 1)
	// if err != nil {
	// 	log.Fatalf("Failed to send event close messages: %v", err)
	// }
	// log.Println("Event close messages sent successfully")
	// log.Println("\n✅ All messages sent successfully")
}
