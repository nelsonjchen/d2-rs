# TALA RNG diagnostics

RNG diagnostics compile only with the `diagnostic-traces` feature. Ordinary
and release builds omit the observers, their state, the argument formatting
and the primitive draw counter. Unit tests keep the counter to check the Go
stream contract; without `diagnostic-traces`, compatibility snapshots report
zero draws.

Build the native CLI with symbols for breakpoints and watchpoints:

```sh
cargo build -p d2-rs --profile diagnostic --features diagnostic-traces
```

- `D2RS_TRACE_RNG_OPERATIONS=1` records each semantic `Float64` result as
  IEEE-754 hex bits, each `Int63` result, and each shuffle's start, end and
  every swap. Events carry the stream identity and the primitive draw count;
  `Float64` records also keep the count before the call, so resampling is
  visible. A shuffle interrupted by a work or cancellation error emits an
  incomplete end.
- `TALA_TRACE_RNG_JSONL=1` adds the primitive draws, initialization and
  graph-scope records.

The Go side needs the same events from an instrumented build.
`tools/patches/d2-v090-rng-operation-trace.patch` adds
`D2_TRACE_RNG_OPERATIONS=1` and `TALA_TRACE_RNG_JSONL=1` to Go; it was written
against v0.9.0 and is not in the manifest's `trace_patches`, so rebase it
onto the pin before applying it to `../d2-trace`. Compare the two event
streams in their actual order, with exact values and float bits; do not sort
streams or normalize away repeat variation.

Matching RNG events does not by itself establish candidate, score,
traversal, transaction, routing or seed-race parity. Assess individual seeds
and repeated runs apart from concurrent seed races.
