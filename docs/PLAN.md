# Plan

## Contract

d2-rs builds native libraries, the `d2-rs` executable and Node and browser
WASM packages. Every algorithm runs in Rust, with no Go, JS engine or
browser runtime; thin JS bindings around the WASM module are allowed.

The target is the Go D2 commit pinned in `reference-manifest.json`,
including its bundled TALA engine. Output must be byte-identical with the
pinned Go for the same input and options: diagnostics, geometry, SVG, PNG,
GIF, PPTX and the deterministic parts of PDF. Where Go is nondeterministic
(map order, wall-clock dates, temp paths), d2-rs matches one of the outputs
Go produces and the tests accept every variant Go produces. Speed and peak
memory must stay at or under Go's on every benchmarked case.

The workspace mirrors upstream's package layout
([PORT-MAP.md](PORT-MAP.md)). AST, IR, graph, target and scene stay
distinct stages, and parse, compile, layout, render, format and edit are
exposed separately. External layout plugins (`d2plugin-*`) are out of
scope; upstream master has removed them.

## Where it stands

The port is complete against upstream's own tests: the e2e goldens, the
`e2etests-cli` suite, the oracle tier on arm64 and x86-64, and the
differential harnesses ([STATUS.md](STATUS.md)). Every corpus case is at or
under Go's time and memory in every output format
([BENCHMARKS.md](BENCHMARKS.md)).

## Next

1. **Gaps beyond the corpus.** Close the open items that the generated
   sweeps found (STATUS, "Beyond the upstream corpus"): raster text and
   short shapes in PNG, extreme scales, error-text details, the remaining
   LaTeX and Markdown cases, and TALA's time and memory on large boards.
   Keep running the sweeps; each new mismatch cluster is a failing board.
2. **Track upstream.** When upstream master moves, move the pin as
   [REFERENCES.md](REFERENCES.md) describes, regenerate the fixtures, and
   treat every new failure as drift for the Rust code to follow.
3. **Unmeasured areas.** WASM performance beyond the 36-row benchmark,
   browsers, macOS and Windows hosts, and long fuzz runs.

Work is chosen and checked by the scoreboards in
[SCOREBOARD-WORKFLOW.md](SCOREBOARD-WORKFLOW.md), under the rules in
[AGENTS.md](../AGENTS.md).
