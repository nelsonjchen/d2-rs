// SPDX-License-Identifier: MPL-2.0
use std::{io::BufRead, path::Path};

fn main() {
    for line in std::io::stdin().lock().lines() {
        let request: serde_json::Value = serde_json::from_str(&line.unwrap()).unwrap();
        let input = std::fs::read(request["input"].as_str().unwrap()).unwrap();
        let output = Path::new(request["out"].as_str().unwrap());
        std::fs::create_dir_all(output).unwrap();
        let result =
            d2_rs_raster::normalize_first_frame_png(&input, request["format"].as_str().unwrap());
        match result {
            Ok(png) => std::fs::write(output.join("actual.png"), png).unwrap(),
            Err(error) => std::fs::write(output.join("actual.err"), error).unwrap(),
        }
    }
}
