# Exact RNG diagnostics

RNG diagnostics are compiled only with `diagnostic-traces`. Ordinary and release
builds omit the observers, their state and formatting/JSON argument construction.
The optimized `diagnostic` profile retains symbols for breakpoints and watchpoints.
Primitive draw accounting is also absent from ordinary builds. Unit tests retain
the counter to check the Go stream contract; compatibility snapshots require
`diagnostic-traces` for meaningful draw counts and otherwise report zero.

Build the native CLI:

```sh
cargo build -p d2-rs-cli --profile diagnostic --features diagnostic-traces
```

`D2RS_TRACE_RNG_OPERATIONS=1` captures semantic Float64 results as IEEE-754 hex
bits, direct Int63 results, shuffle start/end and every actual swap index/target.
Events include stream identity and primitive draw count. Float64 records also
retain the count before the call, so resampling is visible. Guarded shuffles emit
an incomplete end when a work/cancellation error interrupts the operation.

For the companion primitive draws, initialization and graph-scope records, enable
`WEFTAN_TRACE_RNG_JSONL=1`, the retained native diagnostic flag. On the instrumented
Go reference, the equivalent flags are `D2_TRACE_RNG_OPERATIONS=1` and
`TALA_TRACE_RNG_JSONL=1`. The Go operation patch is tracked in
`tools/patches/d2-v090-rng-operation-trace.patch`; build provenance is in
`docs/tala-rng-operation-oracle-build-20261002.json`. Go implementations remain
development oracles only.

The capture/gate script `tools/probes/tala-rng-operations-20261002.py` retains raw
stdout, stderr and SVG in a fresh suffix directory under `.reference/`. It compares
all RNG initialization, scope, primitive and operation events in actual order,
with exact values, floating-point bits and unmatched tails. It checks official
release SVG against the instrumented Go build and enabled/disabled observers on
both sides. It does not sort streams or normalize away repeat variation.

This RNG gate does not establish complete candidate, score, traversal,
transaction, routing or seed-race parity. Individual seeds and repeated runs must
be assessed separately from concurrent seed races. Other target/compiler arithmetic
and RNG runtime configurations require their own qualification.
