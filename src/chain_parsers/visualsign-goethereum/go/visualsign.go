package main

import (
	"errors"
	"fmt"
)

// constraint on fields to make it easy to support both types of payload fields generically
type PayloadFieldConstraint interface {
	*SignablePayloadField | *AnnotatedPayloadField
}

// SignablePayload is a JSON-encoded string containing fields for every piece of information that the HSM will execute. This is the exact payload that a user endorses.
// It is derived from user intent, and must be crafted in the TEE.
type SignablePayload struct {
	Version     int64 `json:",string"`
	Title       string
	Subtitle    string `json:",omitempty"`
	Fields      []SignablePayloadField
	PayloadType string // [New] field to specify the type of payload, e.g., "transaction", "message", etc.
	// EndorsedParamsDigest is required for Protocols and is the hash of
	// the endorsedParams.
	EndorsedParamsDigest []byte `json:",omitempty"`
}

// Validate ensures that the signable payload has valid fields
func (p *SignablePayload) Validate() error {
	for i, field := range p.Fields {
		if err := field.Validate(); err != nil {
			return fmt.Errorf("failed to validate field idx %d: %w", i, err)
		}
	}
	return nil
}

// SignablePayloadField represents the metadata about a field for a signable payload.
type SignablePayloadField struct {
	FallbackText  string
	Type          string // used to visualize the data. One of: address, amount...
	Label         string
	PreviewLayout *SignablePayloadFieldPreviewLayout `json:",omitempty"`
	ListLayout    *SignablePayloadFieldListLayout    `json:",omitempty"`
	Text          *SignablePayloadFieldText          `json:",omitempty"` // Deprecated: use TextV2 instead
	TextV2        *SignablePayloadFieldTextV2        `json:",omitempty"`
	Address       *SignablePayloadFieldAddress       `json:",omitempty"` // Deprecated: use AddressV2 instead
	AddressV2     *SignablePayloadFieldAddressV2     `json:",omitempty"`
	Number        *SignablePayloadFieldNumber        `json:",omitempty"`
	Amount        *SignablePayloadFieldAmount        `json:",omitempty"` // Deprecated: use AmountV2 instead
	AmountV2      *SignablePayloadFieldAmountV2      `json:",omitempty"`
	Divider       *SignablePayloadFieldDivider       `json:",omitempty"`
	Unknown       *SignablePayloadFieldUnknown       `json:",omitempty"`
}

func (f *SignablePayloadField) Validate() error {
	if f.Type == "" {
		return errors.New("type is empty")
	}
	if f.FallbackText == "" {
		return errors.New("fallback_text is empty")
	}
	switch f.Type {
	case "text_v2":
		if f.TextV2 != nil && f.TextV2.Text == "" {
			return errors.New("text.text is empty")
		}
		if f.Text != nil {
			return errors.New("field type is declared as TextV2 but Text is set")
		}
	case "text":
		if f.Text != nil && f.Text.Text == "" {
			return errors.New("text.text is empty")
		}
		if f.TextV2 != nil {
			return errors.New("field type is declared as Text but TextV2 is set")
		}
	case "address_v2":
		if f.AddressV2 == nil {
			return errors.New("address_v2 field is nil")
		}
		if f.AddressV2.Address == "" {
			return errors.New("address_v2.address is empty")
		}
		if f.Address != nil {
			return errors.New("field type is declared as AddressV2 but Address is set")
		}
	case "address":
		if f.Address == nil {
			return errors.New("address field is nil")
		}
		if f.Address.Address == "" {
			return errors.New("address.address is empty")
		}
		if f.AddressV2 != nil {
			return errors.New("field type is declared as Address but AddressV2 is set")
		}
	case "amount_v2":
		if f.AmountV2 == nil {
			return errors.New("amount_v2 field is nil")
		}
		if f.AmountV2.Amount == "" {
			return errors.New("amount_v2.amount is empty")
		}
		if f.Amount != nil {
			return errors.New("field type is declared as AmountV2 but Amount is set")
		}
	case "amount":
		if f.Amount == nil {
			return errors.New("amount field is nil")
		}
		if f.Amount.Amount == "" {
			return errors.New("amount.amount is empty")
		}
		if f.AmountV2 != nil {
			return errors.New("field type is declared as Amount but AmountV2 is set")
		}
	case "number":
		// fallbacktext covers it here
	case "divider":
		if f.Divider == nil {
			return errors.New("divider field is nil")
		}
	case "preview_layout":
		if f.PreviewLayout == nil {
			return errors.New("preview_layout field is nil")
		}
	case "list_layout":
		if f.ListLayout == nil {
			return errors.New("list_layout field is nil")
		}
	case "unknown":
		if f.Unknown == nil {
			return errors.New("unknown field is nil")
		}
		if f.Unknown.Data == "" {
			return errors.New("unknown.data is empty")
		}
		if f.Unknown.Explanation == "" {
			return errors.New("unknown.explanation is empty")
		}
	default:
		return fmt.Errorf("unsupported field type: %s", f.Type)
	}
	return nil
}

type DividerStyle string

const (
	DividerStyleThin DividerStyle = "THIN"
)

type SignablePayloadFieldDivider struct {
	Style DividerStyle
}

type SignablePayloadFieldPreviewLayout struct {
	Title     SignablePayloadFieldTextV2
	Subtitle  *SignablePayloadFieldTextV2 `json:",omitempty"`
	Condensed SignablePayloadFieldListLayout
	Expanded  SignablePayloadFieldListLayout
}

type SignablePayloadFieldListLayout struct {
	Fields []*AnnotatedPayloadField
}

// SignablePayloadFieldText represents a value of string type for a field of a signable payload.
type SignablePayloadFieldText struct {
	Text string
}

type SignablePayloadFieldTextV2 struct {
	Text string
}

// SignablePayloadFieldAddress represents a value of address type for a field of a signable payload.
type SignablePayloadFieldAddress struct {
	Address string
	Name    string // a vault name, contract name, trusted destination name, etc.
}

type SignablePayloadFieldAddressV2 struct {
	Address    string
	Name       string
	Memo       string `json:",omitempty"`
	AssetLabel string
	BadgeText  string `json:",omitempty"`
}

// SignablePayloadFieldNumber represents a value of number type for a field of a signable payload.
type SignablePayloadFieldNumber struct {
	Number float64
}

// SignablePayloadFieldAmount represents a value of amount type for a field of a signable payload.
type SignablePayloadFieldAmount struct {
	// Amount is the string value in decimals of the number value - this is more user friendly version of Number
	Amount       string
	Abbreviation string `json:",omitempty"` // derived from assettype.ID. In the future, it might be derived from a more complex multi-part ID and that's okay as long as the "reconstruct" function knows how to derive it.
}

// SignablePayloadFieldAmount represents a value of amount type for a field of a signable payload.
type SignablePayloadFieldAmountV2 struct {
	// Amount is the string value in decimals of the number value - this
	// is more user friendly version of Number.
	Amount string
	// Abbreviation is derived from assettype.ID. In the future, it might be derived
	// from a more complex multi-part ID and that's okay as long as the "reconstruct"
	// function knows how to derive it.
	Abbreviation string `json:",omitempty"`
}

// SignablePayloadFieldUnknown represents a value of data that is unknown to the ABI for a field of a signable payload.
type SignablePayloadFieldUnknown struct {
	// Data is the hex-encoded data that is sent to the contract and unknown to the ABI.
	Data string
	// Explanation is a human-readable explanation of why the data is unknown
	Explanation string
}

type SignablePayloadFieldStaticAnnotation struct {
	Text string
}

type SignablePayloadFieldDynamicAnnotation struct {
	Type   string
	ID     string
	Params []string
}

type AnnotatedPayload struct {
	Version  int64                   `json:",string"`
	Title    string                  `json:",omitempty"`
	Subtitle string                  `json:",omitempty"`
	Fields   []AnnotatedPayloadField `json:",omitempty"`
}

type AnnotatedPayloadField struct {
	SignablePayloadField
	StaticAnnotation  *SignablePayloadFieldStaticAnnotation  `json:",omitempty"`
	DynamicAnnotation *SignablePayloadFieldDynamicAnnotation `json:",omitempty"`
}
