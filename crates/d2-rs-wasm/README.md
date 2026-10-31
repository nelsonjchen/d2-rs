# d2-rs-wasm

Initial Rust WASM bindings migrated from archived Weftan. Exposes
`layout_graph_json` and `layout_d2_json` for already measured graphs,
`parse_d2_source`, `format_d2_source`, and `compile_d2_source` for source
stages, `complete_d2_source`, `board_at_position_json`, and
`reference_ranges_json`, and `reference_ranges_files_json` for editor helpers,
and `layout_d2_source`
for filesystem-free source-to-layout calls.
`render_d2_source` returns the same semantic SVG target used by the native
source command.
The raw editor surface also provides `apply_source_edits_json` plus
AST-backed `create_root_field_json`, `set_root_field_json`,
`delete_root_field_json`, `rename_root_field_json`, `reconnect_edge_json`, and
`move_object_json` operations. Object moves use native compiler references to
preserve declarations, edge references, descendant scopes, collision names and
`near` references through transplants and child hoisting. A move request accepts
`source`, `old_key`, `new_key`, `include_descendants`, optional `path` (default
`"-"`), `files` (a virtual path-to-source map), and `board_path` (alias
`boardPath`, an array of board names). Imports read only the supplied virtual
files; the response contains the updated entry source and selected object key.
The entry compiles before key handling, including unchanged-key requests, and
returned source is formatted. Board write restrictions follow the pinned source
API. Indexed edge keys use D2's Move behavior: arrows change on every compiled
reference while its endpoint paths stay unchanged. Broader inherited/glob/import
combinations and remaining malformed requests stay under parity work. These
options apply to object moves; other graph-edit helpers keep their existing
source-level boundary.
`compile_d2_target_json` additionally returns the indexed measured graph,
source labels/styles, and layout metadata used by the high-level `D2` facade.
Passing `ascii: true` to `render` selects the in-process text canvas, with
`asciiMode: "standard"` mapping the extended box glyphs to basic characters.
Its `asset_data` option accepts a map from an icon/path string to standard
base64 bytes; supplied image assets are embedded as data URIs without giving
the WASM module filesystem or network access.
Its JSON options also accept `target` for an explicit board path such as
`layers.name`, `sketch: true` for the deterministic seeded sketch overlay.
Using `target: "*"` or `target: "layers.name.*"` with a positive
`animateInterval` renders the selected board and descendants as a deterministic
animated SVG in the same source order as D2 v0.9.0; multi-board ASCII output
and unanimated multi-board requests are rejected at the same boundary.
`WasmCancellationToken.expire()` reports a host deadline separately from
`cancel()`: the first cause wins (`context deadline exceeded` or
`context canceled`). These synchronous calls do not create a JavaScript timer.

`WasmCancellationToken` and the corresponding `*_with_cancellation` layout
exports let worker-hosted callers request cooperative cancellation at the
same engine boundaries as native Rust. A synchronous call on the same JS
thread cannot be interrupted until it returns to its host.
`route_edges_d2_json` accepts the native plugin's base64 `g`/`gEdges` envelope
for browser-side standalone routing, with a cancellation variant as well.
The raw v0.9.0 helper names `getParentID`, `getCompletions`,
`getBoardAtPosition`, `getRefRanges`, and `getObjOrder` are also exported with
the original JSON response envelope for existing D2 editor integrations.
`getELKGraph` is available as a measured in-process pre-layout graph for the
deprecated raw editor bridge and matches the pinned converter on simple,
nested, and SQL-table graphs, including fixed column ports and table edge
endpoint IDs. Edge labels carry ELK's inline-label option and large self-loop
labels raise container spacing like the Go adapter; less common option
projection remains under parity work.
The source APIs accept JSON-string options and preserve the UTF-16 position
switch used by the native parser. Supplying `files` and an optional `path` in
those options enables relative imports from an in-memory virtual filesystem.
The high-level facade accepts `fontRegular`, `fontItalic`, `fontBold`, and
`fontSemibold` byte arrays; they are validated and used for source measurement
inside the worker.
The high-level `D2` class now provides the v0.9.0 `compile`, `render`,
`encode`, `decode`, `version`, `jsVersion`, and `dispose` lifecycle in both
Node and browser entrypoints. `compile` accepts source strings or virtual
filesystem requests and returns a diagram projection plus a compiled graph
projection with indexed target metadata (`nodes`, `edges`, `layout`, and source
configuration), serializable `root`/`objects`, graph attributes, parent IDs,
and routed edges that can be passed back to `render`; requests run in a
dedicated worker and pending requests are rejected after disposal. The raw
functions remain available for callers that need synchronous access. Full
v0.9.0 `d2graph.Graph` source-reference identity and exact nested-board
semantics remain tracked in `docs/PLAN.md`; the compiled AST projection is
present at the root and board boundaries, and object/edge records include
deterministic AST-key reference records. The returned `fs` contains the
formatted selected input file, matching the v0.9.0 compile response while
leaving imported virtual files unchanged. Root compile requests include
ordered `layers`, `scenarios`, and `steps` board projections; board AST and
reference fields remain under active parity work. Board projections currently
reuse the source compiler independently for each board; sharing that context
is a later performance optimization for large board trees.

Build-check: `cargo check -p d2-rs-wasm --target wasm32-unknown-unknown`.
The package entrypoint in `src/index.js` lazy-loads the generated web
wasm-bindgen module, while the conditional Node export uses `src/node.cjs`
and the synchronous nodejs target. Run `pnpm build` from this directory to
build both targets with a matching `wasm-bindgen` CLI and generate `pkg/`;
`pnpm check` validates the wrappers and Rust target without generating package
output. The generated directory is ignored and must be rebuilt for each
distribution.
MPL-2.0; preserve LICENSE, THIRD_PARTY_NOTICES.md, DEPENDENCY_NOTICES.md and
SOURCE.md in distributions.


Browser contracts run with `pnpm install --frozen-lockfile` followed by
`pnpm test:browser`, after building `pkg/`. The browser test driver requires
Node 20 or newer and uses the installed Chrome by default; it downloads no
browser. Set `D2_RS_BROWSER_CHANNEL=msedge` to test an installed Edge.
`D2_RS_BROWSER_PACKAGE` selects a frozen package directory for reproducible
qualification, and `D2_RS_BROWSER_OUTPUT` selects the JSON evidence file.
An optional `D2_RS_BROWSER_MATRIX` supplies raw layout inputs or routing
envelopes (`operation: "route"`, `request_json`); complete returned JSON
strings are preserved for independent source comparison.

The test runs real module workers and checks source-backed cancellation, seed
precedence and node measurements, plus imports, boards, SVG/ASCII output,
UTF-8 codecs, UTF-16 edits, error recovery and disposal of posted requests.
Timeouts, missing results, unexpected request failures and browser errors fail
the run. Playwright is a pinned development dependency only. These browser
checks do not certify every browser, operating system or D2 capability.

The CI WASM matrix builds the locked package and runs Node, TypeScript and
real browser-worker contracts on Linux and macOS with Node 22. CI installs
Chromium through the pinned Playwright CLI; local browser tests continue to
use installed Chrome by default. Workflow configuration is not evidence that
all platforms or D2 capabilities have passed.

The legacy `getParentID`, `getObjOrder` and `getELKGraph` exports preserve
Go's distinction between an omitted argument and an explicitly passed value.
Host values are marshaled before the typed WASM string boundary; D2 parsing
and graph work remain in Rust. Codec helpers share this host-value marshaling
but retain their documented Rust convenience response shape.

URL decoding follows Go's padded URL-base64 and DEFLATE rules, including
accepted CR/LF, nonzero pad bits, exact corruption offsets, truncated-stream
errors and invalid-UTF-8 replacement at the JSON boundary. Workers and the
no-worker fallback both reject codec errors. Encoders may emit different valid
DEFLATE bytes; interoperability is checked by decoding in both runtimes.

### Serialized target rendering

The raw WASM entry point `render_d2_target_json(diagramJson, optionsJson)` paints
serialized D2 targets without compiling or laying out source. `optionsJson` uses
public D2 render names such as `pad`, `scale`, `themeID`, `target`, `salt` and
`noXMLTag`; it does not inherit compile options. It returns an SVG string.
Both Node and browser raw packages export it. Run `pnpm test:target` for the
source-backed client contracts.

This entry point supports single-board SVG, including legends painted from the
supplied target records, and standard/Unicode ASCII. Explicit `ascii: true`
requests through high-level `D2.render` also use the supplied target in direct
and worker paths, including deserialized and mutated targets. Run `pnpm test:ascii`
for those client contracts. Target animation and broader font-state/option edge
cases remain incomplete. High-level SVG render paths still render saved source;
their switch to the target entry point remains pending those qualifications.
