# Changelog

All notable changes to d2-rs are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and versions follow
[Semantic Versioning](https://semver.org/). Before 1.0, a minor version may
change public Rust APIs.

Each release names the upstream Go D2 commit it matches, from
[`reference-manifest.json`](reference-manifest.json).

## [Unreleased]

### Fixed

- `--isometric` GIFs are now byte-identical to Go's: they are encoded as Go's
  `image/gif` does (header, color table, frame blocks and LZW data), and a
  long GIF render no longer prints repeated `compiling & running layout
  algorithms...` lines after layout has finished.
- TALA: deeply nested diagrams no longer crash with a stack overflow. More
  than 256 levels of nesting, or more than 10,000 objects, now fail at once
  with Go's error instead of running for minutes.
- Windows: the CLI reserves an 8 MiB main-thread stack, so deeply nested
  diagrams no longer overflow Windows' 1 MiB default.
- Dagre: a grid cell whose label is blank (`" "`) keeps Go's label spacing;
  it was drawn 5 px too high.
- Sequence diagrams whose actors contain grids keep curved edges inside the
  grid, and TALA reports Go's error for their empty spans.
- `--tala-seeds` given more than once now adds to the list, as Go does, and
  accepts `0x`/`0o`/`0b` values. The `D2_TALA_SEEDS` environment variable,
  which Go ignores, no longer affects TALA.

## [0.1.8] - 2026-11-13

Matches Go D2 `master` at 6a7b48378, as 0.1.0 does. Crates, the npm
package, prebuilt binaries and Windows installers.

### Fixed

- TALA: a diagram with connected `shape: step` nodes and any grid, sequence
  diagram or `near` object no longer fails with "has no complete route".
- TALA: repeated edges between step shapes inside a container no longer fail
  with an internal invariant error.
- TALA: placement of container children, nested grids inside grid cells, and
  step sequences (position, labels, crossing counts) now matches Go.
- TALA: a step with an outside label or an icon position is rejected with
  Go's error instead of rendering.

### Changed

- Builds and CI use Rust 1.100.0. The minimum supported Rust version stays
  1.98.1. Dropped unused dependencies (`zune-jpeg`, `unicode-bidi-mirroring`)
  and the redundant `homepage` manifest field.

## [0.1.7] - 2026-11-11

Matches Go D2 `master` at 6a7b48378, as 0.1.0 does. Crates, the npm
package, prebuilt binaries and Windows installers.

### Fixed

Two new differential sweeps (`scripts/shape-trace-differential.py` and
`scripts/graph-differential.py`) render seeded ordinary diagrams with Go and
d2-rs on every engine. They found these differences from Go, now fixed:

- Ovals with only `width` or only `height` set ignored Go's 3:1 aspect limit
  on the label fit, so a long or tall label gave the wrong size.
- Dagre: opposing edges between one pair of nodes, one labeled, after an
  unrelated node, ordered their routes differently.
- Dagre: `label.near: center-left`/`center-right` on objects inside
  containers, and outside label positions on containers, lost Go's side
  spacing and shifted the diagram.
- ELK: labeled edges next to SQL-table ports were spaced 40 px apart
  instead of 5; edges into a grid cell with a label wider than the cell
  stopped short.
- Grid cells holding a 3D hexagon were 8 px taller on every engine.
- TALA: edges with only a source arrowhead were ranked backwards; labels
  that TALA scales up kept their old spacing; `near` objects used unscaled
  label sizes; edges crossing out of a root grid were routed before `near`
  objects and restored in the wrong order.

On 12 seeds x 300 generated diagrams, dagre and ELK now match Go on every
case on arm64 and x86-64, and TALA on 3,595 of 3,600.

### Changed

- Dagre and ELK clip edges with the Go-exact shape tracer that TALA uses;
  the older approximation is gone (no output change found).
- 101 oracle fixture outputs that nothing read since the legacy renderer
  was removed are retired, with reasons in `docs/fixture-retirements/`.

## [0.1.6] - 2026-11-11

Matches Go D2 `master` at 6a7b48378, as 0.1.0 does. Prebuilt binaries and
Windows installers only: the code is unchanged from 0.1.5, so the crates and
the npm package stay at 0.1.5.

### Added

- Windows installers on GitHub releases, `d2-rs-<version>-windows-amd64.msi`
  and `-windows-arm64.msi`. Unlike Go D2's per-machine MSI
  ([d2lang/d2#2941](https://github.com/d2lang/d2/issues/2941)), they install
  for the current user by default, with no administrator rights, into
  `%LOCALAPPDATA%\Programs\d2-rs` and the user `PATH`; choosing all users in
  the installer (or `ALLUSERS=1` with `msiexec /qn`) installs into
  `Program Files\d2-rs` and the system `PATH`. The release workflow builds them
  with WiX 7 from the packaged archives, checks every installed file against
  the archive, and installs and uninstalls each one in both scopes as a test.

## [0.1.5] - 2026-11-11

Matches Go D2 `master` at 6a7b48378, as 0.1.0 does. Crates, the npm
package and prebuilt binaries.

### Added

- Prebuilt `d2-rs` binaries on GitHub releases, for the platforms Go D2
  ships: Linux, macOS and Windows on amd64 and arm64. They are packaged like
  D2's archives (`d2-rs-<version>-<os>-<arch>.tar.gz`, with `bin/`, license,
  notices, README and a `make install` Makefile) and listed in `SHA256SUMS`.
  `.github/workflows/release.yml` builds them on every `v*` tag; v0.1.4 got
  them retroactively.
- Tag pushes no longer rerun the CI test matrix, which already ran on the
  tagged commit's branch push.

### Changed

- The native CLI is 24% smaller: 37.4 MB stripped on Linux arm64, down
  from 49.2 MB (Go's `d2` is 40.8 MB), and 43.1 MB on x86-64, down from
  56.6 MB. Release builds use fat LTO, stable sorts and graph lowering
  share code instead of copying it per comparator or text measurer, and
  PNG, PDF and PPTX are rendered from Go's scene path, which drops the
  earlier SVG renderer and the usvg/resvg stack from the binary. Output
  is unchanged; PNG, PDF and PPTX export is up to 25% faster.
- `D2_RS_SVG_RENDERER=legacy` no longer selects the earlier native SVG
  renderer; SVG output always uses the d2svg port, as it has by default.

### Fixed

- TALA on x86-64: edges ending on non-rectangular shapes (cloud,
  cylinder, person and others) traced the shape border with fused
  multiply-add, as on arm64; they now follow Go's unfused amd64
  arithmetic.
- TALA: a panic while routing edges returns Go's "TALA edge routing
  failed due to an internal invariant" error instead of unwinding.

### Changed

- `d2-rs --version`, `d2-rs version` and the `--help` header report d2-rs's
  own release (such as `v0.1.5`) instead of the Go D2 version it matches
  (`v0.9.0`). Output metadata (SVG `data-d2-version`, PNG EXIF) still carries
  `v0.9.0`, so rendered files stay byte-identical with Go's.

## [0.1.4] - 2026-11-09

Matches Go D2 `master` at 6a7b48378, as 0.1.0 does. Crates only; the npm
package stays at 0.1.3, which is unaffected.

### Security

- `zip` is now only a test dependency of `d2-rs-export`, so published
  crates no longer depend on it (0.1.3 required `zip` 6.0.0, which has
  published security advisories, though d2-rs never called it at runtime: PPTX output
  uses the Go-ported writer).
- `rustls` 0.23.46 for remote icon fetching.

## [0.1.3] - 2026-11-09

Matches Go D2 `master` at 6a7b48378, as 0.1.0 does.

### Fixed

- Isometric SVG export was up to 50x slower than Go on grid-heavy boards
  (Fulcro RAD 6.6 s against Go's 0.4 s). Numbers are formatted in one pass
  and grid cells share the document's assets instead of copying them, as
  in Go; every isometric board in the benchmark now runs faster than Go.
- TALA: the 400-node fan-out error path, long labels and fan-out memory are
  now at or near Go (1.07x, 1.00x and 1.005x), and the 80,000-edge board
  uses 10% less memory. Placement trees no longer search the whole tree
  per node, and the TALA call site borrows the graph instead of copying
  every edge.
- About 40 layout and rendering differences found by fuzzing every engine
  and output format against Go, including:
  - Markdown: empty headings missing from PDF, raw inline HTML, HTML tag
    names, form feeds in labels and a parser hang.
  - Text: zero-width joiners and mixed-script labels in PNG, PDF and PPTX;
    small Unicode and emoji runs in PNG.
  - Sketch mode in PNG: ellipses, shadows and multiline class labels.
  - TALA: nested and wrapper containers, ancestor edges, cloud and circle
    containers, grids, classes and routing.
  - ELK: self-loops inside containers and on SQL table columns, zero-height
    grids, nested sequence diagrams and cloud shapes.
  - Dagre: edges leaving a container toward the same node, and several PNG
    and SVG layouts.
  - Class cycles report the first back-edge only; the CLI prints Go's
    `info:` line before compile errors.
- Layouts found by checking every small graph against Go:
  - Dagre: children of a container that are each fed from outside no
    longer overlap (`p -> k.a` with `q -> k.b`); parallel labeled edges
    with a self-loop inside a container keep their labels next to the
    nodes. Over 500,000 enumerated small graphs match Go.
  - ELK: edges crossing nested containers, parallel labeled edges with a
    self-loop, content bounds, compound and root loops and negative
    coordinates. All 2,750,800 canonical five-node graphs with three or
    four edges match Go.
  - TALA: dense graphs (an 80-node network, 40 nodes in six containers),
    hierarchy eligibility, clustered containers and nested routes. The
    80-node network with `--tala-seeds 3` now takes 7.1 s against Go's
    9.2 s (16.3 s before).
- Negative `width` and `height` are rejected as in Go; a capitalised
  reserved key such as `Shape:` gives Go's errors.
- Custom fonts: `--sketch` measures labels with the user's font, each
  role keeps its own face in PNG, and Markdown uses custom faces as in Go.
- PNG: gradient edge strokes and gradient font colors in sketch mode and
  dark themes match Go.
- Output files get Go's permission bits for every format.
- npm package: one copy of the WebAssembly payload instead of two (30 MB
  smaller), loaded once in Node; correct types for Node CommonJS and ESM
  and for `@d2-rs/wasm/raw`. It no longer ships stale files from earlier
  builds.
- Published crates no longer include internal test fixtures; README links
  work on crates.io and npm, and the README names the crates to use as a
  Rust library.

### Known issues

- TALA: per-seed routing memory is still slightly above Go's, a 400-node
  fan-out reaches its error at about 1.07x Go's wall time, and an
  80,000-edge board peaks at about 1.33x Go's memory.
- The native CLI binary is about 1.2x the size of Go's.

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
  seed routes at once; long labels at 100 nodes run about 1.22× Go wall; a
  400-node fan-out takes 0.36 s to reach the same error Go reports in 0.04 s.
- The native CLI binary is about 1.2× the size of Go's.

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
[0.1.8]: https://crates.io/crates/d2-rs/0.1.8
[0.1.7]: https://crates.io/crates/d2-rs/0.1.7
[0.1.6]: https://github.com/nelsonjchen/d2-rs/releases/tag/v0.1.6
[0.1.5]: https://crates.io/crates/d2-rs/0.1.5
[0.1.4]: https://crates.io/crates/d2-rs/0.1.4
[0.1.3]: https://crates.io/crates/d2-rs/0.1.3
[0.1.2]: https://www.npmjs.com/package/@d2-rs/wasm/v/0.1.2
[0.1.1]: https://crates.io/crates/d2-rs/0.1.1
[0.1.0]: https://crates.io/crates/d2-rs/0.1.0
