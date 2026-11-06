// SPDX-License-Identifier: MPL-2.0
//! Replay native tessellation calls captured from the actual Go golden boards.
#[path = "support/primitive_protocol.rs"]
mod protocol;
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let calls: serde_json::Value = serde_json::from_slice(&std::fs::read(&args[0])?)?;
    let results: Result<Vec<_>, _> = calls
        .as_array()
        .ok_or("calls")?
        .iter()
        .map(protocol::replay)
        .collect();
    std::fs::write(&args[1], serde_json::to_vec(&results?)?)?;
    Ok(())
}
