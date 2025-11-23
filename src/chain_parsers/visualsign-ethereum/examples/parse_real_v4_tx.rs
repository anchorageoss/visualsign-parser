use visualsign_ethereum::protocols::uniswap::V4PoolManagerVisualizer;
use alloy_consensus::{TxEnvelope, TypedTransaction, Transaction};
use alloy_rlp::Decodable;
use hex;
use serde_json;
use std::fs;
use std::path::{Path, PathBuf};

fn main() {
    // Process both transaction files
    // Try to find files relative to workspace root
    let base_paths = vec![
        PathBuf::from("src"),
        PathBuf::from("../src"),
        PathBuf::from("../../src"),
        PathBuf::from("../../../src"),
    ];

    let files = vec![
        ("tx_initializev4.txt", "Initialize"),
        ("tx_swapv4.txt", "Swap"),
    ];

    for (file_name, tx_type) in files {
        let mut found = false;
        for base in &base_paths {
            let file_path = base.join(file_name);
            if file_path.exists() {
                println!("\n{}", "=".repeat(80));
                println!("Processing {} transaction from: {}", tx_type, file_path.display());
                println!("{}", "=".repeat(80));

                match process_transaction_file(&file_path) {
                    Ok(_) => {
                        println!("\n✅ Successfully processed {} transaction", tx_type);
                        found = true;
                        break;
                    }
                    Err(e) => eprintln!("\n❌ Error processing {} transaction: {}", tx_type, e),
                }
            }
        }
        if !found {
            eprintln!("\n❌ Could not find {} transaction file: {}", tx_type, file_name);
        }
    }
}

fn process_transaction_file(file_path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    // Read transaction hex from file
    let tx_hex = fs::read_to_string(file_path)?
        .lines()
        .next()
        .ok_or("File is empty")?
        .trim()
        .to_string();

    // Decode hex string
    let clean_hex = tx_hex.strip_prefix("0x").unwrap_or(&tx_hex);
    let tx_bytes = hex::decode(clean_hex)
        .map_err(|e| format!("Error decoding hex: {}", e))?;
    
    // Try to decode as signed transaction (TxEnvelope)
    let mut tx_bytes_slice = tx_bytes.as_slice();
    let unsigned_tx = match TxEnvelope::decode(&mut tx_bytes_slice) {
        Ok(envelope) => {
            match envelope {
                TxEnvelope::Eip1559(signed) => TypedTransaction::Eip1559(signed.tx().clone()),
                TxEnvelope::Eip2930(signed) => TypedTransaction::Eip2930(signed.tx().clone()),
                TxEnvelope::Eip4844(signed) => TypedTransaction::Eip4844(signed.tx().clone()),
                TxEnvelope::Legacy(signed) => TypedTransaction::Legacy(signed.tx().clone()),
                TxEnvelope::Eip7702(signed) => TypedTransaction::Eip7702(signed.tx().clone()),
            }
        }
        Err(e) => {
            return Err(format!("Error decoding as signed transaction: {}", e).into());
        }
    };
    
    // Extract calldata and chain ID
    let input = unsigned_tx.input();
    let chain_id = unsigned_tx.chain_id().unwrap_or(1);
    
    // Try V4 PoolManager visualizer
    let visualizer = V4PoolManagerVisualizer;
    println!("\n=== V4 Transaction Visualization (JSON) ===");
    if let Some(field) = visualizer.visualize_tx_commands(input, chain_id, None) {
        println!("{}", serde_json::to_string_pretty(&field)?);
    } else {
        println!("Failed to visualize V4 transaction");
    }
    
    Ok(())
}
