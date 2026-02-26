package eip712

import (
	"fmt"
	"math/big"

	"github.com/ethereum/go-ethereum/common"
)

// 合约地址 (EVM 主网/测试网，改值即可兼容不同链)
const (
	EVMCTFExchangeAddress        = "0x65a2085833D2658f2B0ee2216F50A6CD2CE99C93"
	EVMTestnetCTFExchangeAddress = "0x65a2085833D2658f2B0ee2216F50A6CD2CE99C93"
	EVMChainID                   = 56
	EVMTestnetChainID            = 97
)

// Order represents a prediction market order
type Order struct {
	Salt          *big.Int
	Maker         common.Address
	Signer        common.Address
	Taker         common.Address
	TokenId       *big.Int
	MakerAmount   *big.Int
	TakerAmount   *big.Int
	Expiration    *big.Int
	Nonce         *big.Int
	FeeRateBps    *big.Int
	Side          uint8
	SignatureType uint8
}

// OrderInput represents the string-based order input
type OrderInput struct {
	Salt          string
	Maker         string
	Signer        string
	Taker         string
	TokenId       string
	MakerAmount   string
	TakerAmount   string
	Expiration    string
	Nonce         string
	FeeRateBps    string
	Side          int
	SignatureType int
}

// CTFExchangeDomain returns the EIP-712 domain for CTF Exchange
func CTFExchangeDomain(chainID int64, verifyingContract common.Address) Domain {
	return Domain{
		Name:              "Sidekick Predict CTF Exchange",
		Version:           "1",
		ChainID:           big.NewInt(chainID),
		VerifyingContract: verifyingContract,
	}
}

// OrderTypes returns the EIP-712 type definition for Order
func OrderTypes() map[string][]Type {
	return map[string][]Type{
		"Order": {
			{Name: "salt", Type: "uint256"},
			{Name: "maker", Type: "address"},
			{Name: "signer", Type: "address"},
			{Name: "taker", Type: "address"},
			{Name: "tokenId", Type: "uint256"},
			{Name: "makerAmount", Type: "uint256"},
			{Name: "takerAmount", Type: "uint256"},
			{Name: "expiration", Type: "uint256"},
			{Name: "nonce", Type: "uint256"},
			{Name: "feeRateBps", Type: "uint256"},
			{Name: "side", Type: "uint8"},
			{Name: "signatureType", Type: "uint8"},
		},
	}
}

// OrderToMessage converts Order to EIP-712 Message
func OrderToMessage(order *Order) Message {
	return Message{
		"salt":          order.Salt.String(),
		"maker":         order.Maker.Hex(),
		"signer":        order.Signer.Hex(),
		"taker":         order.Taker.Hex(),
		"tokenId":       order.TokenId.String(),
		"makerAmount":   order.MakerAmount.String(),
		"takerAmount":   order.TakerAmount.String(),
		"expiration":    order.Expiration.String(),
		"nonce":         order.Nonce.String(),
		"feeRateBps":    order.FeeRateBps.String(),
		"side":          fmt.Sprintf("%d", order.Side),
		"signatureType": fmt.Sprintf("%d", order.SignatureType),
	}
}

// OrderInputToOrder converts OrderInput to Order
func OrderInputToOrder(input *OrderInput) (*Order, error) {
	order := &Order{}

	// Parse Salt
	salt, ok := new(big.Int).SetString(input.Salt, 10)
	if !ok {
		return nil, fmt.Errorf("invalid salt: %s", input.Salt)
	}
	order.Salt = salt

	// Parse addresses
	order.Maker = common.HexToAddress(input.Maker)
	order.Signer = common.HexToAddress(input.Signer)
	order.Taker = common.HexToAddress(input.Taker)

	// Parse TokenId
	tokenId, ok := new(big.Int).SetString(input.TokenId, 10)
	if !ok {
		return nil, fmt.Errorf("invalid tokenId: %s", input.TokenId)
	}
	order.TokenId = tokenId

	// Parse MakerAmount
	makerAmount, ok := new(big.Int).SetString(input.MakerAmount, 10)
	if !ok {
		return nil, fmt.Errorf("invalid makerAmount: %s", input.MakerAmount)
	}
	order.MakerAmount = makerAmount

	// Parse TakerAmount
	takerAmount, ok := new(big.Int).SetString(input.TakerAmount, 10)
	if !ok {
		return nil, fmt.Errorf("invalid takerAmount: %s", input.TakerAmount)
	}
	order.TakerAmount = takerAmount

	// Parse Expiration
	expiration, ok := new(big.Int).SetString(input.Expiration, 10)
	if !ok {
		return nil, fmt.Errorf("invalid expiration: %s", input.Expiration)
	}
	order.Expiration = expiration

	// Parse Nonce
	nonce, ok := new(big.Int).SetString(input.Nonce, 10)
	if !ok {
		return nil, fmt.Errorf("invalid nonce: %s", input.Nonce)
	}
	order.Nonce = nonce

	// Parse FeeRateBps
	feeRateBps, ok := new(big.Int).SetString(input.FeeRateBps, 10)
	if !ok {
		return nil, fmt.Errorf("invalid feeRateBps: %s", input.FeeRateBps)
	}
	order.FeeRateBps = feeRateBps

	// Parse Side
	if input.Side < 0 || input.Side > 255 {
		return nil, fmt.Errorf("invalid side: %d", input.Side)
	}
	order.Side = uint8(input.Side)

	// Parse SignatureType
	if input.SignatureType < 0 || input.SignatureType > 255 {
		return nil, fmt.Errorf("invalid signatureType: %d", input.SignatureType)
	}
	order.SignatureType = uint8(input.SignatureType)

	return order, nil
}

// GetCTFExchangeAddress returns the CTF Exchange address for the given chain ID
func GetCTFExchangeAddress(chainID int) (common.Address, error) {
	switch chainID {
	case EVMChainID:
		return common.HexToAddress(EVMCTFExchangeAddress), nil
	case EVMTestnetChainID:
		return common.HexToAddress(EVMTestnetCTFExchangeAddress), nil
	default:
		return common.Address{}, fmt.Errorf("unsupported chain_id: %d", chainID)
	}
}

// SignOrder signs a prediction market order
func SignOrder(privateKeyHex string, chainID int64, verifyingContract common.Address, order *Order) (*Signature, error) {
	signer, err := NewSigner(privateKeyHex, chainID)
	if err != nil {
		return nil, fmt.Errorf("failed to create signer: %w", err)
	}

	domain := CTFExchangeDomain(chainID, verifyingContract)
	types := OrderTypes()
	message := OrderToMessage(order)

	return signer.SignTypedData(domain, types, "Order", message)
}

// SignOrderInput is a convenience function that takes OrderInput and returns the signature
func SignOrderInput(privateKeyHex string, chainID int, input *OrderInput) (string, error) {
	// Get verifying contract address
	verifyingContract, err := GetCTFExchangeAddress(chainID)
	if err != nil {
		return "", err
	}

	// Convert input to order
	order, err := OrderInputToOrder(input)
	if err != nil {
		return "", err
	}

	// Sign order
	signature, err := SignOrder(privateKeyHex, int64(chainID), verifyingContract, order)
	if err != nil {
		return "", err
	}

	return signature.Bytes, nil
}
