use visualsign_ethereum::protocols::uniswap::V4PoolManagerVisualizer;
use visualsign_ethereum::transaction_to_visual_sign;
use visualsign::vsptrait::VisualSignOptions;
use alloy_consensus::{transaction::Transaction, TxEnvelope};
use alloy_rlp::Decodable;
use hex;
use serde_json;

fn main() {
    // Real transaction hex from Sepolia (signed transaction)
    let tx_hex = "0x02f906fe01820eda8405f5e100840c542bed830307fb941111111254eeb25477b68fb85ed929f73a9605828719532b70e8dceab9068812aa3caf0000000000000000000000008c864d0c8e476bf9eb9d620c10e1296fb0e2f940000000000000000000000000eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee00000000000000000000000066761fa41377003622aee3c7675fc7b5c1c2fac50000000000000000000000008c864d0c8e476bf9eb9d620c10e1296fb0e2f940000000000000000000000000436c3a6a4eda9cd5b2af1a49c7cc8e383f702f640000000000000000000000000000000000000000000000000019532b70e8dcea000000000000000000000000000000000000000000000015f03155838db8ced2000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000001400000000000000000000000000000000000000000000000000000000000000160000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000004e80000000000000000000000000000000000000000000000000004ca00004e00a0744c8c0900000000000000000000000000000000000000005aafc1f252d544f744d17a4e734afd6efc47ede4000000000000000000000000000000000000000000000000000040d4ea16cf02416066a9893cc07d91d95644aedd05d03f95e1dba8af02e424856bc30000000000000000000000000000000000000000000000000000000000000040000000000000000000000000000000000000000000000000000000000000008000000000000000000000000000000000000000000000000000000000000000011000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000100000000000000000000000000000000000000000000000000000000000000200000000000000000000000000000000000000000000000000000000000000380000000000000000000000000000000000000000000000000000000000000004000000000000000000000000000000000000000000000000000000000000000800000000000000000000000000000000000000000000000000000000000000003060b0e00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000003000000000000000000000000000000000000000000000000000000000000006000000000000000000000000000000000000000000000000000000000000001e0000000000000000000000000000000000000000000000000000000000000026000000000000000000000000000000000000000000000000000000000000001600000000000000000000000000000000000000000000000000000000000000020000000000000000000000000000000000000000000000000000000000000000000000000000000000000000066761fa41377003622aee3c7675fc7b5c1c2fac5000000000000000000000000000000000000000000000000000000000000271000000000000000000000000000000000000000000000000000000000000000c8000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000010000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000015f03155838db8ced2000000000000000000000000000000000000000000000000000000000000012000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000060000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000006000000000000000000000000066761fa41377003622aee3c7675fc7b5c1c2fac50000000000000000000000001111111254eeb25477b68fb85ed929f73a96058200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000009a635db5c001a00f83b01e25ae72ebe9daaf262b093cbc93da138f50aff20791eef017d38cfdc5a002012d558ba667979ae82880e36750f49b64c8fbe2c08cc0dbbf173b6de75970";

    println!("=== Parsing Real V4 Transaction ===\n");
    
    // Decode hex string
    let clean_hex = tx_hex.strip_prefix("0x").unwrap_or(tx_hex);
    let mut tx_bytes = match hex::decode(clean_hex) {
        Ok(bytes) => bytes,
        Err(e) => {
            println!("❌ Error decoding hex: {}", e);
            return;
        }
    };
    
    // Try to decode as signed transaction (TxEnvelope)
    let unsigned_tx = match TxEnvelope::decode(&mut tx_bytes.as_slice()) {
        Ok(envelope) => {
            println!("✅ Transaction decoded as signed transaction");
            match envelope {
                TxEnvelope::Eip1559(signed) => alloy_consensus::TypedTransaction::Eip1559(signed.tx().clone()),
                TxEnvelope::Eip2930(signed) => alloy_consensus::TypedTransaction::Eip2930(signed.tx().clone()),
                TxEnvelope::Eip4844(signed) => alloy_consensus::TypedTransaction::Eip4844(signed.tx().clone()),
                TxEnvelope::Legacy(signed) => alloy_consensus::TypedTransaction::Legacy(signed.tx().clone()),
                TxEnvelope::Eip7702(signed) => alloy_consensus::TypedTransaction::Eip7702(signed.tx().clone()),
            }
        }
        Err(e) => {
            println!("❌ Error decoding as signed transaction: {}", e);
            println!("This might be an unsigned transaction or invalid format");
            return;
        }
    };
    
    // Visualize the transaction
    match visualize_transaction(&unsigned_tx) {
        Ok(_) => println!("\n✅ Transaction visualized successfully!"),
        Err(e) => println!("❌ Error visualizing transaction: {}", e),
    }
}

fn visualize_transaction(tx: &alloy_consensus::TypedTransaction) -> Result<(), Box<dyn std::error::Error>> {
    println!("\nTransaction details:");
    println!("  Type: {:?}", tx.tx_type());
    if let Some(to) = tx.to() {
        println!("  To: {:?}", to);
    }
    if let Some(chain_id) = tx.chain_id() {
        println!("  Chain ID: {}", chain_id);
    }
    
    // Extract calldata
    let input = tx.input();
    println!("  Calldata length: {} bytes", input.len());
    if !input.is_empty() {
        println!("  Calldata prefix: 0x{}", hex::encode(&input[..std::cmp::min(20, input.len())]));
        
        // Try V4 PoolManager visualizer first
        if let Some(chain_id) = tx.chain_id() {
            let visualizer = V4PoolManagerVisualizer;
            println!("\n=== V4 PoolManager Visualization ===");
            if let Some(field) = visualizer.visualize_tx_commands(input, chain_id, None) {
                match serde_json::to_string_pretty(&field) {
                    Ok(json) => {
                        println!("{}", json);
                        return Ok(());
                    }
                    Err(e) => {
                        println!("Error serializing to JSON: {}", e);
                    }
                }
            } else {
                println!("V4 PoolManager visualizer did not match this calldata");
            }
        }
    }
    
    // Try full transaction visualization
    println!("\n=== Full Transaction Visualization ===");
    let options = VisualSignOptions::default();
    match transaction_to_visual_sign(tx.clone(), options) {
        Ok(payload) => {
            match serde_json::to_string_pretty(&payload) {
                Ok(json) => println!("{}", json),
                Err(e) => println!("Error serializing payload to JSON: {}", e),
            }
        }
        Err(e) => {
            println!("Error in full visualization: {}", e);
        }
    }
    
    Ok(())
}
