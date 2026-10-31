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

Default zigzag polygon filling in src/zigzag.rs follows pinned rough-go v0.2.0
filler_zigzag.go, filler_scan_line_hachure.go, geometry.go and the generator
outline-before-fill random stream. The gap derives from stroke width and the
hachure angle is the D2 default (-41 degrees). The renderer fixture
crates/d2-rs-render/tests/fixtures/sketch-zigzag-source-v090.json retains original
Go operation sets and typed scene paths for 34 polygon controls. Broader custom
filler options and complete scene work/cancellation accounting remain separate.

Canonical SVG continuation: src/svg_path.rs translates pinned rough-go v0.2.0
renderer.go::svgPath/bezierToCurrent. Absolute M/L/C/Z commands share one RNG;
cubic control/end perturbations retain source order, authored-current semantics
and two direct cubic passes. Open solid-line fills retain generator.Path's
outline-first RNG and roughness+0.8. This is distinct from native scene cubic
sampling. Original Go SVG/operation fixtures and repeats are retained under
.reference/sketch-svg-20261018 and the renderer's sketch-svg-source-v090.json.

d2sketch SVG generator: src/generator.rs translates the complete pinned rough-go
v0.2.0 Generator surface (core.go, generator.go, renderer.go, path.go,
number.go, math.go, geometry.go, filler*.go; only the module's unused
processSegment/bezierTo are omitted) and src/gomath.rs the Go 1.27.0 math
kernels it calls. FMA contractions follow `go build
-gcflags=github.com/d2lang/rough-go=-S` on darwin/arm64. The consumer is
crates/d2-rs-render/src/d2svg/sketch.rs (d2renderers/d2sketch/sketch.go).
Evidence: all 9 upstream sketch e2e goldens match byte-for-byte, and forcing
`config.sketch` (with and without fontFamily HandDrawn) on all 952 upstream
board.exp.json targets matches the pinned Go d2svg oracle on 947 boards (the
other 5 are pre-existing Go/Rust render errors unrelated to sketch).
