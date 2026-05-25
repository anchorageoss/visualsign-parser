//! Example: render an EIP-2612 USDC Permit signing request via ERC-7730 descriptor.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use visualsign::vsptrait::VisualSignOptions;
use visualsign_ethereum::eip712_typed_data_to_visual_sign;

fn main() {
    let payload = r#"{
      "domain": {
        "name": "USD Coin", "version": "2", "chainId": "0x1",
        "verifyingContract": "0xa0b86991c6218b36c1d19d4a2e9eb0ce3606eb48"
      },
      "primaryType": "Permit",
      "types": {
        "EIP712Domain": [
          {"name": "name", "type": "string"},
          {"name": "version", "type": "string"},
          {"name": "chainId", "type": "uint256"},
          {"name": "verifyingContract", "type": "address"}
        ],
        "Permit": [
          {"name": "owner", "type": "address"},
          {"name": "spender", "type": "address"},
          {"name": "value", "type": "uint256"},
          {"name": "nonce", "type": "uint256"},
          {"name": "deadline", "type": "uint256"}
        ]
      },
      "message": {
        "owner":   "0x1111111111111111111111111111111111111111",
        "spender": "0x2222222222222222222222222222222222222222",
        "value":   "1000000",
        "nonce":   "0",
        "deadline": "1900000000"
      }
    }"#;
    let result = eip712_typed_data_to_visual_sign(payload, VisualSignOptions::default())
        .expect("conversion failed");
    println!("Title: {}", result.title);
    println!("PayloadType: {}", result.payload_type);
    println!("Fields:");
    for f in &result.fields {
        match f {
            visualsign::SignablePayloadField::TextV2 { common, text_v2 } => {
                println!("  [Text]    {}: {}", common.label, text_v2.text);
            }
            visualsign::SignablePayloadField::AddressV2 { common, address_v2 } => {
                println!(
                    "  [Address] {}: {} (name: '{}')",
                    common.label, address_v2.address, address_v2.name
                );
            }
            visualsign::SignablePayloadField::AmountV2 { common, amount_v2 } => {
                println!(
                    "  [Amount]  {}: {} {}",
                    common.label,
                    amount_v2.amount,
                    amount_v2.abbreviation.as_deref().unwrap_or("")
                );
            }
            other => println!("  [Other]   {:?}", other.label()),
        }
    }
}
