// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! One generic combine-work profile on stdin, one raw JSON object on stdout.

#[cfg(feature = "diagnostic-traces")]
fn main() -> std::process::ExitCode {
    use std::io::{Read, Write};

    let mut input = String::new();
    if let Err(error) = std::io::stdin().read_to_string(&mut input) {
        eprintln!("could not read combine-work probe request: {error}");
        return std::process::ExitCode::FAILURE;
    }
    let request = match serde_json::from_str(&input) {
        Ok(request) => request,
        Err(error) => {
            eprintln!("invalid JSON combine-work probe request: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };
    let output = d2_rs_tala::diagnostic::diagnostic_combine_work_probe(request);
    let mut stdout = std::io::stdout().lock();
    if let Err(error) = serde_json::to_writer(&mut stdout, &output)
        .and_then(|()| stdout.write_all(b"\n").map_err(serde_json::Error::io))
    {
        eprintln!("could not write combine-work probe result: {error}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}

#[cfg(not(feature = "diagnostic-traces"))]
fn main() -> std::process::ExitCode {
    eprintln!("tala_combine_work_probe requires the diagnostic-traces feature");
    std::process::ExitCode::FAILURE
}
