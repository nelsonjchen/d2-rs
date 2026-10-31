# d2-rs vs Go D2: benchmarks

d2-rs is compared with upstream Go D2 `master` at 6a7b48378, built with Go
1.27.0, including its bundled TALA engine. Every timed run also compares the
two outputs byte for byte. A row only counts if both produced the same SVG.

## Correctness first

All speed and memory numbers below are for identical output. The parity
gates come first:

- **e2e golden suite:** 944 of 944 upstream e2e boards match. That is 933
  SVGs plus 11 expected errors, across Dagre, ELK and TALA.
  - `scripts/e2e-scoreboard.py` compares the SVG bytes.
  - It normalizes only three things: the XML declaration, the
    `d2-version` stamp and the trailing newline.
- **Perf-bench:** compares raw bytes against the Go binary's own output. 36 of
  36 rows are identical, at every repeat.
- **WASM:** compares d2-rs's WASM SVG with Go `d2js`'s. 36 of 36 are identical.

## Native CLI

**Harness:** `scripts/perf-bench.py`.
- It takes 12 upstream e2e cases: the six largest plus six spread across the
  smaller ones. Each runs with all 3 engines, giving 36 rows.
- Each row runs `--repeat 5`, and the report gives medians.
- Peak RSS is the child process's `ru_maxrss`, measured one invocation at a
  time.
- **Host:** Linux arm64, a GCP c4a-standard-16, idle.
- **Commit:** `e1f1e13b`.

Rust/Go ratios (below 1.0 means d2-rs is faster or smaller):

| Engine | median wall | total wall | worst wall | median peak RSS | worst peak RSS |
|---|---:|---:|---:|---:|---:|
| Dagre | 0.34x | 0.52x | < 0.9x | 0.53x | < 0.9x |
| ELK | 0.34x | 0.46x | < 0.9x | 0.53x | < 0.9x |
| TALA | 0.71x | 0.88x | 0.92x | 0.59x | 0.95x |

No row is at or above 1.0x on wall time or on peak RSS.

The worst TALA row is `real_world/tpmjs_architecture`:
- wall time 3.08 s (Go 3.35 s);
- peak RSS 122 MB (Go 129 MB).

The cases are:
- `measured/empty-shape`, `regression/empty_nested_grid`, `stable/li2`;
- `stable/grid_edge_across_cell`, `stable/classes`, `patterns/real`;
- `stable/giant_markdown_test`, `stable/arrowhead_scaling`, `stable/grid_large_checkered`;
- `real_world/tpmjs_architecture`, `regression/grid_oom`, `real_world/spyre_encoder`.

Reproduce:

```sh
cargo build --release --locked -p d2-rs-cli
uv run python scripts/perf-bench.py --rust target/release/d2-rs \
  --output perf.json --repeat 5
```

## WebAssembly in Node

**Harness:** `scripts/wasm-vs-go/` (see its README).
- **Go side:** `d2js`, built at the pin with `GOOS=js GOARCH=wasm` and the
  same version stamp, driven through `d2.compile` / `d2.render`.
- **d2-rs side:** its `D2` facade.
- Both run in-process with no workers, in Node 22, and each measurement runs
  in a fresh process.
- It uses the same 12 cases × 3 engines, and reports the median of 3 runs
  after one warm-up.
- **Commit:** `508ce533`.

| Engine | median wall | total wall | median peak RSS |
|---|---:|---:|---:|
| Dagre | 0.30x | 0.31x | 0.33x |
| ELK | 0.29x | 0.33x | 0.32x |
| TALA | 0.40x | 0.27x | 0.61x |

| | Go `d2js` | d2-rs |
|---|---:|---:|
| Module size, raw / gzip -9 | 60.0 / 15.3 MB | 27.2 / 9.2 MB |
| Startup (instantiate + init) | ~405 ms | ~43 ms |
| TALA `tpmjs_architecture` | 58.6 s | 15.3 s |
| TALA `arrowhead_scaling` | 29.6 s | 8.2 s |

TALA's lead is larger in WASM than natively. In WASM, Go runs its goroutines on
one thread, so the concurrent TALA seeds lose their parallelism; d2-rs also
runs them one after another there, but its single-thread speed carries over.

## Where the speed comes from

The algorithms are not new. Every change is an exact refactor of a profile
hotspot, checked against the Go source and the scoreboards. The main sources:

- **Front end:** the CLI compiles and validates once instead of repeatedly.
  Font tables are sorted and served from static bytes. Markdown regexes are
  built lazily.
- **TALA placement and packing:** dense bitsets and hash sets replace general
  maps, and ancestry chains are cached while the topology is stable. The
  arena nodes have cache-line layouts.
- **TALA routing:** heap sifts move a hole instead of swapping. The search
  graph is a flat CSR adjacency table, with 8-byte queue entries and u32 slots.
  Search state is pooled per thread.
- **Memory:**
  - Each routing pass's freed memory goes back to the OS right away.
  - At most two TALA seeds route at once. This doesn't change the output:
    every seed still runs, and the winner is still chosen by score and then
    index.
  - On Linux the CLI turns off transparent huge pages for its own process.

`docs/STATUS.md` has the per-commit before → after history.

## Caveats

- **Hosts:** native timing is Linux arm64 only. macOS and x86-64 were not
  benchmarked.
- **Architecture:** byte-identity was checked against Go built for arm64.
  Go fuses multiply-add on arm64 but not on x86-64, so last-bit float output
  differs between them. d2-rs mirrors that per architecture, but its x86-64
  output has not been qualified against Go on amd64
  ([FIXTURES.md](FIXTURES.md#host-architecture)).
- **WASM:** measured in Node only, not in browsers.
- **Go settings:** Go runs with default settings (no `GOGC` or `GOMAXPROCS`
  tuning).
- **Allocator:** d2-rs's native CLI uses mimalloc with a 2 ms purge delay.
  The WASM build uses the default dlmalloc.
- **Representativeness:** 12 cases is a fixed sample of upstream's own e2e
  corpus, not of real-world diagrams.
