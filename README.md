# d2-rs

A Rust port of [D2](https://github.com/terrastruct/d2), the diagram scripting
language. It includes the Dagre, ELK and TALA layout engines, all running in
process. It builds as a native CLI and as a WebAssembly package.

d2-rs targets byte-identical output with upstream Go D2 `master` at the commit
pinned in [`reference-manifest.json`](reference-manifest.json) (6a7b48378,
2026-10-11). Upstream's own test suites are the oracle.

## Status

**Parity:** all 944 of upstream's e2e golden boards match: 933 SVGs plus 11
expected errors, across Dagre, ELK and TALA. The comparison is byte for byte,
except for three things: the XML declaration, the version stamp and the
trailing newline.

**Speed and memory:**
- The native CLI beats Go D2 on wall time and peak RSS on all 36 benchmark rows
  (12 cases × 3 engines), with identical output on every row.
  - Dagre and ELK take about a third of Go's time on the median row.
  - TALA takes about 0.7×.
- The WebAssembly build beats Go's `d2js` on all 36 rows, also with identical
  SVGs.
  - It takes 0.27–0.33× Go's total time.
  - The module is less than half the size and starts in about a tenth of the
    time.

See [benchmarks](docs/BENCHMARKS.md) for the method and caveats.

**Not done yet:**
- **CLI:** a few artifacts in upstream's CLI-test suite (`e2etests-cli`) still
  differ.
- **Platforms:** parity and benchmarks are qualified on arm64 (Apple Silicon,
  Linux arm64) and Node. Go's float output differs slightly between arm64 and
  x86-64 (FMA fusion). d2-rs mirrors this per architecture, but x86-64 parity
  has not been measured yet ([details](docs/FIXTURES.md#host-architecture)).
- **Releases:** none yet.

External layout plugins (`d2plugin-*`) are out of scope; upstream master has
removed them too.

## Use

```sh
cargo build --release --locked -p d2-rs-cli
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

Most of the code was written by AI coding agents (Anthropic's Claude and
OpenAI's Codex), under the scoreboard workflow in
[docs/SCOREBOARD-WORKFLOW.md](docs/SCOREBOARD-WORKFLOW.md).

## Repository map

| Path | What |
|---|---|
| `crates/d2-rs-language` | parser, IR and compiler |
| `crates/d2-rs-lib`, `d2-rs-layout` | graph lowering, engine selection |
| `crates/d2-tala-rs`, `d2-rs-dagre`, `d2-rs-elk` | layout engines |
| `crates/d2-rs-render`, `d2-rs-export`, `d2-rs-raster` | SVG renderer and export formats |
| `crates/d2-rs-cli` | the `d2-rs` binary |
| `crates/d2-rs-wasm` | WebAssembly package |
| `crates/d2-rs-editor` | completions and source-edit helpers (D2's `d2oracle`/`d2lsp`) |
| `scripts/` | scoreboards, benchmarks, reference tooling |
| `docs/` | [plan](docs/PLAN.md), [status](docs/STATUS.md), [references](docs/REFERENCES.md), [provenance](docs/PROVENANCE.md) |

## Testing

```sh
cargo test --workspace --locked
uv run python scripts/references.py verify
# needs the pinned e2e case dump; see docs/REFERENCES.md
uv run python scripts/e2e-scoreboard.py --rust target/release/d2-rs --output e2e.json
```

Normal builds compile diagnostic tracing out. It is enabled with
`--features diagnostic-traces`.

## License

MPL-2.0, the same license as D2. This is a derivative work of D2, Copyright
2022 Terrastruct Inc. Third-party material keeps its notices. See
[THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and
[DEPENDENCY_NOTICES.md](DEPENDENCY_NOTICES.md).
