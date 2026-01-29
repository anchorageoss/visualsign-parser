// Test file to verify library documentation examples compile
// Run with: cargo run -p library_integration_test

use generated::parser::{chain_metadata::Metadata, ChainMetadata, EthereumMetadata};
use parser_app::registry::create_registry;
use visualsign::registry::Chain;
use visualsign::vsptrait::{DeveloperConfig, VisualSignOptions};
use visualsign::{SignablePayload, SignablePayloadField};

/// Display a SignablePayload in human-readable format
fn display_payload(payload: &SignablePayload) {
    println!("Transaction: {}", payload.title);

    if let Some(subtitle) = &payload.subtitle {
        println!("  {subtitle}");
    }

    for field in &payload.fields {
        match field {
            SignablePayloadField::TextV2 { common, text_v2 } => {
                println!("  {}: {}", common.label, text_v2.text);
            }
            SignablePayloadField::AmountV2 { common, amount_v2 } => {
                println!(
                    "  {}: {} {}",
                    common.label,
                    amount_v2.amount,
                    amount_v2.abbreviation.as_deref().unwrap_or("")
                );
            }
            SignablePayloadField::AddressV2 { common, address_v2 } => {
                println!("  {}: {}", common.label, address_v2.address);
            }
            _ => {
                // Handle other field types as needed
            }
        }
    }
}

fn main() {
    // Create the parser registry (includes all supported chains)
    let registry = create_registry();

    // A real Ethereum EIP-1559 transaction (Uniswap swap)
    let raw_tx = "02f903f801820232844b627561846952bea58304023c9466a9893cc07d91d95644aedd05d03f95e1dba8af8711c37937e08000b903c53593564c000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000000a0000000000000000000000000000000000000000000000000000000006882a27000000000000000000000000000000000000000000000000000000000000000040b080604000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000e000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000280000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000011c37937e08000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000011c37937e08000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002000000000000000000000000c02aaa39b223fe8d0a0e5c4f27ead9083c756cc2000000000000000000000000b1137b9ce6db98312bc9dcb3a8a41eb3d212776f0000000000000000000000000000000000000000000000000000000000000060000000000000000000000000b1137b9ce6db98312bc9dcb3a8a41eb3d212776f000000000000000000000000000000fee13a103a10d593b9ae06b3e05f2e7e1c00000000000000000000000000000000000000000000000000000000000000190000000000000000000000000000000000000000000000000000000000000060000000000000000000000000b1137b9ce6db98312bc9dcb3a8a41eb3d212776f0000000000000000000000006b95d095598e1a080cb62e8ccd99dd64853f1b9900000000000000000000000000000000000000000000000000000e2ab638514b0bc0";

    // Configure parsing options
    let options = VisualSignOptions {
        decode_transfers: true,
        transaction_name: None,
        metadata: Some(ChainMetadata {
            metadata: Some(Metadata::Ethereum(EthereumMetadata {
                network_id: Some("1".to_string()), // Ethereum Mainnet
                abi: None,
            })),
        }),
        developer_config: Some(DeveloperConfig {
            allow_signed_transactions: true,
        }),
        abi_registry: None,
    };

    // Parse the transaction
    match registry.convert_transaction(&Chain::Ethereum, raw_tx, options) {
        Ok(payload) => {
            println!("=== Parsing successful! ===\n");
            display_payload(&payload);
        }
        Err(e) => {
            eprintln!("Parse error: {e:?}");
            std::process::exit(1);
        }
    }
}
