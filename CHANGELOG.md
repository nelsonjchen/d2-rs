# Changelog

All notable changes to d2-rs are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Before 1.0, a minor version may
change public Rust APIs.

Each release names the upstream Go D2 commit it matches, from
[`reference-manifest.json`](reference-manifest.json).

## [Unreleased]

## [0.1.2] - 2026-11-07

### Changed

- Documentation only: the README and the npm package README give the
  `cargo install d2-rs` and `npm install @d2-rs/wasm` commands and the
  current known issues. No code changes from 0.1.1.

## [0.1.1] - 2026-11-07

Matches Go D2 `master` at 6a7b48378, as 0.1.0 does.

### Fixed

- Windows: the default test tier passes and all 944 e2e boards match Go on
  native MSVC and under Wine. Imports across drives, path spelling in
  diagnostics and Windows file errors follow Go, and Go-matching math
  replaces C library results that differ on Windows. Files with CRLF line
  endings render as in Go.
- TALA: boards with many edges ran up to 1.7x slower than Go because seeds
  waited for each other to route; every seed now routes at once, as in Go.
  Parallel edges, nested boards and class diagrams use less CPU or memory,
  and the 200-node nested work-limit error matches Go.
- PNG: right-to-left override text, short `cylinder`, `callout` and
  `c4-person` shapes, `--scale=0.05`, and malformed PNG icons (now rejected
  as Go does) match Go.
- Layout and error cases found by fuzzing: glob-applied circle containers,
  `near` on the root board, repeated grid dimensions, inline HTML in
  Markdown, TALA containers with explicit widths, nested sequence diagrams,
  four LaTeX line-break cases and several error messages.
- Faster PNG export of shadows, double borders, gradients and long Markdown.

### Known issues

- TALA: a 200-node fan-out peaks at about 1.05× Go's memory now that every
  seed routes at once; long labels at 100 nodes run about 1.26× Go wall; a
  400-node fan-out takes 0.36 s to reach the same error Go reports in 0.04 s.
- The native CLI binary is about 1.2× the size of Go's.
- CI builds Windows but does not run its tests.
- Four LaTeX inputs and three fuzz singletons from an earlier sweep were not
  kept and could not be replayed; their variants match Go.

## [0.1.0] - 2026-11-06

### Added

- First release: a Rust port of the D2 toolchain, matching Go D2 `master` at
  6a7b48378 (2026-10-11), including its bundled TALA layout engine.
  - The compiler, formatter and editor helpers (`d2oracle`, `d2lsp`).
  - The Dagre, ELK and TALA layout engines, grids, sequence diagrams and
    `near` placement.
  - SVG, sketch and isometric rendering; PNG, PDF, PPTX, GIF and ASCII
    export; Markdown, code highlighting in every Chroma language, and LaTeX.
  - The `d2-rs` command line (`cargo install d2-rs`) and the `@d2-rs/wasm`
    WebAssembly package for Node and browsers.
- Byte-identical output with the pinned Go D2 on all 944 upstream e2e golden
  boards, upstream's `e2etests-cli` suite, the oracle test tier on arm64 and
  x86-64, and the Go-vs-Rust differential harnesses.
- Faster than Go D2 with lower peak memory on every upstream e2e case in
  every output format ([benchmarks](docs/BENCHMARKS.md)).

### Known issues

- Gaps found beyond upstream's test corpus, all listed in
  [STATUS.md](docs/STATUS.md#beyond-the-upstream-corpus): right-to-left
  text, combining marks and short `cylinder`, `callout` and `c4-person`
  shapes in PNG; `--scale=0.05` in PNG; a few error-message details; four
  LaTeX inputs and one Markdown case; TALA's time and memory on large
  boards.
- The native CLI binary is about 1.2× the size of Go's.
- Windows builds but is not qualified against Go: imports across drives,
  path separators in diagnostics and one TALA layout differ in tests.

[Unreleased]: https://github.com/nelsonjchen/d2-rs/commits/main
[0.1.2]: https://crates.io/crates/d2-rs/0.1.2
[0.1.1]: https://crates.io/crates/d2-rs/0.1.1
[0.1.0]: https://crates.io/crates/d2-rs/0.1.0
