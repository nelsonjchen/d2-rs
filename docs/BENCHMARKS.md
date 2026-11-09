# d2-rs vs Go D2: benchmarks

This doc gives the speed and memory of d2-rs against Go D2, how they were
measured, and the caveats. Every timed row also compares the outputs.

Go D2 here is upstream `master` at 6a7b48378, built with Go 1.27.0,
including its bundled TALA engine. A row only counts if both produced the
same SVG.

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
- **Host:** Linux arm64, 16 vCPUs, idle.

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

Reproduce (needs the Go oracle and the e2e case dump from
[REFERENCES.md](REFERENCES.md)):

```sh
cargo build --release --locked -p d2-rs
uv run python scripts/perf-bench.py --rust target/release/d2-rs \
  --output perf.json --repeat 5
```

## WebAssembly in Node

**Harness:** `scripts/wasm-vs-go/` (see its [README](../scripts/wasm-vs-go/README.md)).
- **Go side:** `d2js`, built at the pin with `GOOS=js GOARCH=wasm` and the
  same version stamp, driven through `d2.compile` / `d2.render`.
- **d2-rs side:** its `D2` facade.
- Both run in-process with no workers, in Node 22, and each measurement runs
  in a fresh process.
- It uses the same 12 cases × 3 engines, and reports the median of 3 runs
  after one warm-up.

| Engine | median wall | total wall | median peak RSS |
|---|---:|---:|---:|
| Dagre | 0.30x | 0.31x | 0.33x |
| ELK | 0.29x | 0.33x | 0.32x |
| TALA | 0.40x | 0.27x | 0.61x |

| | Go `d2js` | d2-rs |
|---|---:|---:|
| Module size, raw / gzip -9 | 60.0 / 15.3 MB | 31.3 / 10.1 MB |
| Startup (instantiate + init) | ~405 ms | ~43 ms |
| TALA `tpmjs_architecture` | 58.6 s | 15.3 s |
| TALA `arrowhead_scaling` | 29.6 s | 8.2 s |

TALA's lead is larger in WASM than natively. In WASM, Go runs its goroutines on
one thread, so the concurrent TALA seeds lose their parallelism; d2-rs also
runs them one after another there, but its single-thread speed carries over.

## Whole corpus, every format

perf-bench's 12 SVG cases missed the export formats, which were once 2x Go
on the median. `scripts/corpus-perf-sweep.py` now covers every e2e case:
SVG with all three engines, plus PNG, PDF, PPTX and GIF (125 ms frames)
through Dagre, 2,086 jobs. Pass 1 times each job once per implementation;
every row over Go is then re-timed with 3 interleaved runs (medians).
Remote-icon cases are skipped, as in perf-bench. Both CLIs write the same
output file name, and PNG, PPTX and GIF bytes equal Go's on every case
where Go writes output (PDF embeds Go's wall-clock date).

Rust/Go ratios on Linux arm64 (AWS Graviton4, quiet
host; every row over Go re-timed with 5 interleaved runs):

| Output | median wall | median peak RSS | rows slower or larger than Go |
|---|---:|---:|---:|
| SVG, Dagre | 0.21x | 0.60x | 0 of 291 |
| SVG, ELK | 0.21x | 0.60x | 0 of 296 |
| SVG, TALA | 0.47x | 0.68x | 0 of 296 |
| PNG | 0.46x | 0.82x | 0 of 291 |
| PDF | 0.45x | 0.74x | 0 of 284 |
| PPTX | 0.44x | 0.75x | 0 of 287 |
| GIF | 0.15x | 0.55x | 0 of 252 |

No corpus row is over Go. One PNG board (`real_world/leios_simulator`)
sits at about 1.01x Go's peak RSS in some runs and under it in others. The
x86-64 corpus sweep was last run on an earlier build; every change since was gated
on x86-64 for output identity and both test tiers, not re-timed there.

Reproduce (Go oracle and e2e case dump as for perf-bench; about 15 minutes):

```sh
cargo build --release --locked -p d2-rs
uv run python scripts/corpus-perf-sweep.py --rust target/release/d2-rs \
  --output corpus.json
```

## Generated scale sweep

`scripts/scale-sweep.py` measures deterministic chains, fan-out trees, grids,
sparse and dense graphs, nested containers, sequence diagrams, SQL tables
and long-label boards at 25, 50, 100, 200, 400 and 800 nodes. It runs SVG
with Dagre, ELK and TALA, plus Dagre PNG/PDF. Each row records median wall,
CPU and peak RSS from three interleaved runs per binary. SVG and PNG must
match Go in every sample; PDF bytes are not comparable. Reports include
per-family tables and flag ratios that grow with size, including ratios
still below Go. On a shared host, use CPU and peak RSS to decide which
costs to investigate; confirm wall time on a quiet host.

```sh
uv run --no-project python scripts/scale-sweep.py \
  --output oracle-cache/reports/scale-sweep/scale.json
```

The sweep enforces a 300 s timeout per invocation, a 128 MiB per-file limit
and a 1 GiB cumulative capture budget. A Go timeout is a recorded skip; a
Rust timeout when Go finishes fails. Budget exhaustion stops the capture.
Matching Go budget rejections are recorded as skips; Rust must reject the
same input with the same error before the row can be skipped.
Measurement-worker failures retain their exit status and diagnostics in a
failed row; the remaining queue still runs. Unavailable measurements stay null.
Use `--families`, `--sizes`, `--engines` and `--formats` to narrow subsequent
runs. Generated inputs and output samples stay beside the report in the
ignored cache. The scoreboard exits nonzero for output differences, failed
runs, exceeded budgets, or performance boards above Go or rising with size.

Raster stress families are opt-in: `markdown`, `emoji`, `opacity`, `gradient`,
`code-block`, `shadow` and `labeled-edges`. For example, use
`--families markdown,emoji,opacity,gradient --sizes 10,40,160,480 --engines dagre --formats png`.
Here size counts list items, code lines, painted nodes or labeled edges.

### Results

Ratios are wall / CPU / peak RSS against Go, quiet arm64. Every Dagre and
ELK family is under Go at every size where Go produces output: the worst are
ELK sparse at 400 nodes (1.23 wall, 0.99 CPU) and Dagre SQL at 800 (1.08
RSS). Sequence diagrams reach 0.93-0.94 wall and about 1.0 RSS at 800 actors.
TALA is the open engine at scale: fan-out at 200-400 nodes is 1.5-1.75x wall
with CPU near or under Go (TALA's parallel seeds overlap less), long labels
at 100 nodes reach 1.15x RSS, and nested at 100 nodes uses 2.24x Go's peak
RSS. Larger TALA boards hit TALA's work limits in both implementations.

Other generated sweeps (`raster-stress.py`, `feature-stress.py`,
`registry-sweep.py`, `fuzz-differential.py`) and their open findings are
listed in [STATUS.md](STATUS.md).

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

## Caveats

- **Hosts:** native timing is Linux only: arm64 for perf-bench, arm64 and
  x86-64 for the corpus sweep. macOS was not benchmarked.
- **Architecture:** Go fuses multiply-add on arm64 but not on x86-64, so
  last-bit float output differs between them. d2-rs mirrors that per
  architecture; each architecture's output is compared with Go built for it
  ([FIXTURES.md](FIXTURES.md#host-architecture)).
- **WASM:** measured in Node only, not in browsers.
- **Go settings:** Go runs with default settings (no `GOGC` or `GOMAXPROCS`
  tuning).
- **Allocator:** d2-rs's native CLI uses mimalloc with a 2 ms purge delay.
  The WASM build uses the default dlmalloc.
- **Representativeness:** both harnesses use upstream's own e2e corpus, not
  real-world diagrams; perf-bench is a fixed 12-case sample of it.
