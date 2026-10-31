// SPDX-License-Identifier: MPL-2.0
#[cfg(feature = "diagnostic-traces")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let request = serde_json::from_reader(std::io::stdin().lock())?;
    let result = d2_tala_rs::diagnostic::diagnostic_node_bounds_probe(request)?;
    serde_json::to_writer(std::io::stdout().lock(), &result)?;
    println!();
    Ok(())
}

#[cfg(not(feature = "diagnostic-traces"))]
fn main() -> std::process::ExitCode {
    eprintln!("tala_node_bounds_probe requires the diagnostic-traces feature");
    std::process::ExitCode::FAILURE
}
