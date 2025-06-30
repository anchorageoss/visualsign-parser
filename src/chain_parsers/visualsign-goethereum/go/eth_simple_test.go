package main

import (
	"fmt"
	"math/big"
	"testing"

	"github.com/ethereum/go-ethereum/common"
	"github.com/ethereum/go-ethereum/core/types"
	"github.com/ethereum/go-ethereum/crypto"
	"github.com/ethereum/go-ethereum/rlp"
)

func TestSimpleEthereumTransaction(t *testing.T) {
	// Create a simple test transaction
	privateKey, err := crypto.GenerateKey()
	if err != nil {
		t.Fatal(err)
	}

	// Create transaction
	tx := types.NewTransaction(
		0, // nonce
		common.HexToAddress("0x3535353535353535353535353535353535353535"), // to
		big.NewInt(1000000000000000000),                                   // value (1 ETH)
		21000,                                                             // gas limit
		big.NewInt(20000000000),                                           // gas price (20 Gwei)
		nil,                                                               // data
	)

	// Sign transaction
	chainID := big.NewInt(1) // mainnet
	signedTx, err := types.SignTx(tx, types.NewEIP155Signer(chainID), privateKey)
	if err != nil {
		t.Fatal(err)
	}

	// RLP encode
	encoded, err := rlp.EncodeToBytes(signedTx)
	if err != nil {
		t.Fatal(err)
	}

	t.Logf("Test transaction hex: 0x%x", encoded)
	t.Logf("Length: %d", len(encoded))

	// Now test our decoder
	decoder, err := NewTransactionDecoder([]ABIInfo{})
	if err != nil {
		t.Fatal(err)
	}

	result, err := decoder.DecodeTransactionJSON(fmt.Sprintf("0x%x", encoded))
	if err != nil {
		t.Fatal(err)
	}

	t.Log("Decoded result:")
	t.Log(result)

	// Verify the result is not empty
	if result == "" {
		t.Error("Expected non-empty result from decoder")
	}
}

func TestSimpleEthereumTransactionFromHex(t *testing.T) {
	// Test with a known transaction hex
	testTx := "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"

	decoder, err := NewTransactionDecoder([]ABIInfo{})
	if err != nil {
		t.Fatal(err)
	}

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
