package main

/*
#include <stdlib.h>
*/
import "C"
import (
	"encoding/json"
	"log"
	"unsafe"
)

// Simple C interface for the Ethereum decoder

//export DecodeEthereumTransactionToJSON
func DecodeEthereumTransactionToJSON(rawTxHex *C.char) *C.char {
	// Convert C string to Go string
	goString := C.GoString(rawTxHex)

	// Create a decoder with no ABI mappings for now
	decoder, err := NewTransactionDecoder([]ABIInfo{})
	if err != nil {
		log.Printf("Failed to create decoder: %v", err)
		return C.CString(`{"error": "Failed to create decoder"}`)
	}

	// Decode the transaction
	payload, err := decoder.DecodeRawTransaction(goString)
	if err != nil {
		log.Printf("Failed to decode transaction: %v", err)
		return C.CString(`{"error": "Failed to decode transaction"}`)
	}

	// Convert to JSON
	jsonBytes, err := json.Marshal(payload)
	if err != nil {
		log.Printf("Failed to marshal JSON: %v", err)
		return C.CString(`{"error": "Failed to marshal JSON"}`)
	}

	return C.CString(string(jsonBytes))
}

//export FreeString
func FreeString(s *C.char) {
	C.free(unsafe.Pointer(s))
}
