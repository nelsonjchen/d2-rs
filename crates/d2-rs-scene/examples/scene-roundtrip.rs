// SPDX-License-Identifier: MPL-2.0
//! Decodes Go probe Document JSON files and re-encodes them, so a caller can
//! check that the Rust model loses nothing (asset file references kept).

fn main() -> Result<(), Box<dyn std::error::Error>> {
    for path in std::env::args().skip(1) {
        let document: d2_rs_scene::Document = serde_json::from_slice(&std::fs::read(&path)?)?;
        std::fs::write(format!("{path}.rs.json"), serde_json::to_vec(&document)?)?;
    }
    Ok(())
}
