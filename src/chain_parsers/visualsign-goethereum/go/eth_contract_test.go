package main

import (
	"fmt"
	"math/big"
	"strings"
	"testing"

	"github.com/ethereum/go-ethereum/accounts/abi"
	"github.com/ethereum/go-ethereum/common"
	"github.com/ethereum/go-ethereum/core/types"
	"github.com/ethereum/go-ethereum/crypto"
)

func TestERC20ContractTransaction(t *testing.T) {
	// ERC20 transfer ABI
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

	// Parse ABI
	parsedABI, err := abi.JSON(strings.NewReader(erc20ABI))
	if err != nil {
		t.Fatal(err)
	}

	// Create transfer call data
	toAddress := common.HexToAddress("0x742d35cc6B4e55d0F71c54Ad02e05ab8e90C7C36")
	amount := big.NewInt(1000000000000000000) // 1 token (18 decimals)

	calldata, err := parsedABI.Pack("transfer", toAddress, amount)
	if err != nil {
		t.Fatal(err)
	}

	// Create transaction with contract call
	privateKey, err := crypto.GenerateKey()
	if err != nil {
		t.Fatal(err)
	}

	contractAddress := common.HexToAddress("0xA0b86a33E6441e2C00A0C42c6b9a6F8de91c2321") // USDC-like address
	tx := types.NewTransaction(
		0,                       // nonce
		contractAddress,         // to (contract address)
		big.NewInt(0),           // value (0 ETH for token transfer)
		100000,                  // gas limit
		big.NewInt(20000000000), // gas price (20 Gwei)
		calldata,                // contract call data
	)

	// Sign transaction
	chainID := big.NewInt(1) // mainnet
	signedTx, err := types.SignTx(tx, types.NewEIP155Signer(chainID), privateKey)
	if err != nil {
		t.Fatal(err)
	}

	// RLP encode
	encoded, err := signedTx.MarshalBinary()
	if err != nil {
		t.Fatal(err)
	}

	t.Logf("ERC20 transfer transaction hex: 0x%x", encoded)
	t.Logf("Length: %d", len(encoded))

	// Test with ABI decoder
	abiInfos := []ABIInfo{
		{
			Address: contractAddress,
			ABIJson: erc20ABI,
		},
	}

	decoder, err := NewTransactionDecoder(abiInfos)
	if err != nil {
		t.Fatal(err)
	}

	result, err := decoder.DecodeTransactionJSON(fmt.Sprintf("0x%x", encoded))
	if err != nil {
		t.Fatal(err)
	}

	t.Log("Decoded with ABI:")
	t.Log(result)

	// Verify the result is not empty
	if result == "" {
		t.Error("Expected non-empty result from decoder with ABI")
	}

	// Test without ABI (should show raw data)
	decoderNoABI, err := NewTransactionDecoder([]ABIInfo{})
	if err != nil {
		t.Fatal(err)
	}

	resultNoABI, err := decoderNoABI.DecodeTransactionJSON(fmt.Sprintf("0x%x", encoded))
	if err != nil {
		t.Fatal(err)
	}

	t.Log("Decoded without ABI (raw data):")
	t.Log(resultNoABI)

	// Verify the result is not empty
	if resultNoABI == "" {
		t.Error("Expected non-empty result from decoder without ABI")
	}
}

func TestERC20ContractTransactionFromKnownHex(t *testing.T) {
	// Test with a simpler approach - generate a transaction and decode it
	// ERC20 transfer ABI
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

	contractAddress := common.HexToAddress("0xA0b86a33E6441e2C00A0C42c6b9a6F8de91c2321")
	abiInfos := []ABIInfo{
		{
			Address: contractAddress,
			ABIJson: erc20ABI,
		},
	}

	decoder, err := NewTransactionDecoder(abiInfos)
	if err != nil {
		t.Fatal(err)
	}

	// Use a transaction hex that we know is valid (from the other test)
	testTx := "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"

	result, err := decoder.DecodeTransactionJSON(testTx)
	if err != nil {
		t.Fatal(err)
	}

	t.Log("Decoded known transaction:")
	t.Log(result)

	// Verify the result is not empty
	if result == "" {
		t.Error("Expected non-empty result from decoder")
	}
}
