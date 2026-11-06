// SPDX-License-Identifier: MPL-2.0
fn main() {
    let include = std::env::var_os("DEP_MIMALLOC_INCLUDE_DIR")
        .expect("libmimalloc-sys must provide its matching mimalloc header");
    println!("cargo:rerun-if-changed=src/allocator.c");
    println!("cargo:rerun-if-env-changed=DEP_MIMALLOC_INCLUDE_DIR");
    cc::Build::new()
        .include(include)
        .file("src/allocator.c")
        .compile("d2_rs_allocator");
}
