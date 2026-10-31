# d2-rs WASM vs Go D2 WASM

These scripts compare the d2-rs WASM package with upstream Go D2's own WASM
build (`d2js`) in Node. Both are driven in-process, with no workers: Go through
the raw `d2.compile` and `d2.render`, d2-rs through its `D2` facade. Each
measurement runs in a fresh process and reports:

- startup time;
- median compile+render time;
- peak RSS;
- the SVG's sha256, so byte identity is checked on every row.

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
node scripts/wasm-vs-go/bench.mjs 3 dagre,elk,tala
```

`bench.mjs` writes `bench-results.json` in the working directory.
