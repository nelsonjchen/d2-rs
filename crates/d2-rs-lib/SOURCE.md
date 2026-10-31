# Corresponding source

This is an unpublished development checkout of d2-rs. There is not yet a public
release source URL. Keep this complete checkout, its Git revision and all local
modifications alongside any locally shared binary or WASM artifact. Do not
substitute the archived Weftan repository for the source of d2-rs modifications.

The upstream source and historical Rust input are pinned in
reference-manifest.json. Development references can be reconstructed with
scripts/references.py and docs/REFERENCES.md. Instrumentation patches are tracked
under tools/patches; ignored references must not contain undisclosed changes.

The source-derived math table in `crates/d2-rs-math/src/glyphs.rs` is generated
from `github.com/d2lang/mathjax-go v0.1.0/internal/font/data_normal_gen.go`,
the MathJax 3.2.2 TeX font data used by the pinned D2 release. Reproduce it
with `uv run --no-project python scripts/generate-math-glyphs.py
/path/to/mathjax-go/internal/font/data_normal_gen.go
crates/d2-rs-math/src/glyphs.rs` before extending the subset. This process uses
the verified local source and does not execute JavaScript or a browser.

Before publication, supply a verified accessible source URL and exact source
revision for every artifact. Include LICENSE, THIRD_PARTY_NOTICES.md,
the repository root's DEPENDENCY_NOTICES.md and this file with every binary/WASM distribution and
crate. Source recipients must receive the MPL-covered modifications needed to
build their version. Third-party material retains its applicable terms.

## Master URL script codec (Lane L15)

`src/urlenc.rs` exposes `encode`/`decode` from pinned master
`lib/urlenc/urlenc.go:15–62` (6a7b48378). Encode uses the shared Go 1.27
BestCompression port and padded URL base64. Decode checks literal, stored
block and back-reference growth against the 16 MiB uncompressed limit before
publishing a result; an oversize stream returns the complete upstream error.
The native base64/Huffman reader reuses the existing Go-derived translation
in `d2-rs-wasm/src/url_codec.rs`; Go BSD notices remain in this crate's
THIRD_PARTY_NOTICES.md. The WASM decoder is left for a later cross-scope
consolidation onto this native API; this change does not qualify its size bound.

`tests/master_urlenc.rs` replays both original calls from
`TestDecodeRejectsOversizedScript` with Go assertions active, complete encoded
bytes and exact errors. Its compact fixture reconstructs the original repeated
input by count and SHA-256; the original raw capture remains in L15 evidence.
Unit tests reuse all 987 existing source decoder fixtures, including malformed
streams, and exercise exact-boundary and other compression-level inputs.

## Master library configuration (Lane L26)

`src/config.rs` translates `d2lib/d2.go:108–140,289–354` at master
6a7b48378. `apply_configs`, `apply_defaults` and `resolve_config` preserve
caller option presence (including false, zero and empty palette pointers),
return effective options and feed appearance-sensitive values back into the
source config. Native values own their nested data. This configuration API
does not yet implement the Go library's layout-resolver callbacks or reuse.
`tests/master_config.rs` binds ten observed original config/default calls,
plus independent ownership, explicit-empty and sketch/font contracts.

## Master library source admission (Lane L27)

`src/source.rs` maps `d2lib/d2.go:26–47,68–106` and the parser/IR
forwarding in `d2compiler/compile.go:64–99` at master 6a7b48378. The
library boundary forwards input path, UTF-16 positions, all four expansion
budgets and the explicit import opener. The same cancellation token reaches
parsing and IR compilation; failures preserve the typed context cause and
ordered diagnostics. No host-filesystem fallback is supplied. `parse_source`
retains the partial syntax tree as `d2parser/parse.go:52–100` does.

`compile_source_ir` ends at IR; graph lowering, config, measurement, layout,
resolver callbacks and rendering are separate APIs. No partial IR escapes on
error. The seven original calls in six `d2lib/d2_test.go:18–91` functions
compare full errors, cancellation categories and the complete partial parse
tree. The observation adapter rejects successful full-Compile results rather
than claiming to implement graph/layout/render through this source API.
`tests/master_source.rs` also verifies error-chain typing, cancellation during
an import, and explicitly enabled imports. Fixture paths are diagnostic input
labels from the original capture; tests do not open those host paths.
