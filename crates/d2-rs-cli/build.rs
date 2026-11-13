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
    // Go grows the main goroutine's stack on demand. Windows gives a process
    // main thread 1 MiB, which deep ELK and dagre nesting (up to their 1024
    // object limit) overflows; reserve the 8 MiB that Linux and macOS give
    // the main thread by default. The reservation is virtual until touched.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        match std::env::var("CARGO_CFG_TARGET_ENV").as_deref() {
            Ok("msvc") => println!("cargo:rustc-link-arg-bins=/STACK:8388608"),
            Ok("gnu") => println!("cargo:rustc-link-arg-bins=-Wl,--stack,8388608"),
            _ => {}
        }
    }
}
