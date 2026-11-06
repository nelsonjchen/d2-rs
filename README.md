# d2-rs

A Rust port of the [D2](https://github.com/terrastruct/d2) toolchain: the
compiler, the Dagre, ELK and TALA layout engines, the renderers and exporters,
and the `d2` command line, for the D2 diagram scripting language. Everything
runs in process, and it builds as a native CLI and as a WebAssembly package.

d2-rs targets byte-identical output with upstream Go D2 `master` at the commit
pinned in [`reference-manifest.json`](reference-manifest.json) (6a7b48378,
2026-10-11). Upstream's own test suites are the oracle.

## Status

**Parity:** all 944 of upstream's e2e golden boards match byte for byte
(933 SVGs and 11 expected errors) across Dagre, ELK and TALA, apart from the
XML declaration, the version stamp and the trailing newline, which the goldens
differ in by how they were generated.

**Speed and memory:** across every e2e case and output format, d2-rs is
faster and smaller than Go D2 on the median, with output identical to Go's.
Rust/Go ratios on Linux arm64, lower is better:

| Output | Median wall time | Median peak RSS | Rows slower than Go |
|---|---:|---:|---:|
| SVG, Dagre / ELK | 0.21× / 0.21× | 0.60× / 0.60× | 0 of ~295 each |
| SVG, TALA | 0.47× | 0.68× | 0 of 296 |
| PNG / PDF / PPTX / GIF | 0.46× / 0.45× / 0.44× / 0.15× | 0.55–0.82× | 0 per format |
| WebAssembly vs `d2js` (total, 36 rows) | 0.27–0.33× | 0.32–0.61× | 0 of 36 |

The WebAssembly module is about half the size of `d2js` (31.3 MB against
60.0 MB uncompressed, 10.1 against 15.3 MB gzipped) and starts in about a
tenth of the time. The native CLI is the other way round: stripped on Linux
arm64, it is 48.9 MB against Go's 40.8 MB (1.20×). See
[benchmarks](docs/BENCHMARKS.md) for x86-64, the generated scale sweep, the
method and caveats.

**CLI:** all 67 applicable assertions in upstream's `e2etests-cli` suite
pass, and every retained golden that the pinned Go still reproduces (51 of
66) matches byte for byte.

**Platforms:** parity is qualified on arm64 (Apple Silicon, Linux arm64) and
x86-64 (Linux), each against Go built for that architecture, since Go's float
output differs slightly between them
([details](docs/FIXTURES.md#host-architecture)). Benchmarks were measured on
arm64 and in Node.

**Differential runs:** the pinned Go CLI and d2-rs render the same upstream
inputs and mutations of them under the same options; all 5,024 original jobs
match. The only exceptions are 5 mutated inputs that crash the pinned Go
itself, where d2-rs reports an error instead.

**Known gaps at the extremes:** upstream's tests use small, well-formed
diagrams and few code languages. Generated sweeps beyond them cover very
large graphs, extreme sizes and scales, every shape, flag and Chroma lexer,
and random mutations. They found and fixed many gaps; these are still open
([details](docs/STATUS.md#beyond-the-upstream-corpus)):

- **PNG only** (the SVG matches): right-to-left text, combining marks and
  multi-code-point emoji; `cylinder`, `callout` and `c4-person` shapes 25-31
  px tall; `--scale=0.05`.
- **Errors:** d2-rs renders a malformed PNG icon that Go rejects, escapes a
  right-to-left override character in an error message that Go prints raw,
  and names a different stage when a 200-node nested board exceeds TALA's
  work limit in both.
- **Markdown and LaTeX:** a tab-indented empty heading inside a block, and
  four LaTeX inputs where a style declaration spans a line break.
- **Fuzz singletons:** duplicated `grid-rows`, `*: suspend` globs, a
  `package` shape four levels deep, Gujarati next to an emoji.
- **Speed at scale:** TALA on large boards is slower or larger than Go
  (fan-out at 200-400 nodes 1.5-1.75× wall, nested at 100 nodes 2.24× peak
  RSS, 100 parallel edges 1.24× CPU), and a few raster stress cases run
  1.04-1.33× Go (a shadowed container with 60 children, 240 double-bordered
  shapes, 240 gradient fills, long Markdown). WebAssembly speed beyond the
  36-row benchmark is unmeasured.

## Use

```sh
cargo build --release --locked -p d2-rs
target/release/d2-rs input.d2 output.svg
target/release/d2-rs --layout=tala input.d2 output.svg
```

The CLI follows D2's command line:
- `fmt`, `validate`, `--theme`, `--sketch`, `--pad` and `--scale`;
- multiboard output;
- PNG, PDF, PPTX, GIF and ASCII export.

### WebAssembly

```sh
cd crates/d2-rs-wasm && node ./scripts/build-wasm.mjs
```

The package in `crates/d2-rs-wasm` has a `D2` class with the same
`compile` / `render` shape as D2.js. It ships builds for both Node and browsers.

## How it was built

The port started with static comparison against the Go source. A divergence
was fixed at its earliest point, never by matching a fixture's output.
Scoreboards choose the work:
- `scripts/e2e-scoreboard.py` covers upstream's e2e goldens;
- other scripts cover raster, CLI and language suites;
- `scripts/perf-bench.py` covers speed and memory.

Every change must keep every scoreboard unchanged. Go D2 is a test oracle
only: no Go code is linked into d2-rs.

Most of the code was written by AI coding agents, in two stages:
- **From 2026-09-09:** OpenAI's Codex, running GPT-6.1 Sol and GPT-6 Astra,
  started the port and worked on it alone.
- **From 2026-10-22:** Anthropic's Claude Opus 5.5 took the lead on the same
  codebase. It chose the work by scoreboard
  ([docs/SCOREBOARD-WORKFLOW.md](docs/SCOREBOARD-WORKFLOW.md)), wrote code
  itself, and supervised parallel Codex lanes running GPT-6.1 Sol, GPT-6.1
  Terra and GPT-6 Astra, reviewing and verifying their patches before each
  commit.

## Go behavior, ported

Byte-identical output rules out most Rust equivalents. A different deflate
encoder, float formatter, sort order or random generator changes the bytes,
even when the result looks the same. So beyond D2 itself, d2-rs ports the Go
code that D2's output depends on, matching each to its pinned Go source:

- **Go standard library:** `strconv` float parsing and formatting, `sort`,
  `math` (including its per-architecture routines) and `math/rand`,
  `compress/flate`, `compress/zlib`, `compress/lzw`, `hash/adler32`,
  `image/png`, `image/gif` and JPEG decoding, `archive/zip`, `encoding/xml`
  name rules, `encoding/json` string quoting, `net/url`, `html` and
  `unicode/utf8`.
- **Go libraries D2 uses:** `goldmark` (Markdown), `chroma` (code
  highlighting), `mathjax-go` (LaTeX), `go-text/typesetting` (text shaping,
  through a fork of Rustybuzz in `vendor/`), `rough-go` (sketch mode),
  `dagro` (Dagre), `elk-go` (ELK), Go-PDF/FPDF (PDF), `go-quantize` (GIF
  palettes), `csscolorparser`, `golang.org/x/net/html` name folding, the
  Brotli reader's errors, a TrueType font subsetter and Workiva's Fibonacci
  heap.
- **The Go compiler's arithmetic:** on arm64, Go fuses `x*y+z` into one
  rounding step, so d2-rs does too there, and plain arithmetic on x86-64.

Each port keeps its upstream license notice in
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).

These ports are useful beyond D2: any Go-to-Rust port that has to reproduce
Go's output exactly needs the same pieces. `crates/d2-rs-goflate` (Go's
`compress/flate` and `compress/zlib`) is already a standalone crate. Most
others are still modules inside the crates that needed them, for example
`d2-rs-core/src/go_float.rs` (`strconv`), `go_sort.rs` (`sort`),
`d2-rs-export/src/go_png.rs` and `go_zip.rs`, `d2-rs-elk/src/go_math.rs` and
`go_trig.rs`, and `d2-rs-tala/src/internal/runtime/go_rng.rs`
(`math/rand`). None of these crates is published, and there are no plans to
publish them yet.

## Repository map

The workspace mirrors upstream D2's package layout. Each crate ports the Go
packages named beside it; [docs/PORT-MAP.md](docs/PORT-MAP.md) maps every
upstream Go package to its Rust location.

**Language**

| Crate | Ports |
|---|---|
| `d2-rs-language` | `d2parser`, `d2ir`, `d2compiler`, `d2format`: parsing, compilation and formatting |
| `d2-rs-editor` | `d2oracle`, `d2lsp`: source edits and completions |
| `d2-rs-core` | `d2ast`, plus the shared graph, IR, geometry and layout result types |
| `d2-rs-lib` | the serialized graph JSON boundary between the compiler and the layout engines |

**Layout**

| Crate | Ports |
|---|---|
| `d2-rs-layout` | `d2layouts`: engine selection, grids, sequence diagrams, `near` placement |
| `d2-rs-tala` | TALA, the layout engine bundled in D2 |
| `d2-rs-dagre` | the Dagre adapter and `dagro` |
| `d2-rs-elk` | the ELK adapter and `elk-go` |

**Text and math**

| Crate | Ports |
|---|---|
| `d2-rs-text` | text measurement, Markdown (`goldmark`) and code highlighting (`chroma`) |
| `d2-rs-fontface` | D2's internal fontface package: parsing, shaping, glyph outlines, COLRv1 |
| `d2-rs-math` | `d2latex` and `mathjax-go`: LaTeX layout and SVG |

**Rendering**

| Crate | Ports |
|---|---|
| `d2-rs-target` | `d2target`: the diagram model and theme resolution |
| `d2-rs-scene` | `d2renderers/d2scene`: the renderer-neutral scene model |
| `d2-rs-scenebuild` | `d2renderers/d2scenebuild`: diagram to scene |
| `d2-rs-render` | `d2renderers/d2svg` and `d2renderers/d2sketch`: the SVG and sketch renderers |
| `d2-rs-sketch` | `rough-go`: the rough drawing primitives behind sketch mode |
| `d2-rs-isometric` | `d2renderers/d2isometric`: the isometric renderer |
| `d2-rs-raster` | `d2raster`: scene to pixels |

**Export and front ends**

| Crate | Ports |
|---|---|
| `d2-rs-export` | PNG, PDF, PPTX, GIF and ASCII export (`lib/png`, `lib/pdf`, `lib/pptx`, ...) |
| `d2-rs-goflate` | Go's `compress/flate` and `compress/zlib`, byte for byte |
| `d2-rs-cli` | `d2cli`: the `d2-rs` binary |
| `d2-rs-wasm` | `d2js`: the WebAssembly package |
| `d2-rs-testdata` | test-only access to the regenerated oracle fixtures |

**Elsewhere**

| Path | What |
|---|---|
| `vendor/` | `rustybuzz-go-text`: Rustybuzz 0.20.1 modified so text shaping matches Go's `go-text/typesetting` (GPOS pair adjustment, coverage digests, Unicode 17 property tables); see its [SOURCE.md](vendor/rustybuzz-go-text/SOURCE.md) |
| `scripts/` | scoreboards, differential harnesses, benchmarks, fixture runner |
| `tools/` | fixture recipes and hooks, upstream patches, oracle programs |
| `tests/` | hand-written D2 inputs |
| `docs/` | [status](docs/STATUS.md), [fixtures](docs/FIXTURES.md), [benchmarks](docs/BENCHMARKS.md), [references](docs/REFERENCES.md), [provenance](docs/PROVENANCE.md) |

## Testing

Plan for about 20 GB free. A debug test build of the workspace takes about
12 GB in `target/` and grows with rebuilds; the oracle tier adds the pinned
upstream checkout, Go's build cache and the regenerated fixtures, about 5 GB
together.

```sh
cargo test --workspace --locked
uv run python scripts/references.py verify
# needs the pinned e2e case dump (reference-manifest.json `e2e_cases`);
# see docs/SCOREBOARD-WORKFLOW.md
cargo build --locked --profile diagnostic -p d2-rs
uv run python scripts/e2e-scoreboard.py --output e2e.json
```

Normal builds compile diagnostic tracing out. It is enabled with
`--features diagnostic-traces`.

## License

MPL-2.0, the same license as D2. This is a derivative work of D2, Copyright
2022 Terrastruct Inc. Third-party material keeps its notices: see
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md). Cargo dependency notices
for distributions are generated with `node scripts/sync-licenses.mjs`; see
[SOURCE.md](SOURCE.md).
