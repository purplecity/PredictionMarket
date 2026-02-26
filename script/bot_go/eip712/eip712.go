// Package eip712 provides EIP-712 typed data signing for prediction market orders.
package eip712

import (
	"crypto/ecdsa"
	"errors"
	"fmt"
	"math/big"
	"sort"
	"strings"

	"github.com/ethereum/go-ethereum/common"
	"github.com/ethereum/go-ethereum/common/hexutil"
	"github.com/ethereum/go-ethereum/common/math"
	"github.com/ethereum/go-ethereum/crypto"
	"github.com/ethereum/go-ethereum/signer/core/apitypes"
)

// Signer provides a simple interface for EIP-712 signing
type Signer struct {
	privateKey *ecdsa.PrivateKey
	address    common.Address
	chainID    *big.Int
}

// NewSigner creates a new EIP-712 signer from a private key
func NewSigner(privateKeyHex string, chainID int64) (*Signer, error) {
	// Remove 0x prefix if present
	privateKeyHex = strings.TrimPrefix(privateKeyHex, "0x")

	privateKey, err := crypto.HexToECDSA(privateKeyHex)
	if err != nil {
		return nil, fmt.Errorf("invalid private key: %w", err)
	}

	publicKey := privateKey.Public()
	publicKeyECDSA, ok := publicKey.(*ecdsa.PublicKey)
	if !ok {
		return nil, errors.New("error casting public key to ECDSA")
	}

	address := crypto.PubkeyToAddress(*publicKeyECDSA)

	return &Signer{
		privateKey: privateKey,
		address:    address,
		chainID:    big.NewInt(chainID),
	}, nil
}

// Address returns the signer's Ethereum address
func (s *Signer) Address() common.Address {
	return s.address
}

// Domain represents the EIP-712 domain separator
type Domain struct {
	Name              string         `json:"name"`
	Version           string         `json:"version"`
	ChainID           *big.Int       `json:"chainId,omitempty"`
	VerifyingContract common.Address `json:"verifyingContract,omitempty"`
	Salt              [32]byte       `json:"salt,omitempty"`
}

// Message represents a simple wrapper for EIP-712 messages
type Message map[string]any

// Type represents an EIP-712 type field
type Type struct {
	Name string `json:"name"`
	Type string `json:"type"`
}

// Signature contains the signature components
type Signature struct {
	R     string `json:"r"`
	S     string `json:"s"`
	V     uint8  `json:"v"`
	Hash  string `json:"hash"`
	Bytes string `json:"signature"`
}

// SignTypedData signs an EIP-712 typed data message
func (s *Signer) SignTypedData(domain Domain, types map[string][]Type, primaryType string, message Message) (*Signature, error) {
	// Validate for cyclic structures
	if err := validateNoCycles(types); err != nil {
		return nil, err
	}

	// Convert to apitypes format
	typedData := apitypes.TypedData{
		Types:       make(apitypes.Types),
		PrimaryType: primaryType,
		Domain:      s.domainToAPITypes(domain),
		Message:     apitypes.TypedDataMessage(message),
	}

	// Convert types
	for typeName, fields := range types {
		typedData.Types[typeName] = make([]apitypes.Type, len(fields))
		for i, field := range fields {
			typedData.Types[typeName][i] = apitypes.Type{
				Name: field.Name,
				Type: field.Type,
			}
		}
	}

	// Add EIP712Domain type if not present
	if _, ok := typedData.Types["EIP712Domain"]; !ok {
		typedData.Types["EIP712Domain"] = s.buildDomainTypes(domain)
	}

	// Hash the typed data
	hash, _, err := apitypes.TypedDataAndHash(typedData)
	if err != nil {
		return nil, fmt.Errorf("failed to hash typed data: %w", err)
	}

	// Sign the hash
	signature, err := crypto.Sign(hash, s.privateKey)
	if err != nil {
		return nil, fmt.Errorf("failed to sign: %w", err)
	}

	// Transform V from 0/1 to 27/28 per Ethereum convention
	signature[64] += 27

	return &Signature{
		R:     hexutil.Encode(signature[:32]),
		S:     hexutil.Encode(signature[32:64]),
		V:     uint8(signature[64]),
		Hash:  hexutil.Encode(hash),
		Bytes: hexutil.Encode(signature),
	}, nil
}

func (s *Signer) domainToAPITypes(domain Domain) apitypes.TypedDataDomain {
	d := apitypes.TypedDataDomain{
		Name:    domain.Name,
		Version: domain.Version,
	}

	if domain.ChainID != nil {
		d.ChainId = (*math.HexOrDecimal256)(domain.ChainID)
	}

	if domain.VerifyingContract != (common.Address{}) {
		d.VerifyingContract = domain.VerifyingContract.Hex()
	}

	if domain.Salt != [32]byte{} {
		d.Salt = hexutil.Encode(domain.Salt[:])
	}

	return d
}

func (s *Signer) buildDomainTypes(domain Domain) []apitypes.Type {
	types := []apitypes.Type{
		{Name: "name", Type: "string"},
		{Name: "version", Type: "string"},
	}

	if domain.ChainID != nil {
		types = append(types, apitypes.Type{Name: "chainId", Type: "uint256"})
	}

	if domain.VerifyingContract != (common.Address{}) {
		types = append(types, apitypes.Type{Name: "verifyingContract", Type: "address"})
	}

	if domain.Salt != [32]byte{} {
		types = append(types, apitypes.Type{Name: "salt", Type: "bytes32"})
	}

	return types
}

// inferTypes attempts to infer EIP-712 types from a message
func inferTypes(message map[string]any) []Type {
	types := make([]Type, 0, len(message))

	for name, value := range message {
		var fieldType string

		switch v := value.(type) {
		case string:
			if common.IsHexAddress(v) {
				fieldType = "address"
			} else if _, ok := new(big.Int).SetString(v, 10); ok {
				fieldType = "uint256"
			} else {
				fieldType = "string"
			}
		case *big.Int:
			fieldType = "uint256"
		case int, int8, int16, int32, int64:
			fieldType = "uint256"
		case uint, uint8, uint16, uint32, uint64:
			fieldType = "uint256"
		case bool:
			fieldType = "bool"
		case []byte:
			fieldType = fmt.Sprintf("bytes%d", len(v))
		default:
			fieldType = "string"
		}

		types = append(types, Type{
			Name: name,
			Type: fieldType,
		})
	}

	sort.Slice(types, func(i, j int) bool {
		return types[i].Name < types[j].Name
	})

	return types
}

// validateNoCycles checks for cyclic references in type definitions
func validateNoCycles(types map[string][]Type) error {
	visited := make(map[string]bool)
	inPath := make(map[string]bool)

	for typeName := range types {
		if err := checkCycle(typeName, types, visited, inPath); err != nil {
			return err
		}
	}

	return nil
}

// checkCycle performs DFS to detect cycles in type definitions
func checkCycle(typeName string, types map[string][]Type, visited, inPath map[string]bool) error {
	if inPath[typeName] {
		return fmt.Errorf("cyclic reference detected in type: %s", typeName)
	}

	if visited[typeName] {
		return nil
	}

	visited[typeName] = true
	inPath[typeName] = true

	if fields, ok := types[typeName]; ok {
		for _, field := range fields {
			baseType := strings.TrimSuffix(field.Type, "[]")
			if _, isCustom := types[baseType]; isCustom {
				if err := checkCycle(baseType, types, visited, inPath); err != nil {
					return err
				}
			}
		}
	}

	inPath[typeName] = false
	return nil
}
