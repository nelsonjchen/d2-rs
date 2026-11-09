# Retired oracle fixtures: d2-rs-language, d2-rs-lib, d2-rs-elk

Every other tracked fixture over 256 KB in these crates is now a recipe in
`tools/fixtures/langlibelk-*.toml` (docs/FIXTURES.md). The data of the entries
below remains at base commit `9b706ac1` and in the private
`nelsonjchen/d2-rs-history` repository.

## `crates/d2-rs-language/tests/fixtures/edge-key-validation-source-v090.json`

- Tests: none (no reader; it was referenced only by the crate's former SOURCE.md,
  the historical `docs/translation-map.json` evidence list, and as the input
  list of `scripts/refresh-language-master-colors.py --diagnostics`).
- Reason: the v0.9.0 capture of the same 1,362 inputs as
  `edge-key-validation-source-master.json`, superseded by the master capture.
  That fixture is now recipe `lang-edge-key-validation`, whose inputs come from
  `scripts/edge_key_validation_cases.py` instead of this file.

## `crates/d2-rs-language/tests/fixtures/public-graph-structure-v090.json`

- Test: `original_graph_compiler_preserves_complete_root_structure`
  (`crates/d2-rs-language/tests/public_graph_structure.rs`, deleted).
- Reason: its 2,576 requests are literals mined from pinned upstream tests and
  testdata (d2oracle edit_test, d2compiler compile_test, e2etests
  stable/regression/testdata, d2parser, d2ir, d2lsp, sketch/dark/appendix
  and others). The miner no longer exists (only its output survives, in
  `.reference/late-compiler-validation-20261016/requests.json`), and it used
  many extraction kinds with uneven UTF-16 variants, so it cannot be rebuilt
  faithfully. The inputs are upstream golden inputs, covered at the pin by
  the language scoreboard (d2parser/d2ir/d2compiler/d2oracle goldens,
  including compiled graphs) and the e2e and golden-sketch/dark/appendix
  scoreboards. It also carried rows 2416-2420 hand-edited to master values in
  14a7e3d0.

## `crates/d2-rs-language/tests/fixtures/public-measured-state-v090.json`, rows 0..2576

- Test: `original_public_graph_dimensions_bytes_and_attributes`
  (removed from `crates/d2-rs-language/tests/public_measured_state.rs`).
- Reason: the same mined 2,576-request corpus as public-graph-structure (see
  above), measured with `SetDimensions`; label bytes and dimensions of these
  upstream inputs are covered by the e2e and golden scoreboards' rendered
  geometry. The 604 constructed boundary rows (old rows 2576..3180) remain as
  recipe `lang-public-measured-state`, identical to the v0.9.0 rows.

## Merged, not retired

`crates/d2-rs-language/tests/fixtures/ir-compiler.json` and `ir-import.json`
were the trace captures without their `trace` key. Only
`ir-compiler-trace.json` and `ir-import-trace.json` are cached (recipes
`lang-ir-compiler`, `lang-ir-import`); `tests/ir_compiler.rs` strips `trace`
for the plain tests, and the generators assert the stripped capture equals an
uninstrumented Go build.

## d2-rs-elk (second pass)

Nothing in `crates/d2-rs-elk/src/fixtures/` or `tests/elk/` was retired and
nothing was an orphan. Every remaining elk-go capture is now a recipe in
`tools/fixtures/langlibelk-elk-{layered,providers,scalars}.toml`; the case
generators in `scripts/elk_cases/` were recovered from the 2026-10-16..19
probes and reproduce the recorded inputs exactly, except where noted.

Regenerated at the pin and compared with the `9b706ac1` copies:

- Byte-identical: every Box, Fixed, Random, Rectpacking, curated,
  owned-edge, label-identity, junction-admission, list-presence,
  coordinate-ancestor, SPoRE root-transfer and option-resolution fixture, and
  `tests/elk/inline-center-label-minimum-size-go-v090.json`.
- `original_force_logarithm.json` (recipe `elk-scalar-log`): the eleven
  special inputs are unchanged; the 1,000 random inputs came from an inline
  generator that was not retained (its seed is not among Python
  `random.Random(0..10^6)` under the plausible draw formulas), so
  `stress.log_bits()` draws a fresh tail from the same distribution
  (uniform exponent field, positive finite) with seed 1019. The outputs are
  Go's `math.Log` bits for those inputs; nothing is hand-edited.
- D2-adapter-shaped fixtures (`original_{cycle_geometry,fixed_sql_ports,
  mixed_loop_geometry,broader_loop_geometry,greedy_cycle_geometry,
  nested_edge_container,ancestor_descendant_edges}.json`): the ELK request is
  now captured from the pinned D2 adapter (`scripts/elk_cases/adapter_wire.py`,
  `tools/elk-adapter-wire-oracle`). Master drift: d2elklayout now emits
  `spacing.edgeEdgeBetweenLayers` where v0.9.0 emitted
  `elk.layered.spacing.edgeEdgeBetweenLayers` (same value, different key
  position). With that rename applied to the old copies, inputs and elk-go
  outputs are otherwise identical, every float bit included.
- `original_mrtree_expanded.json`: the two Go stack-overflow rows
  (`fractional-011`, `wire-11`) keep only the stable crash header plus a fixed
  marker (`elk_oracle.stable_stderr`); goroutine stacks and addresses are
  dropped. The test only checks for `fatal error: stack overflow`. Two
  `--force` runs are byte-identical.

Kept, not recipes:

- `crates/d2-rs-elk/src/fixtures/original_option_ids.txt`: elk-go's option
  registry ID list, read at run time by `utility.rs` (option suffix
  resolution), so it stays committed like generated library source. Recipe
  `elk-option-registry` checks it against the pinned registry on every regen.
- `tests/elk/*-wire.json` and the eight `tests/elk/d2-*.json` graphs: ELK
  request graphs written by the d2-rs adapter (they carry `d2-rs.*` options),
  not oracle output. Unit tests in `src/lib.rs` lay them out and compare with
  hand-typed expectations; `scripts/check-elk-wire-parity.py` and
  `scripts/check-elk-p5-corpus.py` glob them as their corpus (which is why
  `d2-p3-hierarchical-sweepiness.json` and `d2-p3-multi-external-fanin-down.json`
  have no Rust reader but are not orphans).

## d2-rs-language (second pass)

The remaining small `crates/d2-rs-language/tests/fixtures/*` files are now
recipes, except the entries below. Their data stays at base commit `9b706ac1`
and in `nelsonjchen/d2-rs-history`.

New recipe families: the parser/IR families in
`tools/fixtures/langlibelk-language.toml` (`lang-parser-comment`, `-imports`,
`-indent`, `-numeric`, `-single-quoted`, `lang-edge-resolution`,
`lang-ir-edges`, `-references`, `-tree`, `lang-import-loading`,
`lang-import-selection`, `lang-array-capacities`; `lang-ir-import` now also
writes `ir-import-panics.json` and `ir-import-projection.json`), the
`lang-master-*` hook replays in `langlibelk-language-compiler.toml`
(`compiler-bounds`, `variable-expansion`, `glob-expansion`,
`import-expansion`, `ir-cancellation`, `ast-clone`, `ir-copy`, `small`), the
seeded `lang-public-*` families in `langlibelk-language-public.toml` and the
`lang-contract-*` families in `langlibelk-language-contracts.toml`.

Seeds. The public-compiler and contract families replay constructed D2 inputs
kept under `tools/fixtures/seeds/lang-public-compiler/` and
`tools/fixtures/seeds/lang-compile-contracts/`: the request halves of the
old fixtures with every Go output removed. No generator for them survives;
they are boundary matrices written for these tests (a few public rows are
`pinned/d2compiler/compile_test.go#L…` literals, labelled with their origin).
`position-validation.json` (567 rows) and `style-colors.json` (807 colors,
the first ~450 the CSS named colors in three spellings) are enumerable and
are the first candidates for a generator. `legend-validation-inputs-v090.json`
and `config-data-inputs-v090.json` stay in place as hand-written request
files; the tests read them directly and the recipes use them as inputs.

## Deleted orphans (second pass)

- `crates/d2-rs-language/tests/fixtures/ir-compiler-pending.json`: no reader;
  a one-row native "untranslated imports" note, not Go output.
- `crates/d2-rs-language/tests/fixtures/style-color-go-v090.json`: read only
  by `scripts/refresh-language-master-colors.py` as the input list for the
  master capture. The list is now the seed
  `tools/fixtures/seeds/lang-compile-contracts/style-colors.json` (identical
  inputs) and the master capture is recipe `lang-contract-style-colors`. The
  refresh script, which also targeted the already retired
  `edge-key-validation-source-v090.json`, is deleted with it.
- `crates/d2-rs-language/tests/fixtures/style-color-diagnostics-go-v090.json`:
  no reader; the v0.9.0 capture of the same 88 inputs as
  `style-color-diagnostics-go-master.json` (recipe
  `lang-contract-style-color-diagnostics`).

## `crates/d2-rs-language/tests/fixtures/compiler-source-graph-v090.json`

- Test: `complete_upstream_compiler_failure_cluster`
  (removed from `crates/d2-rs-language/tests/compiler_source_graph.rs`; the
  file's other test stays).
- Reason: nine verbatim d2compiler `TestCompile`/`TestCompile2` cases (input
  and complete `.exp.json` golden) copied from upstream. The same goldens are
  compared at the pin by the language scoreboard
  (`scripts/language-scoreboard.py`, d2compiler package), which is where the
  fixture's own provenance note says they were verified.

## Format and metadata changes (no Go drift)

Every regenerated artifact was compared with its `9b706ac1` copy. All are
identical or differ only in JSON formatting or header metadata, except:

- `import-selection.json`: 80 rows instead of the 19 that matched Rust. The
  old generator kept only rows equal to the native result; rows whose Go
  diagnostic prints AST pointers now keep everything but `errors` and carry
  `unstable_errors`, which the test honors.
- `ir-import-panics.json`: the native `native` field is gone (Go output only).
- `master-glob-expansion.json`, `master-import-expansion.json`: requests carry
  the current hook's complete option set (`max_*`, `context_policy`, `files`);
  results are unchanged.
- `master-compiler-bounds.json`, `master-import-depth.json`: `t.TempDir()`
  paths are now `/tmp/d2rs-tempdir/<Test>/<NNN>` (requests and diagnostics
  alike), so two `--force` runs are byte-identical.
- Dropped header fields (`oracle`, `source`, `scope`, `test_path`,
  `source_test`, `capture_hashes_sha256`) that no test reads.

## d2-rs-cli, d2-rs-lib, d2-rs-dagre, d2-rs-sketch (second pass)

The remaining small Go-oracle artifacts of these crates are recipes:
`tools/fixtures/langlibelk-cli.toml` (Go CLI runs and upstream test-call
captures, `scripts/cli_go_fixtures.py`, `scripts/cli_go_api_fixtures.py`,
`tools/fixtures/programs/cli-layout-probes`) and the `lib-library-*`,
`lib-master-urlenc` recipes in `tools/fixtures/langlibelk-lib.toml`. Two
orphans were deleted; their data remains at `9b706ac1`.

### Deleted orphans

- `crates/d2-rs-cli/tests/fixtures/version-boundary-extended-source-v090.json`:
  no test or script reads it (only `.reference/cli-version-20261019` freeze
  manifests named it). Its 69 extra `--version` argument spellings were
  captured from v0.9.0 next to `version-flags-source-v090.json`, which remains
  as recipe `cli-version-flags`.
- `crates/d2-rs-lib/tests/fixtures/sql_table_columns.d2`: a hand-written SQL
  table diagram from the initial import (`20e3a067`) that nothing reads.

### Kept in place

- The CLI flag table and Go record schema are Rust source declarations in
  `crates/d2-rs-cli/src/command_line_flags.rs` and
  `crates/d2-rs-lib/src/json_record_types.rs`; their former JSON probe
  envelopes are no longer runtime inputs or package contents.
- `crates/d2-rs-lib/tests/fixtures/simple.json`: hand-written serialized graph
  input (it carries `futureField: "preserve me"` probes), also the base of the
  `lib-raw-labeled-opposing` recipe's requests.
- `crates/d2-rs-cli/tests/fixtures/import_lib.d2`, `import_main.d2`:
  hand-written import seeds for the CLI and `scripts/check-cli-watch.py`.

### Captured differently at the pin

- `subtree-output-source-v090.json` and `png-metadata-source-v090.json` now
  list the `.d2-board-output-manifest-v1.json` that master's Go CLI writes into
  multi-board output folders; the v0.9.0 captures had none.
- Upstream `t.TempDir()` paths in `board-path-api.json` and
  `library-source-api.json` are rewritten consistently in requests and results
  to fixed, nonexistent `/tmp/d2rs-oracle/<test>/` paths; document-dispatch
  stderr in `version-flags-source-v090.json` carries `{duration}` and `{cwd}`
  placeholders. No test compares those parts against the filesystem.
- Provenance blocks drop the v0.9.0 oracle binary hashes and scratch paths.

## Top-level `tests/` (second pass)

- `tests/d2/*` and `tests/regressions/**`: hand-written D2 seeds, kept.
- `tests/graphjson/{decoder,preflight}-cases.json`: hand-written graph JSON
  decoder/preflight request lists (named cases such as `ascii_case`,
  `pointer_merge`) read by `scripts/check-graphjson-*.py`; kept.
- `tests/upstream/tala-fuzz-expanded/` (24 corpus records + `manifest.json`):
  locally chosen `FuzzAutolayout` tuples in Go's corpus encoding, not
  upstream entries and not oracle output; kept as seeds.
- `tests/elk/*`: see the d2-rs-elk section.

### Replaced upstream copies

- `tests/upstream/d2/*.d2` (27 files) and
  `tests/upstream/d2/real_world_licenses/jupyter_k8s_oidc_LICENSE`: verbatim
  copies of `e2etests/testdata/files/*` from v0.9.0, all 27 byte-identical at
  the pin 6a7b48378 (checked by sha256 on d2rs-small). Generators now read
  them from the pinned tree through `scripts/upstream_inputs.py`
  (`lang-parser-containers`, `lang-format`, `lang-ast-clone`; their keys now
  name `e2etests/testdata/files` instead of the copies). The one Rust reader,
  `dagre_node_order_matches_go_127_mixed_variable_reference_sort` in
  `crates/d2-rs-language/tests/graph_lowering.rs`, reads recipe
  `lang-upstream-e2e-input` and is now in the oracle tier. Developer scripts
  (`check-render-parity.py`, `check-grid-trace.py`, `check-dagre-order-trace.py`,
  `audit-swap-port-trace-repeatability.py`, a 2026-10-02 probe) point at the
  pristine checkout. The license file travels with its source upstream.
  `containers.json` rows for these inputs now label them
  `upstream/e2etests/testdata/files/<name>` instead of `tests/upstream/d2/<name>`;
  the bytes, hashes and Go results are unchanged.
- `tests/upstream/manifest.json`: the v0.9.0 checksum inventory of those
  copies plus 129 TALA internal testdata paths. Its `upstream_commit` is
  v0.9.0, so `check-boundaries.py` already refused to run at the pin. That
  check is removed (nothing is copied any more) and
  `audit-engine-fixtures.py` enumerates the internal testdata from the
  pristine tree.

### Deleted orphans

- `tests/d2/markdown-nested-inline.svg`: a v0.9.0 Go render
  (`data-d2-version="v0.9.0"`) of `tests/d2/markdown-nested-inline.d2` that
  nothing reads. The other `tests/d2/*.svg`, `*.jpg` files are image assets
  the D2 seeds embed as icons, kept with them.
- `tests/runtime/go127-int63n.json`: a byte-identical duplicate of
  `crates/d2-rs-tala/src/internal/runtime/fixtures/go127-int63n.json`, the
  copy `go_rng.rs` actually reads (TALA area); nothing read the top-level one.
