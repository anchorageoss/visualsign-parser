package main

import (
	"encoding/hex"
	"encoding/json"
	"fmt"
	"math/big"
	"strings"

	"github.com/ethereum/go-ethereum/accounts/abi"
	"github.com/ethereum/go-ethereum/common"
	"github.com/ethereum/go-ethereum/core/types"
	"github.com/ethereum/go-ethereum/rlp"
)

// ABIInfo contains ABI information for a contract address
type ABIInfo struct {
	Address common.Address `json:"address"`
	ABI     abi.ABI        `json:"-"`
	ABIJson string         `json:"abi"`
}

// TransactionDecoder handles Ethereum transaction decoding with ABI support
type TransactionDecoder struct {
	abiMap map[common.Address]ABIInfo
}

// NewTransactionDecoder creates a new decoder with the provided ABI mappings
func NewTransactionDecoder(abiInfos []ABIInfo) (*TransactionDecoder, error) {
	decoder := &TransactionDecoder{
		abiMap: make(map[common.Address]ABIInfo),
	}

	for _, info := range abiInfos {
		// Parse ABI JSON string
		parsedABI, err := abi.JSON(strings.NewReader(info.ABIJson))
		if err != nil {
			return nil, fmt.Errorf("failed to parse ABI for address %s: %w", info.Address.Hex(), err)
		}
		info.ABI = parsedABI
		decoder.abiMap[info.Address] = info
	}

	return decoder, nil
}

// DecodeRawTransaction decodes a hex-encoded Ethereum transaction
func (d *TransactionDecoder) DecodeRawTransaction(rawTxHex string) (*SignablePayload, error) {
	// Remove 0x prefix if present
	if strings.HasPrefix(rawTxHex, "0x") {
		rawTxHex = rawTxHex[2:]
	}

	// Decode hex to bytes
	txBytes, err := hex.DecodeString(rawTxHex)
	if err != nil {
		return nil, fmt.Errorf("failed to decode hex transaction: %w", err)
	}

	// Parse RLP-encoded transaction
	var tx types.Transaction
	if err := rlp.DecodeBytes(txBytes, &tx); err != nil {
		return nil, fmt.Errorf("failed to decode RLP transaction: %w", err)
	}

	// Convert to visual sign payload
	return d.transactionToVisualSign(&tx)
}

// transactionToVisualSign converts an Ethereum transaction to VisualSign format
func (d *TransactionDecoder) transactionToVisualSign(tx *types.Transaction) (*SignablePayload, error) {
	fields := []SignablePayloadField{}

	// Network field
	fields = append(fields, SignablePayloadField{
		Type:         "text_v2",
		Label:        "Network",
		FallbackText: "Ethereum",
		TextV2: &SignablePayloadFieldTextV2{
			Text: "Ethereum",
		},
	})

	// To address field
	if tx.To() != nil {
		fields = append(fields, SignablePayloadField{
			Type:         "address_v2",
			Label:        "To",
			FallbackText: tx.To().Hex(),
			AddressV2: &SignablePayloadFieldAddressV2{
				Address:    tx.To().Hex(),
				Name:       d.getAddressName(*tx.To()),
				AssetLabel: "ETH",
			},
		})
	}

	// Value field
	if tx.Value().Cmp(big.NewInt(0)) > 0 {
		ethValue := new(big.Float).Quo(new(big.Float).SetInt(tx.Value()), big.NewFloat(1e18))
		fields = append(fields, SignablePayloadField{
			Type:         "amount_v2",
			Label:        "Value",
			FallbackText: fmt.Sprintf("%s ETH", ethValue.String()),
			AmountV2: &SignablePayloadFieldAmountV2{
				Amount:       ethValue.String(),
				Abbreviation: "ETH",
			},
		})
	}

	// Gas limit field
	fields = append(fields, SignablePayloadField{
		Type:         "text_v2",
		Label:        "Gas Limit",
		FallbackText: fmt.Sprintf("%d", tx.Gas()),
		TextV2: &SignablePayloadFieldTextV2{
			Text: fmt.Sprintf("%d", tx.Gas()),
		},
	})

	// Gas price field
	if tx.GasPrice() != nil {
		gweiPrice := new(big.Float).Quo(new(big.Float).SetInt(tx.GasPrice()), big.NewFloat(1e9))
		fields = append(fields, SignablePayloadField{
			Type:         "text_v2",
			Label:        "Gas Price",
			FallbackText: fmt.Sprintf("%s Gwei", gweiPrice.String()),
			TextV2: &SignablePayloadFieldTextV2{
				Text: fmt.Sprintf("%s Gwei", gweiPrice.String()),
			},
		})
	}

	// Decode transaction data if available
	if len(tx.Data()) > 0 {
		if tx.To() != nil {
			if abiInfo, exists := d.abiMap[*tx.To()]; exists {
				// Try to decode with ABI
				decodedField, err := d.decodeCalldata(tx.Data(), abiInfo.ABI, *tx.To())
				if err == nil {
					fields = append(fields, *decodedField)
				} else {
					// Fall back to raw data
					fields = append(fields, d.createRawDataField(tx.Data()))
				}
			} else {
				// No ABI available, show raw data
				fields = append(fields, d.createRawDataField(tx.Data()))
			}
		} else {
			// Contract creation, show raw data
			fields = append(fields, SignablePayloadField{
				Type:         "text_v2",
				Label:        "Contract Creation Data",
				FallbackText: fmt.Sprintf("0x%x", tx.Data()),
				TextV2: &SignablePayloadFieldTextV2{
					Text: fmt.Sprintf("0x%x", tx.Data()),
				},
			})
		}
	}

	return &SignablePayload{
		Version:     0,
		Title:       "Ethereum Transaction",
		Fields:      fields,
		PayloadType: "transaction",
	}, nil
}

// decodeCalldata attempts to decode transaction calldata using the provided ABI
func (d *TransactionDecoder) decodeCalldata(data []byte, contractABI abi.ABI, contractAddress common.Address) (*SignablePayloadField, error) {
	if len(data) < 4 {
		return nil, fmt.Errorf("calldata too short")
	}

	// Extract method selector (first 4 bytes)
	methodID := data[:4]

	// Find matching method in ABI
	method, err := contractABI.MethodById(methodID)
	if err != nil {
		return nil, fmt.Errorf("method not found in ABI: %w", err)
	}

	// Decode method arguments
	args, err := method.Inputs.Unpack(data[4:])
	if err != nil {
		return nil, fmt.Errorf("failed to unpack method arguments: %w", err)
	}

	// Create condensed view
	condensedFields := []*AnnotatedPayloadField{}
	condensedFields = append(condensedFields, &AnnotatedPayloadField{
		SignablePayloadField: SignablePayloadField{
			Type:         "text_v2",
			Label:        "Method",
			FallbackText: method.Name,
			TextV2: &SignablePayloadFieldTextV2{
				Text: method.Name,
			},
		},
	})

	// Create expanded view with all parameters
	expandedFields := []*AnnotatedPayloadField{}
	expandedFields = append(expandedFields, &AnnotatedPayloadField{
		SignablePayloadField: SignablePayloadField{
			Type:         "text_v2",
			Label:        "Contract",
			FallbackText: contractAddress.Hex(),
			TextV2: &SignablePayloadFieldTextV2{
				Text: contractAddress.Hex(),
			},
		},
	})

	expandedFields = append(expandedFields, &AnnotatedPayloadField{
		SignablePayloadField: SignablePayloadField{
			Type:         "text_v2",
			Label:        "Method",
			FallbackText: method.Name,
			TextV2: &SignablePayloadFieldTextV2{
				Text: method.Name,
			},
		},
	})

	// Add parameters to expanded view
	for i, input := range method.Inputs {
		if i < len(args) {
			value := d.formatABIValue(args[i], input.Type)
			expandedFields = append(expandedFields, &AnnotatedPayloadField{
				SignablePayloadField: SignablePayloadField{
					Type:         "text_v2",
					Label:        input.Name,
					FallbackText: value,
					TextV2: &SignablePayloadFieldTextV2{
						Text: value,
					},
				},
			})
		}
	}

	return &SignablePayloadField{
		Type:         "preview_layout",
		Label:        "Smart Contract Call",
		FallbackText: fmt.Sprintf("%s.%s(...)", contractAddress.Hex(), method.Name),
		PreviewLayout: &SignablePayloadFieldPreviewLayout{
			Title: SignablePayloadFieldTextV2{
				Text: fmt.Sprintf("%s(...)", method.Name),
			},
			Condensed: SignablePayloadFieldListLayout{
				Fields: condensedFields,
			},
			Expanded: SignablePayloadFieldListLayout{
				Fields: expandedFields,
			},
		},
	}, nil
}

// formatABIValue formats an ABI value for display
func (d *TransactionDecoder) formatABIValue(value interface{}, abiType abi.Type) string {
	switch abiType.T {
	case abi.AddressTy:
		if addr, ok := value.(common.Address); ok {
			return addr.Hex()
		}
	case abi.UintTy, abi.IntTy:
		if bigInt, ok := value.(*big.Int); ok {
			return bigInt.String()
		}
	case abi.BoolTy:
		if b, ok := value.(bool); ok {
			return fmt.Sprintf("%t", b)
		}
	case abi.StringTy:
		if s, ok := value.(string); ok {
			return s
		}
	case abi.BytesTy, abi.FixedBytesTy:
		if bytes, ok := value.([]byte); ok {
			return fmt.Sprintf("0x%x", bytes)
		}
	case abi.SliceTy, abi.ArrayTy:
		// For arrays/slices, recursively format elements
		return fmt.Sprintf("%v", value)
	}
	return fmt.Sprintf("%v", value)
}

// createRawDataField creates a field for raw transaction data
func (d *TransactionDecoder) createRawDataField(data []byte) SignablePayloadField {
	return SignablePayloadField{
		Type:         "text_v2",
		Label:        "Raw Data",
		FallbackText: fmt.Sprintf("0x%x", data),
		TextV2: &SignablePayloadFieldTextV2{
			Text: fmt.Sprintf("0x%x", data),
		},
	}
}

// getAddressName returns a friendly name for an address if known
func (d *TransactionDecoder) getAddressName(addr common.Address) string {
	if _, exists := d.abiMap[addr]; exists {
		// Could extend this to include contract names from ABI metadata
		return "Smart Contract"
	}
	return ""
}

// DecodeTransactionJSON is a convenience function that returns JSON
func (d *TransactionDecoder) DecodeTransactionJSON(rawTxHex string) (string, error) {
	payload, err := d.DecodeRawTransaction(rawTxHex)
	if err != nil {
		return "", err
	}

	jsonBytes, err := json.MarshalIndent(payload, "", "  ")
	if err != nil {
		return "", fmt.Errorf("failed to marshal JSON: %w", err)
	}

	return string(jsonBytes), nil
}
