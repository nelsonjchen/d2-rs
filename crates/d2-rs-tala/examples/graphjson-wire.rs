// SPDX-License-Identifier: MPL-2.0
//! GraphJSON wire representation and preflight check.
fn run() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{Read, Write};
    let mut data = Vec::new();
    std::io::stdin().read_to_end(&mut data)?;
    if std::env::args().nth(1).as_deref() == Some("--preflight") {
        return d2_rs_tala::graphjson::graphjson_preflight(&data);
    }
    let output = d2_rs_tala::graphjson::graphjson_wire_roundtrip(&data)?;
    std::io::stdout().write_all(&output)?;
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
