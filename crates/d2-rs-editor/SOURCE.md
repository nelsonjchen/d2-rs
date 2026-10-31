# Corresponding source

This is an unpublished development checkout of d2-rs. There is not yet a public
release source URL. Keep this complete checkout, its Git revision and all local
modifications alongside any locally shared binary or WASM artifact. Do not
substitute the archived Weftan repository for the source of d2-rs modifications.

The upstream source and historical Rust input are pinned in
reference-manifest.json. Development references can be reconstructed with
scripts/references.py and docs/REFERENCES.md. Instrumentation patches are tracked
under tools/patches; ignored references must not contain undisclosed changes.

`imports.rs::update_import` translates `d2oracle/edit.go:3337–3478`
(`UpdateImport`, `_updateImport`, `updateImportPath`, `matchesImportPath`,
`getNewImportPath`). It parses with an empty source path, walks map imports,
removes spread nodes or clears field imports, updates exact paths/directory
prefixes, preserves selective-import components and formats without compiling
or opening imported files. It retains the source's all-dot/slash prefix behavior
and does not walk array contents. Parse errors carry Go-quoted operation context.
The primary-scalar import branch is unreachable in the pinned ScalarBox shape.

The oracle-tier artifact `tests/fixtures/update-import-v090.json` binds every
current TestUpdateImport text test of `d2oracle/edit_test.go` (recipe
`mathtext-txt-editor-update-import`, an env-gated recorder over the upstream
test). `update-import-boundaries-v090.json` adds six pinned-Go source witnesses
for selective/directory paths, all-dot/slash prefixes, arrays and contextual
errors (recipe `mathtext-txt-editor-update-import-boundaries`, inputs in
`tools/fixtures/inputs/mathtext-txt/update-import-boundaries.json`). These are text contracts; the 15 historical
UpdateImport graph goldens are not exercised by the current Go tests and remain
non-passing in the graph tier.

`graph_edit.rs::rename_object{,_with_options}` and
`graph_edit/compiled_move.rs::rename` translate pinned
`d2oracle/edit.go:1699–1748` (`Rename`): board lookup, literal final-component
names, generated-name conflicts, indexed-edge arrow changes, and the shared
`move(..., false)` reference update. The existing root-only helper is retained
with its original contract. The TestRename rows of the oracle-tier
`tests/fixtures/editor-graphs-v090.json` (recipe `mathtext-txt-editor-graphs`)
bind all 25 live upstream TestRename formatted-source/error results. The language scoreboard now qualifies both the formatted-source/error tier and
the complete native graph result (run 10).

`compiled_move::{recompile,compiled_result}` return the complete native
pre-measurement graph through the shared `compiler::source_graph_json` projection.
This follows `d2oracle/edit.go:315–324`: format the edited AST, reparse at the
original path with its import filesystem, and compile. No-op Move/Reconnect
branches preserve the original compile/ranges. The final format precedes JSON
serialization, as in `edit_test.go:8327–8347`; it fills single-quoted raw text
(`TestSet/label_primary`). Existing serialized source-result schemas remain
unchanged; their graph fields are available directly to native callers.

`create_object{,_with_options}` and `compiled_move/set.rs::create` map
`edit.go:31–77,2650–2731`: board selection, unique object/edge keys and shared
`_set` insertion. Scalar boards use an empty IR map and materialized BaseAST,
matching `d2compiler/compile.go:185–211,230–267` (`TestCreate/add_layer/4,5`).
`reconnect_edge_with_options` and `compiled_move/reconnect.rs` map
`edit.go:124–313`: compiled edge lookup, no-op endpoints, writable board refs
(`get.go:243–250`), preserving former endpoint nodes, splitting chains and
rewriting every endpoint relative to its reference scope. Existing AST-only
root/Create/Reconnect helpers keep their contracts.

`tests/fixtures/editor-graphs-v090.json` retains all **426 live** pinned
`d2oracle` inputs, original golden hashes and complete expected JSON: 36 Create,
102 Set, 122 Delete, 117 Move, 25 Rename and 24 Reconnect. Tests compare exact
AST/reference ranges, null/empty collections, ordering and structured errors,
alongside all formatted source results. All 426 match; the 19 historical editor
goldens without live inputs remain explicitly unbound, not passing.

`graph_edit.rs::{delete_object,set_object}{,_with_options}` add the native
compiler-resolved Delete/Set APIs while preserving the root-only helpers.
`compiled_move/delete.rs` maps `d2oracle/edit.go:937–1697`: reserved fields,
import/glob null overrides, board write scopes, child conflict renames and
hoisting, edge-chain splitting, endpoint preservation, index decrements and
near-reference updates. Shared source-reference helpers use
`d2oracle/get.go:127–155,234–250`. Table/class columns remain outside graph
child lookup, matching `d2compiler.compileField` and `d2graph.HasChild`.
`compiled_move/set.rs` maps `edit.go:80–129,327–935`: literal RawString and
tagged block values, nested insertion scopes, writable labels, indexed-edge
references and reserved-attribute setters. Materializing an imported board's
writable AST retains its spread import, as `_findFieldAST` does in
`d2compiler/compile.go:231–267`.

The TestDelete/TestSet rows of `tests/fixtures/editor-graphs-v090.json` bind
every live pinned TestDelete/Set request and complete formatted-source/error result: 122 Delete and 102 Set
cases, with no test exclusions. These finite text/error matches do not prove
full editing semantics or the complete graph contract. The two retained
historical goldens without live hook inputs remain unbound in the scoreboard.

`graph_edit.rs::move_object` and `move_object_with_options` translate object
moves from pinned `d2oracle/edit.go::Move`/`move` (1765–2311). The latter accepts
an entry path, virtual import files and a board-name path. Native IR supplies
source references; component indices and primary maps are captured before AST
mutation, matching the graph reference snapshots in `d2compiler/compile.go`.

`graph_edit/compiled_move.rs` handles lookup, validation, board write scopes,
reference renaming, terminal edges, `ensureNode`, `updateNear` and recompilation.
`compiled_move/cross_scope.rs` translates declaration and intermediate-edge
transplants, destination selection, flattened-map splitting, child hoisting,
underscore relocation and implicit-parent preservation. Source helpers include
`getMostNestedRefs`, `appendUniqueMapKey`, `appendMapKey`, `hoistRefChildren`,
`findNearestParentScope` and `pathFromScopeObj`. `compiled_move/conflicts.rs`
translates `renameConflictsToParent` and ordered generated-name collisions.
Graph name lookup follows `d2graph.HasChild`; board write restrictions follow
`d2oracle/get.go::GetWriteableRefs` and `GetBoardGraph`. The distinct projected
board AST and writable source AST preserve the source's observable behavior,
including moves that disappear from returned board source and conflict errors.
Formatting runs after native recompilation, as in the Go source caller.

`tests/fixtures/move-transplant-v090.json` contains 392 complete outputs/errors
from repeated pristine Go calls: all 119 TestMove fixtures, every fixture with
the opposite descendant setting, 24 earlier audit inputs, 92 reference probes,
and 38 board/import probes. Both fixtures skipped by the upstream test runner
are executed. `move-oracle` accepts the same path/files/board context as the
WASM request. The move corpora are oracle-tier artifacts (recipes
`mathtext-txt-editor-move-*` over `tools/fixtures/inputs/mathtext-txt/editor-move.json`
and the upstream TestMove table); the earlier 92-case reference fixture, a
subset of the transplant corpus without a reader, was deleted
(docs/fixture-retirements/mathtext.md).

A further 62-case `move-boundary-v090.json` corpus checks input compilation
before key handling, unchanged-key formatting, missing import diagnostics,
board lookup before key errors and invalid board-local near references. Entry
and result validation lower all compiled boards, as `compileBoard` does in the
source. Identical key strings bypass key and board lookup only after compiling
the entry, including identical empty or edge-like strings.

`compiled_move/edges.rs` translates the indexed-edge branch of `move`,
`generateUniqueKey`, `edgeTrimCommon`, `d2graph.HasEdge`/`FindEdges` and
`ResolveUnderscoreKey`. It uses the native compiled graph plus source IR edge
origins, retaining graph occurrence order before AST sorting for `initIndex`.
Scoped edge-property keys take the object branch using their containing path,
as Go does. Source references in the edge branch are rewritten in reverse
order, changing only arrow strings;
endpoint paths remain unchanged. The selected board supplies destination-index
lookup, while the source edge search deliberately uses the original root graph,
as Go does. Recompile catches invalid import/local-reference combinations.
The 176-case edge corpus includes aliasing, chains, table/class columns,
sequences, globs, boards and imports. Its eight reproducible source panics stay
in the test inventory as explicit differences with safe native rejection.

These finite corpora are not full `d2oracle.Move` parity. Own-child moves,
remaining malformed inputs, deeper inherited board/glob/near/import combinations
and complete graph validation/error aggregation remain outside qualification. Other editor
operations retain their separately documented API boundaries. See
`docs/STATUS.md` for measured runtime results and remaining work.

The source-derived math table in `crates/d2-rs-math/src/glyphs.rs` is generated
from `github.com/d2lang/mathjax-go v0.1.0/internal/font/data_normal_gen.go`,
the MathJax 3.2.2 TeX font data used by the pinned D2 release. Reproduce it
with `uv run --no-project python scripts/generate-math-glyphs.py
/path/to/mathjax-go/internal/font/data_normal_gen.go
crates/d2-rs-math/src/glyphs.rs` before extending the subset. This process uses
the verified local source and does not execute JavaScript or a browser.

Before publication, supply a verified accessible source URL and exact source
revision for every artifact. Include LICENSE, THIRD_PARTY_NOTICES.md,
the repository root's DEPENDENCY_NOTICES.md and this file with every binary/WASM distribution and
crate. Source recipients must receive the MPL-covered modifications needed to
build their version. Third-party material retains its applicable terms.

## Completion context follows d2lsp completion.go (2026-10-23, Claude)

`completion_items` ports `d2lsp/completion.go` getCompletionItems (lines
43–99): parse the document, re-parse the text before the cursor on error
(getTextUntilPosition), resolve the keyword with getKeywordContext (120–236)
over the AST, then use its switch. Helpers columnByteOffset, isAfterDot,
isAfterColon (Go's unicode.IsSpace on a byte) and isPositionInMap are
translated directly. This replaced lexical prefix heuristics that missed the
arrowhead-map context. Vocabularies follow source: d2target.Shapes uses
`stored_data`; `near` uses d2ast.LabelPositionsArray. Witness: the
lsp-completion inline tier, 26/40 → 40/40.
