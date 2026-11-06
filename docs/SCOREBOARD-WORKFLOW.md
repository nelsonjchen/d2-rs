# Scoreboard workflow

This doc lists the scoreboards that measure d2-rs against upstream Go D2,
how to run each one, and what counts as passing. Work is chosen by these
scoreboards: fix whatever breaks the most of them, and never regress one.

All of them compare against the Go D2 pinned in `reference-manifest.json`.
Set up the references first (`docs/REFERENCES.md`). Python scripts run with
uv; reports default to the gitignored `oracle-cache/reports/` or
`.reference/` and are never committed.

Build the CLI once for the scoreboards that run it:

```sh
cargo build --release --locked -p d2-rs
```

If other builds may run while a scoreboard is scoring, score a copy of the
binary so it cannot change mid-run.

## The scoreboards

### e2e goldens

```sh
uv run --no-project python scripts/e2e-scoreboard.py --rust target/release/d2-rs --output e2e.json
```

Renders every upstream `e2etests` case with each engine upstream tests
(`--engines dagre,elk,tala`) and compares the SVG with upstream's golden.
A board passes as `svg-match` (SVG bytes equal after three normalizations:
the XML declaration, the version stamp and the trailing newline) or
`error-match` (the expected error). `target-match` means only D2's hash of
the target diagram matched. Crashes, timeouts and missing output fail;
`inapplicable-*` and `no-golden` rows are reported apart and never pass.
Current: 944 of 944 (933 SVGs, 11 errors). `--filter` and `--jobs` narrow or
speed up a run; `--go <oracle>` also scores the Go CLI through the same
harness, which rules out harness artifacts.

On Linux, `scripts/windows-e2e-scoreboard.py --go <pinned-Windows-Go.exe>`
compares the complete output file inventory from Windows Go and Rust under
Wine. Build Rust with `cargo build --locked --profile diagnostic -p d2-rs
--target x86_64-pc-windows-gnu`; cross-build Go from the manifest's checkout
with `GOOS=windows GOARCH=amd64` and the manifest's version stamp. The script
checks the Go commit, toolchain, architecture and version, and reports errors,
timeouts and missing child outputs as failures. `--wine` selects a Wine loader;
`WINEPREFIX` selects its configuration. Reports and failing witnesses stay in
`oracle-cache/reports/`. The Windows comparison normalizes only version stamps.

### Oracle test tier

```sh
scripts/test-oracle.sh
```

Regenerates the Go-oracle fixtures, then runs `cargo test` with every
crate's `oracle-fixtures` feature. These tests compare internal results
(parser and IR output, layout geometry, Markdown, math, rasters, exporters)
with Go's at each stage. Passing means every test passes; a missing fixture
fails. See `docs/FIXTURES.md`, including how to run it per architecture.

### Differential CLI

```sh
uv run --no-project python scripts/differential-scoreboard.py --rust target/release/d2-rs
```

Runs Go and Rust on upstream `.d2` files and test scripts, each with one
seeded mutation (`--seed`, `--cap` cases), and compares exit code, stdout,
stderr and output bytes. A case passes as `match`. Mismatches are printed
grouped by cluster; Go crashes or timeouts, unscored outputs and budget
exhaustion are counted apart, never as matches. A few mismatch clusters
remain open.

For a fresh search, add `--expanded-matrix --seed 20261103` and use a
clock oracle (`scripts/build-cli-clock-oracle.py`, `--go <clock-oracle>
--epoch 946782245`). This mode preserves the older queues and adds all
bundled themes, both dark themes, pad/center/scale/sketch sweeps, deeper
containers, relative near, grids, table/class shapes, Markdown, LaTeX and
icons. Each original and mutated source runs on Dagre, ELK and TALA;
`--cap` truncates this queue, so use at least six times the corpus size to
cover every input. Exports and layer targets/animation are sampled. Reports
include outcome counts per mutation family and command. The capture limit
remains 1 GiB total and 128 MiB per file.

### Differential WASM

```sh
node scripts/wasm-differential-scoreboard.mjs --seed 20261102 --cap 2000 --timeout 30000
```

Compares the d2-rs WASM package with Go's `d2js` through their public Node
APIs: the complete compile response and render result, including errors,
each case in a fresh process. Build both packages first, as described in
`scripts/wasm-vs-go/README.md`. A case passes only if both results are
deeply equal; timeouts and crashes are recorded and never match. Results go
to `oracle-cache/reports/wasm-differential/`. A few mismatch clusters remain
open.

### CLI suite

```sh
uv run --no-project --with pillow python scripts/cli-scoreboard.py \
  --rust target/release/d2-rs --capture <fresh-dir> --output cli.json
```

Runs upstream's `e2etests-cli` test bodies against the pinned Go CLI, then
the same bodies against d2-rs, using a hook installed in `../d2-trace`. Two
tiers: the test assertions, and the retained goldens. A golden counts only
if the pinned Go reproduces it too. SVG uses the e2e normalizations; PNG and
GIF compare decoded pixels and frames; PDF and PPTX compare exact bytes.
Skipped, unsupported, timed-out and missing results never pass. Current: all
67 applicable assertions pass, and 51 of 66 goldens (every one the pinned
Go still reproduces) match. `--capture` must be a fresh directory; `--reuse`
rescores a completed capture.

### Performance

```sh
uv run python scripts/perf-bench.py --rust target/release/d2-rs --output perf.json --repeat 5
```

Times 12 fixed upstream e2e cases with each engine against the pinned Go CLI
and compares the outputs byte for byte. A row passes when the outputs are
identical and d2-rs's wall time and peak RSS are below Go's (ratio < 1.0).

```sh
uv run python scripts/corpus-perf-sweep.py --rust target/release/d2-rs --output corpus.json
```

The release gate for speed: every e2e case with every engine as SVG, plus
PNG, PDF, PPTX and GIF, about 2,000 jobs. perf-bench alone once hid export
formats running at twice Go's time. Every row it prints as slower or larger
than Go is a failing board. Method and current results: `docs/BENCHMARKS.md`.

### Beyond the corpus

Upstream's corpus is small, well-formed and uses few code languages, so
passing it hid real gaps. These sweeps generate inputs instead and compare
the outputs, and for some the time and memory, with the pinned Go CLI:

- `scripts/scale-sweep.py`: layout families from 25 to 800 nodes on every
  engine. A row fails when it is over Go or its ratio grows with size.
- `scripts/raster-stress.py` and `scripts/feature-stress.py`: one raster or
  language feature at a time, at growing size.
- `scripts/registry-sweep.py`: every shape, label position, style keyword,
  arrowhead and CLI flag as small boards.
- `scripts/chroma-differential.py`: one snippet per Chroma lexer.
- `scripts/fuzz-differential.py`, then `scripts/fuzz-minimize.py` on each
  witness: random structural mutations of corpus inputs.

Each distinct mismatch cluster is a failing board. Open findings are in
`docs/STATUS.md`, "Beyond the upstream corpus".

## Checking a change

Score before and after the change, then diff the two reports. The delta
must show zero regressions:

```sh
uv run --no-project python scripts/e2e-scoreboard-delta.py before.json after.json
uv run --no-project python scripts/cli-scoreboard-delta.py before.json after.json
```

## Finding the next fix

1. Group the remaining failures by the field or cluster that differs. A
   field that fails the same way on Dagre, ELK and TALA is a renderer or
   exporter bug, and fixing it pays three times.
2. Diff one board in detail. This needs a CLI built with
   `--features diagnostic-traces`:

   ```sh
   cargo build --profile diagnostic -p d2-rs --features diagnostic-traces
   cp target/diagnostic/d2-rs target/diagnostic/d2-rs-traces
   uv run --no-project python scripts/e2e-target-diff.py stable/direction --engine dagre
   ```
3. Reduce a failing input to a minimal witness while the symptom holds:

   ```sh
   uv run --no-project python scripts/reduce-d2-witness.py in.d2 -- \
     sh -c 'target/release/d2-rs --layout=tala "$0" out.svg 2>&1 | grep -q "<symptom>"'
   ```
4. Check that the pinned Go CLI behaves correctly on the witness. Then
   compare the Rust code with the Go source and fix the earliest divergence.
   Never special-case a fixture's output.
