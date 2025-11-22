/// Quick Decoder Testing Tool
///
/// This file lets you test any Ethereum contract decoder by just changing the calldata.
/// No need to modify tests or recompile the whole project.
use alloy_primitives::hex;
use visualsign::SignablePayloadField;
use visualsign_ethereum::protocols::aave::contracts::PoolVisualizer;
use visualsign_ethereum::protocols::morpho::contracts::BundlerVisualizer;
use visualsign_ethereum::protocols::uniswap::contracts::UniversalRouterVisualizer;
use visualsign_ethereum::registry::ContractRegistry;

fn main() {
    println!("\n╔════════════════════════════════════════════════════════════╗");
    println!("║          🧪 Ethereum Contract Decoder Tester              ║");
    println!("╚════════════════════════════════════════════════════════════╝\n");

    // ═══════════════════════════════════════════════════════════
    // 🔧 CONFIGURATION - CHANGE THESE VALUES TO TEST
    // ═══════════════════════════════════════════════════════════

    // STEP 1: Choose which protocol to test (uncomment one)
    let protocol = "aave"; // Aave v3
                           // let protocol = "morpho";   // Morpho
                           // let protocol = "uniswap";  // Uniswap Universal Router

    // STEP 2: Paste your calldata here (with or without 0x prefix)
    let calldata_hex = "0x563dd613000000000000000000000000000200000000000000000000000006052340000c";

    // STEP 3: Set the chain ID (1 = Ethereum, 137 = Polygon, etc.)
    let chain_id = 42161;

    // ═══════════════════════════════════════════════════════════
    // 🚀 EXECUTION - No need to change anything below
    // ═══════════════════════════════════════════════════════════

    // Decode hex
    let calldata = match hex::decode(calldata_hex.trim_start_matches("0x")) {
        Ok(data) => data,
        Err(e) => {
            eprintln!("❌ Failed to decode hex: {}", e);
            eprintln!("   Make sure your calldata is valid hex!");
            return;
        }
    };

    println!("📊 Test Configuration:");
    println!("  Protocol: {}", protocol.to_uppercase());
    println!("  Chain ID: {}", chain_id);
    println!("  Calldata length: {} bytes", calldata.len());
    println!("  Function selector: 0x{}\n", hex::encode(&calldata[0..4]));

    // Create registry with token metadata
    let registry = ContractRegistry::with_default_protocols();

    // Route to the appropriate visualizer
    let result = match protocol {
        "aave" => {
            println!("🏦 Testing with Aave v3 Pool decoder...\n");
            PoolVisualizer {}.visualize_pool_operation(&calldata, chain_id, Some(&registry))
        }
        "morpho" => {
            println!("🦋 Testing with Morpho Bundler decoder...\n");
            BundlerVisualizer {}.visualize_multicall(&calldata, chain_id, Some(&registry))
        }
        "uniswap" => {
            println!("🦄 Testing with Uniswap Universal Router decoder...\n");
            UniversalRouterVisualizer {}.visualize_tx_commands(&calldata, chain_id, Some(&registry))
        }
        _ => {
            eprintln!("❌ Unknown protocol: {}", protocol);
            eprintln!("   Valid options: aave, morpho, uniswap");
            return;
        }
    };

    // Display results
    match result {
        Some(SignablePayloadField::PreviewLayout {
            common,
            preview_layout,
        }) => {
            println!("╔════════════════════════════════════════════════════════════╗");
            println!("║                    ✅ DECODE SUCCESS!                      ║");
            println!("╚════════════════════════════════════════════════════════════╝\n");

            println!("📋 Label: {}", common.label);
            println!("📝 Summary: {}\n", common.fallback_text);

            if let Some(title) = &preview_layout.title {
                println!("🏷️  Title: {}", title.text);
            }

            if let Some(subtitle) = &preview_layout.subtitle {
                println!("📄 Subtitle: {}\n", subtitle.text);
            }

            if let Some(expanded) = &preview_layout.expanded {
                println!("╔════════════════════════════════════════════════════════════╗");
                println!("║                  📊 Detailed Parameters                    ║");
                println!("╚════════════════════════════════════════════════════════════╝");

                for (i, field) in expanded.fields.iter().enumerate() {
                    match &field.signable_payload_field {
                        SignablePayloadField::TextV2 { common, text_v2 } => {
                            println!("  {}. {}: {}", i + 1, common.label, text_v2.text);
                        }
                        SignablePayloadField::PreviewLayout { common, .. } => {
                            println!("  {}. {} (nested)", i + 1, common.label);
                        }
                        _ => {
                            println!(
                                "  {}. {} ({})",
                                i + 1,
                                match &field.signable_payload_field {
                                    SignablePayloadField::TextV2 { common, .. } => &common.label,
                                    SignablePayloadField::AmountV2 { common, .. } => &common.label,
                                    SignablePayloadField::AddressV2 { common, .. } => &common.label,
                                    _ => "Unknown",
                                },
                                "other type"
                            );
                        }
                    }
                }
                println!();
            }

            println!("╔════════════════════════════════════════════════════════════╗");
            println!("║                     🎉 Test Complete!                      ║");
            println!("╚════════════════════════════════════════════════════════════╝\n");
        }
        Some(_) => {
            println!("⚠️  Decoded but got unexpected format (not PreviewLayout)");
        }
        None => {
            println!("❌ Failed to decode transaction");
            println!("   Possible reasons:");
            println!("   - Wrong protocol selected");
            println!("   - Invalid function selector");
            println!("   - Malformed calldata");
            println!("   - Unsupported function\n");
        }
    }
}
