# Visual Sign Protocol

Structured format for displaying transaction details to users for approval.

## Key Points

- **SignablePayload JSON is NOT canonical** - treat as an opaque string field
- **Display all fields** - at minimum show `FallbackText` for each field
- Field ordering is deterministic but not guaranteed to remain alphabetical
- v1 field types (`text`, `address`, `amount`) exist for backwards compatibility but are not used
- `AnnotatedFields` wrap `SignablePayloadField` with additional wallet context (not part of core spec)

## SignablePayload Structure

```json
{
  "Version": "0",
  "Title": "Withdraw",
  "Subtitle": "to 0x8a6e30eE13d06311a35f8fa16A950682A9998c71",
  "PayloadType": "Withdrawal",
  "Fields": [...]
}
```

| Field       | Type                          | Description          |
|-------------|-------------------------------|----------------------|
| Version     | String                        | Protocol version     |
| Title       | String                        | Primary title        |
| Subtitle    | String (optional)             | Secondary text       |
| PayloadType | String                        | Operation identifier |
| Fields      | Array\<SignablePayloadField\> | Transaction details  |

## Field Types

All fields have common properties:

| Field        | Type   | Description        |
|--------------|--------|--------------------|
| Label        | String | Field label        |
| FallbackText | String | Plain text fallback|
| Type         | String | Type identifier    |

### TextV2
```json
{
  "Label": "Asset",
  "FallbackText": "ETH | Ethereum",
  "Type": "text_v2",
  "TextV2": { "Text": "ETH | Ethereum" }
}
```

### AddressV2
```json
{
  "Label": "Recipient",
  "FallbackText": "0x1234...",
  "Type": "address_v2",
  "AddressV2": {
    "Address": "0x1234...",
    "Name": "My Wallet",
    "Memo": "optional memo",
    "AssetLabel": "ETH",
    "BadgeText": "Verified"
  }
}
```

### AmountV2
```json
{
  "Label": "Amount",
  "FallbackText": "0.00001234 BTC",
  "Type": "amount_v2",
  "AmountV2": { "Amount": "0.00001234", "Abbreviation": "BTC" }
}
```

### Number
```json
{
  "Label": "gasLimit",
  "FallbackText": "21000",
  "Type": "number",
  "Number": { "Number": "21000" }
}
```

### Divider
```json
{
  "Label": "",
  "Type": "divider",
  "Divider": { "Style": "thin" }
}
```

### PreviewLayout

Condensed/expanded view for complex data:

```json
{
  "Type": "preview_layout",
  "PreviewLayout": {
    "Title": { "Text": "Delegate" },
    "Subtitle": { "Text": "1 SOL" },
    "Condensed": { "Fields": [...] },
    "Expanded": { "Fields": [...] }
  }
}
```

## Adding New Field Types

1. Define the struct:
```rust
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct SignablePayloadFieldCurrency {
    #[serde(rename = "CurrencyCode")]
    pub currency_code: String,
    #[serde(rename = "Symbol")]
    pub symbol: String,
}
```

2. Add the enum variant to `SignablePayloadField`:
```rust
#[serde(rename = "currency")]
Currency {
    #[serde(flatten)]
    common: SignablePayloadFieldCommon,
    #[serde(rename = "Currency")]
    currency: SignablePayloadFieldCurrency,
},
```

3. Add to `serialize_to_map()`:
```rust
SignablePayloadField::Currency { common, currency } => {
    serialize_field_variant!(fields, "currency", common, ("Currency", currency));
},
```

4. Add to `get_expected_fields()`:
```rust
SignablePayloadField::Currency { .. } => base_fields.push("Currency"),
```

5. Add to `field_type()`:
```rust
SignablePayloadField::Currency { .. } => "currency",
```

6. Implement `DeterministicOrdering` for the new struct.

## Example Fixtures

Bitcoin Withdraw:
![Bitcoin Withdraw](docs/testFixtures.bitcoin_withdraw_fixture_generation.png)

ERC20 Token Withdraw:
![ERC20 Token Withdraw](docs/testFixtures.erc20_withdraw.png)

Solana Withdraw with expandable layouts:
![Solana withdraw](docs/testFixtures.solana_withdraw_fixture_generation.png)

Expanded details:
1. ![Details 1](docs/testFixtures.solana_withdraw_fixture_generation_expandable_details_1.png)
2. ![Details 2](docs/testFixtures.solana_withdraw_fixture_generation_expandable_details_2.png)
