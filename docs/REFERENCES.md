# Durable reference setup

`../d2` is detached at the manifest's exact v0.9.0 commit; its existing master
branch is preserved. The reproducible stage-trace oracle uses the dedicated
`../d2-v090-trace-oracle` worktree at that commit. The existing
`../d2-v0.9.0-trace` worktree contains additional local D2 investigations and
is preserved separately. All authoritative artifacts live under `~/code`.

```sh
uv run python scripts/references.py fetch
uv run python scripts/references.py verify
uv run python scripts/references.py dependencies
uv run python scripts/build-oracle.py
GOTOOLCHAIN=go1.27.0 go run tools/inventory/main.go
cargo build --release --locked -p d2-rs-lib --examples
uv run python scripts/check-boundaries.py
uv run python scripts/check-source-graph.py
uv run python scripts/check-oracle-equivalence.py
```

`references.py fetch` now verifies the macOS ARM64 v0.9.0 release archive and
extracts the official `d2` binary under `.reference/releases/`. The source
oracle is rebuilt with Go 1.27.0 and `-X github.com/d2lang/d2/lib/version.Version=v0.9.0`,
so version-sensitive CLI checks use the same identity as the release asset.
The release and source binaries are compared separately; matching geometry from
one does not substitute for the other.
`check-oracle-equivalence.py` records exact SVG/PNG bytes and decoded pixels for
the five renderer fixtures in `oracle-cache/reports/oracle-equivalence.json`.

The strict boundary checker currently passes **176/176** applicable comparisons
(87 successful layout runs plus the source-position cases, with exact JSON
fields and IEEE-754 numeric bits) and records one documented pre-measure
inapplicable source. Keep this check strict: a regenerated baseline is
evidence only when the checker exits successfully. The source-position checker
covers valid Unicode input; malformed encodings and remaining AST/parser
behavior are not yet implemented.

`check-source-graph.py` compares the pinned Go 1.27.0 `measure` projection
against the native `graph` command before layout. The current report covers all
79 `.d2` fixtures in the pinned v0.9.0 checkout with exact node dimensions,
label dimensions, shape normalization, hierarchy order, direction, and edge
identity/arrow semantics (`oracle-cache/reports/source-graph-parity.json`).

For the separate trace checkout, apply the tracked stage-only patch once:

```sh
git -C ../d2-v090-trace-oracle apply "$PWD/tools/patches/d2-v090-stage-trace.patch"
uv run python scripts/build-oracle.py --trace
uv run python scripts/check-instrumentation.py
```

The trace builder checks the complete modified source against the tracked patch
using a disposable Git index. It rejects untracked source and unexpected edits.
The diagnostic Rust oracle and the patched Go oracle can then be compared with:

```sh
cargo build --release --locked -p d2-rs-lib --example tala-oracle
cargo build --profile diagnostic --locked -p d2-rs-lib \
  --features diagnostic-traces --example tala-oracle
uv run python scripts/check-layout-parity.py --seed 123 \
  --report docs/layout-trace-parity.json
```

The current tracked report covers 12 representative source fixtures and
matches both normalized event streams and public layout bytes. The report is
still a qualification slice; it does not claim full language, rendering, or
export parity.

The local oracle supports parse, format, compile, measure, layout and position.
Its measure operation uses theme 0 and default fonts on the compiled graph; it
is an isolated diagnostic boundary, not the full CLI's configuration/board
orchestration. Rust production code never invokes this executable.

Official-release benchmarks:

```sh
uv run python scripts/benchmark.py --output docs/performance-baseline.json \
  tests/d2/direction-right.d2 tests/d2/features.d2 tests/d2/benchmark-medium.d2
```

Run timing experiments after builds/tests finish. The native harness measures
Rust parse/graph/layout/render stages beside the official v0.9.0 binary, with
explicit seed 1. It records the compiler from the Go binary's build metadata
(`go1.27.0`) separately from the host Go version, so a downloaded toolchain
cannot be mistaken for the runtime under test. Direct-engine timing, plugin
overhead, and memory profiling remain separate work; these numbers must not be
attributed entirely to IPC.

The SVG target's built-in pattern assets are copied from the pinned D2 v0.9.0
renderer. Their source checksums are:

```text
crates/d2-rs-render/assets/grain.png        7aec37844fa033cbdb586efaa1955dc6f43d56707c0b1bf91e8b5e32386d8d69
crates/d2-rs-render/assets/paper.txt.br     fe047f4104975b17b033988e49981c42a5aca928da39d149e35f4e31c8fb248a
crates/d2-rs-render/assets/paper.svg        98b3c110441c6c02cb685c0a7303e7a36c1faee7fa1f65f6424b48ec51123527
crates/d2-rs-render/assets/streak_path.txt 7b4001e7d043ac395abd015ce0bc2bbe018725a634f6e352b1eb2ff60cc99dea
```

`paper.svg` is the deterministic decompression of `paper.txt.br`; both are
retained so the acquisition can be independently checked. The Rust renderer
uses a stable local pattern suffix instead of D2's per-diagram hash.
