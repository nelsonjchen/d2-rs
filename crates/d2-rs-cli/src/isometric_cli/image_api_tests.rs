// SPDX-License-Identifier: MPL-2.0
//! Input-only transport for the original isometricImageOptions calls.
use super::*;

#[test]
#[ignore = "executed explicitly by cli-mode-scoreboard.py --suite image"]
fn replay() {
    let requests: Vec<serde_json::Value> = serde_json::from_slice(
        &std::fs::read(std::env::var_os("D2RS_IMAGE_REQUEST").unwrap()).unwrap(),
    )
    .unwrap();
    let results: Vec<_> = requests.iter().map(|input| {
            let extension = input["format"].as_str().unwrap();
            let name = extension.strip_prefix('.').unwrap();
            let format = match name {
            "svg" => ExportFormat::Svg,
            "png" => ExportFormat::Png,
            "gif" => ExportFormat::Gif,
            _ => panic!("unexpected original format"),
        };
        let scale = f64::from_bits(u64::from_str_radix(input["scale_bits"].as_str().unwrap(), 16).unwrap());
        let ((width, height), result) = image_dimensions(format, scale);
        serde_json::json!({"format":name, "width":width, "height":height, "error":result.err().unwrap_or_default()})
    }).collect();
    let path = std::env::var_os("D2RS_IMAGE_RESPONSE").unwrap();
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    serde_json::to_writer(&mut output, &results).unwrap();
}
