// SPDX-License-Identifier: MPL-2.0
//! Decode/hash raw public target JSON, preserving duplicate keys.
use base64::{Engine as _, engine::general_purpose::STANDARD};
use d2_rs_render::{target::TargetDiagram, target_hash_id_from_input};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let ins: Vec<serde_json::Value> = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let mut rows = Vec::new();
    for input in ins {
        let name = &input["name"];
        let raw = if let Some(encoded) = input["raw_base64"].as_str() {
            STANDARD.decode(encoded)?
        } else {
            input["raw"].as_str().unwrap().as_bytes().to_vec()
        };
        let row = match TargetDiagram::decode_json(&raw) {
            Err(error) => {
                serde_json::json!({"name":name,"outcome":"decode-error","error":error.to_string()})
            }
            Ok(None) => serde_json::json!({"name":name,"outcome":"nil"}),
            Ok(Some(diagram)) => {
                let canonical = serde_json::to_string(&diagram)?;
                match diagram.hash_input() {
                    Ok(bytes) => {
                        serde_json::json!({"name":name,"outcome":"decoded","canonical":canonical,"hash_input":STANDARD.encode(&bytes),"hash":target_hash_id_from_input(&bytes,None)})
                    }
                    Err(error) => {
                        serde_json::json!({"name":name,"outcome":"hash-error","canonical":canonical,"error":error.to_string()})
                    }
                }
            }
        };
        rows.push(row);
    }
    std::fs::write(&args[2], serde_json::to_vec_pretty(&rows)?)?;
    println!("Native raw targets {}", rows.len());
    Ok(())
}
