// SPDX-License-Identifier: MPL-2.0
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn main() {
    use d2_rs_language::stream::{ReadError, open_native};
    let paths: Vec<String> = serde_json::from_reader(std::io::stdin()).unwrap();
    let results: Vec<_> = paths
        .iter()
        .map(|path| match open_native(path) {
            Err(error) => serde_json::json!({"open_error":error}),
            Ok(mut file) => {
                let mut bytes = [0; 4096];
                let (n, error) = file.read_bytes(&mut bytes);
                let message = match error {
                    None => String::new(),
                    Some(ReadError::Eof) => "EOF".into(),
                    Some(ReadError::Other(s)) => s,
                };
                serde_json::json!({"bytes":bytes[..n],"read_error":message})
            }
        })
        .collect();
    println!("{}", serde_json::to_string(&results).unwrap());
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn main() {
    panic!("native file oracle not supported on this platform");
}
