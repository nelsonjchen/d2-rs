// SPDX-License-Identifier: MPL-2.0
//! Decode and authenticate the bundled Noto Color Emoji font once, at build
//! time. Source d2fonts/noto_color_emoji_native.go decodes the Brotli asset
//! and checks both pinned SHA-256 digests on first use; doing that here keeps
//! the same guarantee (the build fails unless both digests match) without
//! repeating the work in every process.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::PathBuf;

const COMPRESSED_PATH: &str = "assets/NotoColorEmoji-COLRv1-v2.051.ttf.br";
const COMPRESSED_SIZE: usize = 2_354_437;
const COMPRESSED_SHA256: &str = "342beffe73f7fa450d2486d3ad62ab2308df68192718965baf23f5a85eef8247";
const DECODED_SIZE: usize = 4_991_984;
const DECODED_SHA256: &str = "0ae57fe58645638523ba35f388d93739d292539a9acb84df5700c81b1e1a28d2";

fn main() {
    println!("cargo:rerun-if-changed={COMPRESSED_PATH}");
    println!("cargo:rerun-if-changed=build.rs");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR"))
        .join("NotoColorEmoji-COLRv1-v2.051.ttf");
    // wasm builds do not bundle the face (noto_color_emoji_wasm.go).
    if std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() == Ok("wasm32") {
        std::fs::write(&out, []).expect("write empty bundled face");
        return;
    }
    let compressed = std::fs::read(COMPRESSED_PATH).expect("read bundled Noto Color Emoji");
    assert_eq!(compressed.len(), COMPRESSED_SIZE, "Brotli font size");
    assert_eq!(
        format!("{:x}", Sha256::digest(&compressed)),
        COMPRESSED_SHA256,
        "Brotli font SHA-256"
    );
    let mut decoded = Vec::with_capacity(DECODED_SIZE);
    brotli_decompressor::Decompressor::new(compressed.as_slice(), 4096)
        .take(DECODED_SIZE as u64 + 1)
        .read_to_end(&mut decoded)
        .expect("decode bundled Noto Color Emoji");
    assert_eq!(decoded.len(), DECODED_SIZE, "decoded font size");
    assert_eq!(
        format!("{:x}", Sha256::digest(&decoded)),
        DECODED_SHA256,
        "decoded font SHA-256"
    );
    std::fs::write(&out, decoded).expect("write decoded bundled face");
}
