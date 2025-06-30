package main

import (
	"testing"
)

func TestDecoderCreation(t *testing.T) {
	// Test that we can create a decoder without panicking
	decoder, err := NewTransactionDecoder([]ABIInfo{})
	if err != nil {
		t.Fatal(err)
	}

	if decoder == nil {
		t.Error("Expected non-nil decoder")
	}
}

func TestDecoderWithEmptyABI(t *testing.T) {
	// Test decoder creation with empty ABI list
	decoder, err := NewTransactionDecoder([]ABIInfo{})
	if err != nil {
		t.Fatal(err)
	}

	// Test with a simple transaction hex
	testHex := "0xf86c808504a817c800825208943535353535353535353535353535353535353535880de0b6b3a76400008025a028ef61340bd939bc2195fe537567866003e1a15d3c71ff63e1590620aa636276a067cbe9d8997f761aecb703304b3800ccf555c9f3dc64214b297fb1966a3b6d83"

	result, err := decoder.DecodeTransactionJSON(testHex)
	if err != nil {
		t.Fatal(err)
	}

	t.Logf("Decoder result: %s", result)

	// Verify the result is not empty
	if result == "" {
		t.Error("Expected non-empty result from decoder")
	}
}
