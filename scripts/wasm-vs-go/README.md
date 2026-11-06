# d2-rs WASM vs Go D2 WASM

These scripts compare the d2-rs WASM package with upstream Go D2's own WASM
build (`d2js`) in Node. Both are driven in-process, with no workers: Go through
the raw `d2.compile` and `d2.render`, d2-rs through its `D2` facade. Each
measurement runs in a fresh process and reports:

- startup time;
- compile+render CPU and wall time;
- peak RSS;
- every timed SVG's sha256, so byte identity is checked on every sample.

The driver alternates Go/Rust order between repeats. Each sample uses a fresh
process with one untimed warm-up; startup is recorded separately. Failed calls,
missing SVGs, unstable bytes and mismatches fail the row. The Go module must
contain clean build metadata for the manifest's commit and toolchain and report
the pinned version stamp. Module and case hashes must remain unchanged during
the capture.

## Build

```sh
# d2-rs package (wasm32 release + wasm-bindgen 0.2.128)
(cd crates/d2-rs-wasm && node ./scripts/build-wasm.mjs)
# Go d2js at the pinned commit, stamped with the same version string d2-rs reports
(cd ../d2 && GOOS=js GOARCH=wasm go build \
  -ldflags="-s -w -X github.com/d2lang/d2/lib/version.Version=v0.9.0" \
  -trimpath -o /tmp/go-main.wasm ./d2js)
```

## Run

```sh
uv run --no-project python scripts/wasm-vs-go/export-cases.py /tmp/cases.json
export CASES=/tmp/cases.json GO_WASM=/tmp/go-main.wasm \
  GO_EXEC="$(go env GOROOT)/lib/wasm/wasm_exec.js" \
  RS_DIR="$PWD/crates/d2-rs-wasm"
node scripts/wasm-vs-go/bench.mjs 3 dagre,elk,tala '' /tmp/wasm-perf.json
```

The optional fourth argument selects comma-separated case indices; an empty
argument selects every case. The optional fifth argument sets the report path
(default `bench-results.json` in the working directory). Reports include every
sample and per-engine medians and worst ratios. The command exits nonzero for
incorrect output or any wall median over Go. Set `METRIC=cpu` to gate on the
recorded process CPU medians instead (these include V8 background work).
`WARMUP` sets the number of untimed calls (default 1); `SETTLE_MS` waits after
warmup before timing (default 0), useful for separating V8 tiering CPU from
steady-state compile/render CPU. Both settings are recorded in the report.

## Public API differential scoreboard

`corpus.py` imports the upstream inputs and mutation rules from
`../differential-scoreboard.py`. Layer mutations target the layer they add.
The fixed WASM witnesses follow the sampled jobs.

Build the Rust Node package with `pnpm --dir crates/d2-rs-wasm build`.
Prepare a Go JS package from the pinned `d2js` source, stamped with
`-ldflags='-s -w -X github.com/d2lang/d2/lib/version.Version=v0.9.0'`.
Pass its `index.js` with `--go-package` and the pinned native CLI with `--go`.
The default package location is `oracle-cache/reports/wasm-differential/go-package/`.

Run `node scripts/wasm-differential-scoreboard.mjs --cap 200 --go <Go CLI>`.
`--rust-package` accepts an existing Node package's `src/node.cjs`.
For a private runtime copy, use `--rust-source-root` to name its source checkout.
Package hashes must stay unchanged for the whole capture.
Each runtime runs in its own child process. Reports contain source revisions,
package hashes, options, bounded mismatch values and exact diff paths.
Only SVG version attributes are normalized. Mismatches, timeouts, invalid
responses, process failures and exhausted budgets produce a nonzero exit.

Run harness checks with `node scripts/wasm-differential-scoreboard.mjs --self-test`
and `uv run --no-project python -m unittest discover -s scripts -p test_differential_scoreboard.py`.

The widened CLI matrix requires a shared clock for PDF metadata and warning
logs. Build a clock oracle with `scripts/build-cli-clock-oracle.py`, then run:

```sh
uv run --no-project python scripts/differential-scoreboard.py \
  --go <clock-oracle> --epoch 946782245 --cap 2000
```

The cap bounds sampled rendering jobs. In addition, every corpus input and
sampled mutation runs through `fmt`, `fmt --check` and `validate` in isolated
filesystems. The matrix samples SVG, PNG, PDF, PPTX, GIF, ASCII and stdout,
with bounded line minimization in the report. `--baseline-matrix` retains the
original rendering options and omits command jobs. Both harnesses sample
API options including dark theme, scale, center, real layer targets, ASCII
and isometric rendering. Full output bytes remain the comparison contract.
