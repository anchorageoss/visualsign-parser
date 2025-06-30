package main

import (
	"fmt"
	"os"

	"github.com/ethereum/go-ethereum/common"
)

func main() {
	if len(os.Args) < 2 {
		fmt.Println("Usage: go run . <hex_transaction>")
		fmt.Println("Example: go run . 0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a764000080258080")
		os.Exit(1)
	}

	rawTx := os.Args[1]
	fmt.Printf("Input transaction: %s\n", rawTx)
	fmt.Printf("Length: %d\n", len(rawTx))

	// For demo purposes, create decoder with empty ABI map
	// In production, you would load actual ABI mappings
	decoder, err := NewTransactionDecoder([]ABIInfo{})
	if err != nil {
		fmt.Printf("Failed to create decoder: %v\n", err)
		os.Exit(1)
	}

	// Decode transaction
	result, err := decoder.DecodeTransactionJSON(rawTx)
	if err != nil {
		fmt.Printf("Failed to decode transaction: %v\n", err)
		os.Exit(1)
	}

	fmt.Println(result)
}

// Example usage with ABI:
func exampleWithABI() {
	// Example ERC20 ABI snippet for transfer method
	erc20ABI := `[
		{
			"name": "transfer",
			"type": "function",
			"inputs": [
				{"name": "_to", "type": "address"},
				{"name": "_value", "type": "uint256"}
			],
			"outputs": [
				{"name": "", "type": "bool"}
			]
		}
	]`

	abiInfos := []ABIInfo{
		{
			// Example USDC contract address
			Address: common.HexToAddress("0xA0b86a33E6441e2C00A0C42c6b9a6F8de91c2321"),
			ABIJson: erc20ABI,
		},
	}

	decoder, err := NewTransactionDecoder(abiInfos)
	if err != nil {
		fmt.Printf("Failed to create decoder: %v\n", err)
		return
	}

	// Example transaction hex would go here
	_ = decoder
}
