// SPDX-License-Identifier: MPL-2.0
use std::io;
fn main() {
    let mut blocks: Vec<Vec<i32>> = serde_json::from_reader(io::stdin().lock()).unwrap();
    for block in &mut blocks {
        let mut raw: [i32; 64] = block.as_slice().try_into().unwrap();
        d2_rs_core::jpeg_idct::inverse(&mut raw);
        block.copy_from_slice(&raw);
    }
    serde_json::to_writer(io::stdout().lock(), &blocks).unwrap();
}
