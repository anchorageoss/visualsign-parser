# Visual Sign Protocol Documentation
This document provides specifications for the Visual Sign Protocol (VSP), a structured format for displaying transaction details to users for approval. The VSP is designed to present meaningful, human-readable information about operations requiring signatures.

**Notes**
* We don't use v1 text field types, but they're around for backwards compatibilty for now
* AnnotatedFields are a layer on top of SignablePayload field for our wallet to provide more context, it's not in scope of the SignablePayload, it's still in structs but we'll consider removing in future

## SignablePayload
A SignablePayload is the core structure that defines what is displayed to the user during the signing process. It contains metadata about the transaction and a collection of fields representing the transaction details.

### Structure
<details> <summary>SignablePayload Structure</summary>

```json
{
  "Version": "0",
  "Title": "Withdraw",
  "Subtitle": "to 0x8a6e30eE13d06311a35f8fa16A950682A9998c71",
  "Fields": [
    {
      "FallbackText": "1 ETH",
      "Label": "Amount",
      "Type": "amount_v2",
      "AmountV2": {
        "Amount": "1",
        "Abbreviation": "ETH"
      }
    },
    ...
  ],
  "EndorsedParamsDigest": "DEADBEEFDEADBEEFDEADBEEFDEADBEEF",
}
```
</details>


### Payload Components

| Field                 | Type                       | Description                               |
|-----------------------|----------------------------|-------------------------------------------|
| Version               | String                     | Protocol version                          |
| Title                 | String                     | Primary title for the operation           |
| Subtitle              | String (optional)          | Secondary descriptive text                |
| PayloadType                | String | Identifier for the SignablePayload (ex: Withdrawal, Swap, etc)|
| Fields                | Array of SignablePayloadField | The fields containing transaction details |
| EndorsedParamsDigest  | String (optional)          | Digest of endorsed parameters             |

## Field Types
The Visual Sign Protocol supports various field types to represent different kinds of data.

#### Common Field Structure
All field types include these common properties:

<details> <summary>Common Field Properties</summary>

```json
{
  "Label": "Amount",
  "FallbackText": "1 ETH",
  "Type": "amount_v2"
}
```
</details>


| Field         | Type   | Description                                |
|---------------|--------|--------------------------------------------|
| Label         | String | Field label shown to the user              |
| FallbackText  | String | Plain text representation (for limited clients) |
| Type          | String | Type identifier for the field              |

### Specific Field Types


#### Text Fields
<details> <summary>Text Field Example</summary>

```json
{
  "Label": "Asset",
  "FallbackText": "ETH | Ethereum",
  "Type": "text_v2",
  "TextV2": {
    "Text": "ETH | Ethereum"
  }
}
```
</details>


#### Address Fields
<details> <summary>Address Field Example</summary>

```json
{
  "Label": "Amount",
  "FallbackText": "0.00001234",
  "Type": "amount_v2",
  "AmountV2": {
    "Amount": "0.00001234",
    "Abbreviation": "BTC"
  }
}
```
</details>

### Amount Fields
Amount fields are user friendly ways to display the value being transferred 
<details> <summary>Amount Field Example</summary>

```json
{
  "Label": "Amount",
  "FallbackText": "0.00001234 BTC",
  "Type": "amount_v2",
  "AmountV2": {
    "Amount": "0.00001234",
    "Abbreviation": "BTC"
  }
}
```
</details>

### Number Fields

<details> <summary>Number Field Example</summary>

```json
{
  "Label": "gasLimit",
  "FallbackText": "21000",
  "Type": "number",
  "Number": {
    "Value": "21000"
  }
}
```
</details>

### Divider Fields

Divider fields are UI elements to split the UI on. This is used for clarity and to allow the UI to keep views in separate pages if needed.

<details> <summary>Divider Field Example</summary>

```json
{
  "Label": "",
  "Type": "divider",
  "Divider": {
    "Style": "thin"
  }
}
```
</details>


### Layout Fields
We have additional layout fields for two different use cases - one for creating preview elements, where a condensed view can be optionally expanded by the user. 

<details> <summary>Preview Layout Field Example</summary>

```json
{
  "Type": "preview_layout",
  "PreviewLayout": {
    "Title": "Delegate",
    "Subtitle": "1 SOL Delegated to Jito4APyf642JPZPx3hGc6WWJ8zPKtRbRs4P815Awbb"
  },
  "Condensed": {
    "Fields": [ /* array of SignablePayloadFields */]
  },
  "Expanded": {
    "Fields": [ /* array of SignablePayloadFields */]
  }
}
```
</details> 


## Endorsed Params

The Endorsed Params feature allows passing additional parameters for the visualizer to interpret and potentially use for transforming the raw transaction to make meaningful display for user in a deterministic way. 

### Structure

Endorsed parameters are cryptographically bound to the SignablePayload through the `EndorsedParamsDigest` field, which contains a hash of all endorsed parameters. These are presented as an example - and may be chain or wallet-specific.

<details> <summary>Endorsed Params Structure</summary>

```json
{
  "EndorsedParams": {
    "ChainId": "1",
    "ContractAddress": "0x6B175474E89094C44Da98b954EedeAC495271d0F",
    "MethodSignature": "transfer(address,uint256)",
    "Nonce": "42",
    "CallData": "0xa9059cbb000000000000000000000000...",
    "ABIs": {},
    "IDLs": {}
  }
}
```
</details>

### Usage

1. **Transaction Construction**: The visualizer service collects all necessary parameters for constructing a valid transaction.

2. **Parameter Separation**: Parameters are separated into:
   - User-facing fields (included in the `Fields` array)
   - Hidden parameters (included in `EndorsedParams`)

3. **Digest Creation**: The service computes a hash of the endorsed parameters:
   ```
   EndorsedParamsDigest = sha256(serialize(EndorsedParams))
   ```

4. **Payload Assembly**: The digest is included in the SignablePayload, cryptographically binding the hidden parameters to the displayed information.

### Security Considerations

- The signer must verify that the `EndorsedParamsDigest` matches the endorsed parameters used for transaction construction
- Parameters that affect user funds or authorization should generally be displayed rather than hidden
- Implementations should document which parameters are endorsed vs. displayed to ensure transparency

### Example Use Cases

- Network fees and gas parameters
- Technical identifiers (contract addresses, chain IDs)
- Implementation-specific parameters (nonces, replay protection values)
- Method signatures and serialized call data


## Example Fixtures
Below are screenshots corresponding to specific fixture examples:

Bitcoin Withdraw
![Bitcoin Withdraw using visualsign](docs/testFixtures.bitcoin_withdraw_fixture_generation.png)

ERC20 Token Withdraw
![ERC20 Token Withdraw](docs/testFixtures.erc20_withdraw.png)

Solana Withdraw with Expandable Preview Layouts
![Solana withdraw main page](docs/testFixtures.solana_withdraw_fixture_generation.png)
Expanding fields, these are expected to be shown when one of the expandable fields is clicked

1. ![Solana details1](docs/testFixtures.solana_withdraw_fixture_generation_expandable_details_1.png)
2. ![alt text](docs/testFixtures.solana_withdraw_fixture_generation_expandable_details_2.png)


### Implementation Considerations
Field Ordering: Fields should be displayed in the order they appear in the Fields array
Version Compatibility: Clients should check the Version field to ensure they can properly render the payload
Fallback Rendering: If a client doesn't understand a field type, it should fall back to displaying the FallbackText
Security: Implementations should validate the ReplayProtection and EndorsedParamsDigest values


## Extending SignablePayloadField Types

The VisualSign Protocol is designed to be extensible, allowing developers to safely add new field types while maintaining backward compatibility and ensuring data integrity.

### Architecture Overview

The field serialization system uses a **trait-based architecture with runtime verification** that provides multiple layers of protection against incomplete implementations:

```rust
trait FieldSerializer {
    fn serialize_to_map(&self) -> Result<BTreeMap<String, Value>, Error>;
    fn get_expected_fields(&self) -> Vec<&'static str>;
}
```

### Key Features

- **🔒 Runtime Verification**: Automatically verifies all expected fields are present during serialization
- **📝 Alphabetical Ordering**: Fields are automatically sorted alphabetically for consistent output
- **🚨 Error Detection**: Missing or unexpected fields cause immediate serialization failure with detailed error messages
- **🧪 Test-Driven**: Comprehensive test suite proves the verification system works correctly
- **🔄 Extensible**: Adding new field types is straightforward and safe

### How to Add New Field Types

#### 1. Define the Field Structure

First, create the data structure for your new field type:

```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldCurrency {
    #[serde(rename = "CurrencyCode")]
    pub currency_code: String,
    #[serde(rename = "Symbol")]
    pub symbol: String,
    #[serde(rename = "ExchangeRate", skip_serializing_if = "Option::is_none")]
    pub exchange_rate: Option<String>,
}
```

#### 2. Add the Enum Variant

Add your new variant to the `SignablePayloadField` enum:

```rust
pub enum SignablePayloadField {
    // ... existing variants ...

    #[serde(rename = "currency")]
    Currency {
        #[serde(flatten)]
        common: SignablePayloadFieldCommon,
        #[serde(rename = "Currency")]
        currency: SignablePayloadFieldCurrency,
    },
}
```

#### 3. Implement Serialization Logic

Add your field to both required methods in the `FieldSerializer` implementation:

```rust
impl FieldSerializer for SignablePayloadField {
    fn serialize_to_map(&self) -> Result<BTreeMap<String, Value>, Error> {
        let mut fields = HashMap::new();
        match self {
            // ... existing variants ...

            SignablePayloadField::Currency { common, currency } => {
                serialize_field_variant!(fields, "currency", common, ("Currency", currency));
            },
        }
        Ok(fields.into_iter().collect())
    }

    fn get_expected_fields(&self) -> Vec<&'static str> {
        let mut base_fields = vec!["FallbackText", "Label", "Type"];
        match self {
            // ... existing variants ...

            SignablePayloadField::Currency { .. } => base_fields.push("Currency"),
        }
        base_fields.sort();
        base_fields
    }
}
```

#### 4. Update Helper Methods

Add your variant to the existing helper methods:

```rust
impl SignablePayloadField {
    pub fn field_type(&self) -> &str {
        match self {
            // ... existing variants ...
            SignablePayloadField::Currency { .. } => "currency",
        }
    }

    // Update other helper methods as needed...
}
```

### Runtime Verification System

The system automatically verifies field completeness during serialization:

```rust
// ✅ Successful serialization - all fields present
let currency_field = SignablePayloadField::Currency {
    common: SignablePayloadFieldCommon {
        fallback_text: "USD ($)".to_string(),
        label: "Payment Currency".to_string(),
    },
    currency: SignablePayloadFieldCurrency {
        currency_code: "USD".to_string(),
        symbol: "$".to_string(),
        exchange_rate: None,
    },
};

let json = serde_json::to_string(&currency_field)?;
// Result: {"Currency":{"CurrencyCode":"USD","Symbol":"$"},"FallbackText":"USD ($)","Label":"Payment Currency","Type":"currency"}
```

If you forget to serialize a field or have mismatched expectations:

```rust
// ❌ This would fail with detailed error message:
// "Missing expected field 'Currency'. Expected: ["Currency", "FallbackText", "Label", "Type"], Actual: ["FallbackText", "Label", "Type"]"
```

### Comprehensive Testing

The system includes extensive tests that prove the verification works:

```rust
#[test]
fn test_new_field_type() {
    // Test that new field type serializes correctly with verification
    let field = SignablePayloadField::Currency { /* ... */ };

    // This will succeed only if ALL expected fields are present and correctly serialized
    let result = serde_json::to_string(&field);
    assert!(result.is_ok());

    // Verify alphabetical ordering
    let value: serde_json::Value = serde_json::from_str(&result.unwrap()).unwrap();
    if let serde_json::Value::Object(map) = value {
        let keys: Vec<_> = map.keys().cloned().collect();
        // Keys are automatically in alphabetical order
        assert_eq!(keys, vec!["Currency", "FallbackText", "Label", "Type"]);
    }
}
```

### Benefits of This Approach

1. **🛡️ Defense in Depth**:
   - **Compile-time**: Exhaustive pattern matching ensures all variants are handled
   - **Runtime**: Field verification catches missing/incorrect fields
   - **Test-time**: Comprehensive tests prove the system works

2. **🔍 Clear Error Messages**:
   - Missing fields are immediately identified with specific field names
   - Unexpected fields are caught and reported
   - Detailed error context helps debugging

3. **📊 Consistent Output**:
   - All fields automatically ordered alphabetically
   - Consistent JSON structure across all field types
   - Backward compatibility maintained

4. **🚀 Easy Extension**:
   - Adding new field types requires minimal code changes
   - Macro-based approach reduces boilerplate
   - Impossible to miss required implementation steps

### Migration from Legacy Approach

The new system maintains full backward compatibility while adding safety:

- All existing field types work unchanged
- JSON output format is identical
- No breaking changes to API
- Existing tests continue to pass

### Best Practices

1. **Always test new field types** with the provided verification tests
2. **Use descriptive field names** that clearly indicate their purpose
3. **Follow the naming convention** of existing field types
4. **Document new field types** in this README
5. **Consider backward compatibility** when designing new field structures

This extensible architecture transforms field extension from a error-prone manual process into a safe, verified, and automatic system that catches mistakes before they can cause issues in production.