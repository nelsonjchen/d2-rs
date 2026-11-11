# Port map

Where each upstream Go D2 package lives in d2-rs, at the pinned commit in
`reference-manifest.json`. Paths are under `crates/`. Every package that
affects output is ported; the few that aren't are listed first.

## Not ported (no effect on output)

| Go package | Why |
|---|---|
| `d2chaos` | random-diagram generator for upstream's own tests |
| `lib/env` | reads `TEST_MODE`/`DEV_MODE`/`DEBUG`; Rust uses `std::env` where needed |
| `lib/version` | one version constant; the `data-d2-version` marker is set in the SVG output path |
| `lib/simplelog` | logging adapter; replaced by `d2-rs-cli/src/cli_log.rs` |
| `lib/syncmap` | generic `sync.Map` wrapper; Rust standard types replace it |
| `internal/testlog`, `internal/testutil`, `testutil/imagediff` | test helpers; the Python harnesses in `scripts/` play this role |

## Language and library

| Go package | Rust |
|---|---|
| `d2ast` | `d2-rs-core` |
| `d2parser`, `d2ir`, `d2compiler`, `d2format` | `d2-rs-language` |
| `d2graph` | `d2-rs-core` (graph types) and `d2-rs-language` |
| `d2oracle` | `d2-rs-editor/src/{edit,graph_edit,boards,imports}.rs` |
| `d2lsp` | `d2-rs-editor/src/{completion,references}.rs` (upstream has no LSP transport either) |
| `d2lib` | `d2-rs-lib`, `d2-rs-layout` |
| `lib/urlenc` | `d2-rs-lib/src/urlenc.rs` |
| `d2exporter` | `d2-rs-render` (graph to diagram) |
| `d2target` | `d2-rs-target` |
| `d2themes`, `d2themes/d2themescatalog` | `d2-rs-target/src/theme.rs` |

## Layout

| Go package | Rust |
|---|---|
| `d2layouts`, `d2grid`, `d2near`, `d2sequence` | `d2-rs-layout/src/{lib,grid,near,sequence}.rs` |
| `d2layouts/internal/layoutguard` | `d2-rs-core/src/layout_guard.rs` |
| `d2dagrelayout` (and `dagro`) | `d2-rs-dagre` |
| `d2elklayout` (and `elk-go`) | `d2-rs-elk` |
| `d2talalayout` and its 22 `internal/*` packages | `d2-rs-tala/src/internal/*`, one module per Go package |
| `internal/d2layoutfeatures` | `d2-rs-cli/src/layout_features.rs` |

## Rendering

| Go package | Rust |
|---|---|
| `d2renderers/d2svg`, `d2svg/appendix` | `d2-rs-render/src/d2svg/` |
| `d2renderers/d2sketch` (and `rough-go`) | `d2-rs-render`, `d2-rs-sketch` |
| `d2renderers/d2animate` | `d2-rs-render/src/target.rs` |
| `d2renderers/d2scene` | `d2-rs-scene` |
| `d2renderers/d2scenebuild` | `d2-rs-scenebuild` |
| `d2renderers/d2svgimport` | `d2-rs-scenebuild/src/svgimport_*.rs`, `d2-rs-export/src/svg_import*.rs` |
| `d2renderers/d2raster`, `internal/scanline` | `d2-rs-raster` |
| `d2renderers/d2isometric`, `d2isometricimg` | `d2-rs-isometric` |
| `d2renderers/d2latex` (and `mathjax-go`) | `d2-rs-math`, `d2-rs-render/src/d2svg/latex.rs` |
| `d2renderers/d2fonts` | `d2-rs-render`, `d2-rs-text` |
| `d2renderers/d2ascii` (`asciicanvas`, `asciishapes`, `asciiroute`, `charset`) | `d2-rs-export/src/ascii_target.rs` |
| `d2renderers/internal/fontface` | `d2-rs-fontface` |
| `d2renderers/internal/fontfallback` | `d2-rs-scenebuild/src/fallback.rs` |
| `d2renderers/internal/patternassets` | `d2-rs-render/src/patternassets.rs` |
| `d2renderers/internal/scenepaint` | `d2-rs-scenebuild/src/color.rs` |
| `internal/rasterimage` | `d2-rs-raster/src/first_frame.rs`, `d2-rs-math/src/svg_image_config.rs` |

## Support libraries

| Go package | Rust |
|---|---|
| `lib/background`, `lib/svg` | `d2-rs-render` |
| `lib/color` | `d2-rs-target/src/theme.rs`, `d2-rs-render` |
| `lib/compression`, `lib/font` | `d2-rs-text/src/{woff,subset_font}.rs`, `d2-rs-goflate` |
| `lib/geo` | `d2-rs-core`, `d2-rs-render/src/d2svg/geo.rs` |
| `lib/imageasset`, `lib/localfile`, `lib/netpolicy` | `d2-rs-scenebuild/src/{image_local,image_http_native,netpolicy_client,image_cache}.rs` |
| `lib/imgbundler` | `d2-rs-render/src/image_mime*.rs`, `d2-rs-cli/src/imgbundler.rs` |
| `lib/label` | `d2-rs-layout` |
| `lib/log` | `d2-rs-cli/src/cli_log.rs` |
| `lib/time` | `d2-rs-cli/src/deadline.rs` |
| `lib/pdf` | `d2-rs-export/src/{pdf_document,pdf_font}.rs` |
| `lib/png` | `d2-rs-export/src/{go_png,png_image}.rs` |
| `lib/pptx` | `d2-rs-export/src/{pptx,go_zip}.rs` |
| `lib/xgif` | `d2-rs-export/src/gif_quantize.rs` |
| `lib/shape` | `d2-rs-render/src/d2svg/shape.rs` |
| `lib/textmeasure` | `d2-rs-text/src/measure.rs`, `d2-rs-render/src/markdown_scene.rs` |

## Front ends

| Go package | Rust |
|---|---|
| `.` (`main.go`), `d2cli` | `d2-rs-cli` (fmt, validate, layout, play, watch, isometric, multi-board and raster output) |
| `d2js`, `d2js/d2wasm`, `lib/memfs` | `d2-rs-wasm` |
