# Corresponding source

Lane L25 (master `6a7b48378`): `d2svg/element.rs` follows
`d2themes/element.go:94–330`, routing ordered element attributes through
`lib/svg/markup.go:44–80,145–168`. Paint and string attributes escape once;
deprecated attribute fragments and local references are parsed and validated;
finite numeric attributes and typed text/trusted SVG content keep their source
boundaries. Typed attributes/IDs and Rust `Clone` preserve independent owned
attribute lists. The image caller now supplies its raw URL as
`d2svg.go:1824` does, avoiding double escaping at the new serialization boundary.
The six original `d2themes/element_test.go` functions remain active in the
capture hook; `tests/fixtures/themes-api.json` retains their 15 complete
serialization/panic observations. Native panic admission is compared, not Go
panic diagnostic text. This finite tier does not qualify post-render gradient
field mutation or all theme/palette APIs.

Master run59: `detect_image_mime` exposes the existing Go net/http byte-sniff
translation in `image_mime.rs` without extension lookup. The foundation
imageasset probe uses it for the malformed-image diagnostic in
`lib/imageasset/probe.go:118`; valid renderer output and encoder behavior are
unchanged. The original Go BSD notice stays on the shared detector.

Run 13 (2026-10-23): `TargetDiagram::render_d2svg` and
`d2svg::render_wasm` map original `d2js/d2wasm/functions.go:347–551` to the
same d2svg/appendix/animation implementation as CLI SVG. The shared
`target_diagram_tree_json` preserves recursive board ordering and hashing.
Source-style route faults retain `RenderError`'s recovered fault/context
transport (`d2svg.getArrowheadAdjustments:897`, `drawConnection:1136`).
WASM's target decode and ASCII boundaries remain independent of compilation.
The legacy SVG/precision bridge remains required by native raster exports:
original Go `d2cli/main.go:1005–1032`, `d2cli/raster.go` and `paged.go` use
`d2scenebuild`/`d2raster`, not an SVG screenshot. Replacing that bridge requires
its own zero-regression pixel/document qualification. See run-13 evidence.

Native sketch connections in `src/sketch_connection.rs` follow pinned D2
`d2renderers/d2sketch/scene_connection.go` and
`d2renderers/d2scenebuild/sketch_connection.go`: adjusted route geometry,
eight-step de Casteljau cubic flattening, shared random streams within each
256-segment set, independent endpoint seeds, rough fill/outline paint order,
and local arrow transforms. The original-Go operation fixture is
`tests/fixtures/sketch-connection-source-v090.json`; its reproducible public
Go oracle is `.reference/sketch-connections-20261018/oracle/main.go`.
Filled-diamond zigzag filling now follows the shared native scan-line filler,
including its fixed fill stroke weight and empty-set handling. Canonical SVG
rough path behavior and full document-wide sketch budget/cancellation accounting
remain unqualified.

This is an unpublished development checkout of d2-rs. There is not yet a public
release source URL. Keep this complete checkout, its Git revision and all local
modifications alongside any locally shared binary or WASM artifact. Do not
substitute the archived Weftan repository for the source of d2-rs modifications.

The upstream source and historical Rust input are pinned in
reference-manifest.json. Development references can be reconstructed with
scripts/references.py and docs/REFERENCES.md. Instrumentation patches are tracked
under tools/patches; ignored references must not contain undisclosed changes.

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

Code token/rendering translation: original Go D2 v0.9.0 `d2scenebuild/code_shape.go` and `d2svg.go` language branch; pinned Chroma v2.27.0 `lexers/go.go`, `lexers/embedded/go_template.xml`, `iterator.go`, `style.go`, and `styles/{github,catppuccin-mocha}.xml`. Native Go/template rules and two style tables are implemented; full lexer registry, resources/validation/cancellation, and scene API qualification remain open.

Post-layout outside-label dimensions (checkpoint482): Source d2exporter/export.go
exportShape copies current Text dimensions; d2svg.drawShape places text using
those target dimensions, and d2target.Diagram.BoundingBox reserves the same
label box. Native outside-label painting and decoration bounds prefer live
LayoutResult.node_labels.size, then node.label_size, then the external record.
The retained record's placement provenance is preserved and graph state stays
unchanged. The outside_bottom_labels witness has exact native/Source layout
boxes and live text state; only the renderer retained pre-scale dimensions.

The live-external-label-source-v090.json fixture preserves 120 pristine SVG
outcomes: twelve outside positions, grow/shrink, rectangle/person, multiple and
rectangle 3d. Native checks 240 authored/automatic text origins and full canvas
viewBoxes. Broader text/render/layout/TALA/resources/cancellation/API/WASM/
platform parity remain separate open requirements.

Badge paint ordering follows original Go d2svg.Render's appendixItemBuf:
addAppendixItems emits badges outside shape opacity/hyperlink groups, and the
buffer is painted after every diagram object and before the legend. Native
render_svg collects draw_link_tooltip_badges into a separate String at the
same boundaries. Raw24Source SVG controls and actual PNG evidence are retained
in .reference/badge-paint-order-20261006/; the full source/layout/SVG regression
fixture is crates/d2-rs-cli/tests/fixtures/badge-paint-order-source-v090.json.
This maps paint order only; shape-specific badge positioning remains separate.

Shape-specific badge anchors follow d2svg.addAppendixItems's center/corner
selection, TraceToShapeBorder, two-icon offset and ceil rounding. The renderer
reuses d2-rs-core::shape_geometry::trace_shape_border; d2target.BoundingBox's
top/right badge envelope and width-derived C4-person head bounds are retained.
The raw4320pristine SVG inventories span18families,4metadata states,4sizes,
3origins and5stroke widths; every anchor and every SVG viewBox is compared in
crates/d2-rs-render/tests/fixtures/badge-anchor-source-v090.json. Physical probe
source lives only in ../d2-v0.9.0-trace/d2renderers/d2svg/badge_anchor_probe_test.go;
reproduction and full output evidence is .reference/badge-anchor-20261006/.
Positioned tooltip and full target behavior are separate open contracts.

Positioned-tooltip metadata handoff (checkpoint486) maps original Go
compilePosition/compileField and exporter.toShape. Native SourceNodeStyle now
retains tooltip.near independently of tooltip text and the target emits it.
Quoted reserved keywords remain ordinary graph fields; board recognition uses
actual unquoted wrapper fields, preserving original quote-insensitive core IR
NodeBoardKind solely as navigation. 227 raw Source compile/export controls and
native regressions compare complete ordered metadata arrays/full diagnostics.
Source SVG controls for480inputs are captured and replayed; native positioned
Markdown box/tail/bounds/font/error parity remains open. Evidence and scope:
.reference/positioned-tooltip-20261006/source-comparison.md.

Positioned-tooltip continuation487 translates original d2target positioning/BoundingBox, d2scenebuild/tooltip.go and appendix.go, and textmeasure/markdown_svg.go table border traversal. Shared Markdown measurement now lives in markdown_measure.rs with borrowed custom font faces, replacing the CLI-owned copy. All480SVG geometry/text/viewBox/error controls and384actualPNG controls match; fullSVGbytes and broader Markdown/sketch/theme/API qualification remain open. Evidence: .reference/positioned-tooltip-20261006/paint-source-comparison.md.

Circle-arrow continuation488 translates original d2scenebuild/connection.go endpoint matrix and ellipse radii/stroke modes, plus d2raster/geometry.go cubic quarters. The native raster transport retains local f64 geometry instead of SVG marker expansion. Original Go1.27 ARM64 Matrix.Mul assembly establishes fused product/add operand order. All672matrix/radius bit controls and77of96authored fullPNG controls match;45new matches,0losses.19translucent label/effect-origin controls remain open; other architecture arithmetic is unqualified. Evidence: .reference/owner-opacity-20261006/source-comparison.md.
Checkpoint 491 maps original `lib/textmeasure/markdown_svg.go:paintFlow`,
`paintListMarker` and `textBaseline` to parsed paragraph/list sibling flow,
font-scaled marker geometry and selected CSS font-box baselines. The native
Goldmark projection supplies block boundaries; measurement keeps the legacy
viewport contract. All 72 complete Source primitive controls and 71/72 full PNG
controls match across bundled fonts, sizes and default/terminal/sketch modes.
Evidence and remaining list/wrapping/paint gaps are recorded in
`.reference/markdown-flow-20261007/source-comparison.md`. D2-derived changes
remain covered by MPL-2.0; Goldmark-derived changes retain MIT notices.
Checkpoint 492 translates original `lib/textmeasure/markdown.go` list inline
measurement from parsed text nodes, including innermost font selection and
sibling-space advances. All 144 expanded Go viewports and 72 prior flow
viewports match. The full-image controls improve 71→72/72 and 49→106/144,
with no losses; the 38 remaining expanded cases include four native-only
uppercase raw-HTML rejections and retained code/decoration/link paint failures.

Static appendix placement now shares target bounds from original
`d2target/d2target.go:Diagram.BoundingBox`, passed to appendix measurement by
`d2renderers/d2scenebuild/build.go`. The separately painted legend affects
canvas bounds but not the appendix target box. All 96 original-Go bounds
controls verify integrated/direct appendix APIs with and without a legend.
The complete lion_reader_frontend PNG now matches. Evidence and remaining
PLAN gaps: `.reference/markdown-list-width-20261007/source-comparison.md`.
D2-derived changes remain covered by MPL-2.0.

Checkpoint493 follows original textmeasure/markdown_svg.go paintInlineNodes: paragraph and list contents share CSS code-box metrics, backgrounds precede glyphs, and one Markdown style group retains absolute inline coordinates. HTML consumers fold ASCII names per pinned x/net/html v0.57.0 while leaving Goldmark projection untouched. The96 public API controls retain60 full layouts/36 errors; compiler-stage case-sensitive XML validation remains a distinct gap. Evidence: .reference/markdown-html-case-20261007/source-comparison.md.

Checkpoint495 follows original d2exporter.applyStyles root defaults/hash values, d2scenebuild.buildRoot rectangle/viewport expansions, radius clamping, double borders/patterns and scene root-effect preflight. All121 complete exported-root/viewport/background-geometry/error controls match. Exact PNG results, retained gradient raster differences and broader API/export/PLAN gaps are recorded in .reference/root-canvas-style-20261007/source-comparison.md. D2-derived changes retain MPL-2.0 notices.

Checkpoint496 separates original d2svg raw root rx from d2scenebuild scene radius clamping: draw_root_rectangle clamps only the native scene boundary. All143renderer tests and prior121Source root metadata/geometry/error controls pass. Evidence/scope: .reference/root-gradient-boundary-20261007/source-comparison.md.

Checkpoint499 follows original textmeasure paintFlow/paintBlock/paintListMarker for mixed heading, paragraph and flat list blocks. The shared heading painter reports painted height; list content uses the complete block origin before baseline/marker arithmetic, and legacy sizing rounds the aggregate body rather than each heading. All324 Source heading/body/list controls and72 prior paragraph/list controls match viewport and complete ordered primitive coordinates/sizes/radii. Full144 renderer and150 export tests pass. Mocha full PNG differences improve207,990 to29,185 but remain an exact failure. Evidence and explicit scope limits: .reference/layout-geometry-triage-20261007/source-comparison.md. D2-derived changes retain MPL-2.0 notices.

Checkpoint500 follows original d2scenebuild.buildMarkdownLabel and d2svg markdown render: target fill is the first background inside the clipped Markdown viewport, with theme ownership retained and shape opacity on its parent. Original exporter.applyStyles gives unauthored text shapes transparent fill after theme selection; the handoff now preserves that override. All64 integrated Source background/error controls match (48accepted,16errors) through ordinary SVG/private raster handoff. Final CLI matches60/64, with four retained single-pixel opacity failures. Mocha now has0pixel differences. Source raster/bands visible paint bounds are mapped in d2-rs-export; full145renderer+151export tests pass0ignored. Evidence and remaining limits: .reference/markdown-background-layer-20261007/source-comparison.md. D2-derived changes retain MPL-2.0 notices.

Checkpoint502 maps original compileEdge reference ScopeMap/BoardIDA,
IsSequenceDiagramGroup/Note and ExtractSubgraph root ownership to native
declaration-scope metadata, group/note classification and actor grid projection.
Six complete Source group inventories/default fills match;495 affected tests
and25 raster-harness tests pass. Broad149/159 remains unchanged without losses;
nesting_power improves448106→145138pixels but remains an exact failure. The
focused sequence CLI retains all7 exact failures, including a Source error
Native does not yet reproduce. Canonical actor identities/proxy elimination,
full graph/target/routes/errors/pixels and wider PLAN gates remain open.
Evidence: .reference/sequence-reference-groups-20261007/source-comparison.md
and qualification.json. No full-D2 completion claim.

Checkpoint504 maps original d2exporter/export.go LabelFill and d2svg.drawShape/d2scenebuild.buildShapeLabel/shape_effects.appendShapeGeometry to ordinary sequence group label background rectangles and geometry-only blend ownership. Label fill/text/icons are siblings outside blend, while authored object opacity remains on the complete shape. The permanent sequence-group-label-fill-source-v090.json checks original target/SVG geometry and ownership. All300renderer/export tests and25harness tests pass;12authored Source/official/native controls are exact. Focused sequence1/7→2/7; broad149/159has no regressions. Source nested group height extension and span-before-group painting remain open. Evidence: .reference/sequence-group-layout-20261007/source-comparison.md and qualification.json. This does not qualify full sequence layout or wider API/platform parity; bundled TALA remains in scope, plugins excluded.

Checkpoint506 maps original d2sequence group descendant partition, constants and d2svg.sortObjects to nearest-sequence render roles and complete stable shape/connection paint ordering. Top-level group descendants retain labels; grids above an owning sequence do not override its spans/groups. Four complete Source target inventories qualify20object roles, visible labels and shape order; the actual original-Go private sorter confirms nine mixed objects. All149renderer+153export tests pass0ignored. Focused sequence2/7→4/7, authored22/24 and broad149/159 with no regressions; actor-note layout and broader transient role/API/PLAN contracts remain open. Evidence: .reference/sequence-group-render-roles-20261007/source-comparison.md and qualification.json. D2-derived changes retain MPL-2.0 notices. Bundled TALA remains in scope; plugins are excluded.

Checkpoint508 maps original Object.hasEdgeRef/ContainedBy, IsSequenceDiagramNote/Group and GetFill into source reference-state fill and role selection. A glob-expanded top-level actor can retain rectangle geometry while choosing note fill N7; rendered edge incidence is insufficient. Seventeen complete original-Go controls qualify55object records and one compiler diagnostic, and all150renderer tests pass. Original20note-order controls now20/20exact, quoted-asterisk/glob-descendant controls match and earlier24authored controls remain exact; new geometry/error failures remain recorded. Evidence: .reference/sequence-reference-note-fill-20261007/source-comparison.md and qualification.json. D2-derived changes retain MPL-2.0 notices; wider original-v0.9.0 goal active, bundled TALA included and plugins excluded.

Checkpoint509 maps original addLifelineEdges source-sorted final message and note partition to the metadata-free renderer fallback, preserving extracted-grid and owning-scope boundaries. The complete fallback fixture verifies all29Source synthetic routes across six target inventories, including16actual tied messages. All151renderer tests pass;12new actual Source/official/native PNG controls match and earlier24authored controls remain exact. Expanded controls26/34→28/34fix both explicit-after-glob cases; remaining error/variable differences are recorded. Evidence: .reference/sequence-lifeline-source-order-20261007/source-comparison.md and qualification.json. General metadata-free APIs and full ordering remain unqualified. D2-derived changes retain MPL-2.0 notices; existing Go sort translation retains BSD-3-Clause notices.

Checkpoint511: Original exporter copies live Object.LabelPosition even for group predicates under extracted grids. Native now honors the live position, while actual sequence layout continues to commit top-left. The measured regression qualifies the complete14label coordinate/font inventory across two original-Go targets;152renderer tests pass. Evidence: .reference/sequence-extracted-grid-20261007/source-comparison.md and qualification.json. Full text/style/routing, structured-header and broader PLAN gates remain open. Existing MPL-2.0 and third-party notices are preserved.

Checkpoint513 maps original d2svg.go:1326 and d2scenebuild/labels.go:196 ordinary connection LabelFill to the rounded label-background sibling outside route/arrow masks. Edge gradient fills contribute actual definitions. Original exporter.toConnection copies Link without PrettyLink, so authored edge URL metadata does not add an appendix row; tooltips retain their original appendix path. Empty labels produce no text/background. The measured complete-target fixture qualifies eight accepted controls and one original compiler rejection, with both public/native background inventories and positions checked. All153renderer tests pass. Eighteen fresh Source/official/native authored outputs/diagnostics match across themes0/7. Evidence: .reference/raster-bands-20261007/ and tests/fixtures/connection-label-fill-source-v090.json. Existing MPL-2.0 notices retained; broad PLAN gates remain open.

Checkpoint514 maps original `d2scenebuild/connection.go:430–600` polygon
arrowheads and `d2target.Arrowhead.Dimensions` to `raster_polygon_arrow`.
Native raster submission retains local float64 commands, source-shaped endpoint
matrix and explicit cap/join/miter contracts for arrows, hollow triangles,
lines, diamonds and boxes. Public SVG retains normal marker representation.
The MPL-2.0 virtual original probe in `tools/probes/polygon-arrowheads-20261007.go`
produces210 complete source primitive/transform controls; the permanent
`polygon-arrowheads-source-v090.json` regression verifies coordinates and
contracts and ARM64 matrix bits. Evidence and remaining platform/raster gaps
are in `.reference/arrowhead-raster-20261007/source-comparison.md`.

Checkpoint515 maps original `d2target.Diagram.BoundingBox` outside-icon extent
and `lib/shape.NewShape` case-sensitive fallback to `outside_icon_envelope`.
Lowercase target types resolve to the square fallback at that original call,
so bounds use the complete outer box rather than shaped content insets.
The MPL-2.0 original target overlay `tools/probes/outside-icon-bounds-20261007.go`
records96 complete bounds for eight types and twelve canonical outside
positions. The permanent `outside-icon-bounds-source-v090.json` regression
checks the full min/max box. Initial malformed control spellings, raw failures
and final CLI/raster evidence remain in the checkpoint515 evidence directory.

Checkpoint516 maps private class TextRun origins to original Go v0.9.0
`d2renderers/d2scenebuild/special_shapes.go:buildClassContents/buildClassRowNodes`
and `lib/label/label.go:GetPointOnBox`. Native class text carries exact float64
origins and owner-unique identities into the existing Source paint-reference
contract (`d2raster/raster.go:drawPrimitive`, `d2raster/bands.go:setRasterizerReference`).
Public SVG retains its existing coordinate formatting. The virtual original-Go
probe `tools/probes/class-text-origins-20261007.go` supplies 12 complete class
controls and 165 TextRuns in `tests/fixtures/class-text-origins-source-v090.json`.
Full raster and broad-regression evidence is retained under
`.reference/slow-grid-raster-20261007/`; scoped checks do not qualify the entire
structured-shape or rendering API.

Checkpoint517 maps original `d2scenebuild/latex.go:latexShapeLabelTopLeft`,
`buildLatexLabelNode` and `d2svg.go` math-label placement to intrinsic SVG images.
Unset positions use the inner box center; border positions use the inner box,
outside positions the outer box. 3D/multiple expansion and font-color do not
change math-image placement/color: original uses shape stroke and intrinsic
asset dimensions. `tools/probes/math-label-boxes-20261007.go` supplies84 original
accepted scene geometry controls. Private native image metadata retains exact
origins; public SVG uses unscaled intrinsic MathJax units at16px font reference.
Original `d2graph.Object.GetLabelSize` ignores surrounding font size for TeX;
CLI and WASM math measurers follow that contract. General Native TeX rendering
still has inherited formula-specific routes and approximate fallback layout;
those are explicit incompatibilities, not a general MathJax parity claim.

Checkpoint545 (2026-10-08): Original d2scenebuild/build.go rich-label UTF-8 validation precedes assets.go visibility-filtered math import. Retained graph node bytes now supply validity; literal U+FFFD remains valid. Strict160 CLI controls improve48 to128 with80 gains/0losses;32 earlier Markdown compiler diagnostics remain failures. Evidence: .reference/raw-label-native-20261008/source-comparison.md and progress-bindings.json.

Checkpoint548: markdown_wrap.rs maps original lib/textmeasure/markdown_svg.go wrapInlineTokens, atomizeInlineTokens, appendInlineTextAtoms, inlineUnbrokenWidth, inlineTrailingPaddingWidth and splitInlineTokenAtWidth. lib.rs paragraph painting preserves original code/decoration ownership and concrete/synthetic faces; whitespace ownership follows markdown.go normalizeMarkdownWhitespaceTree. All720 original paragraph serialized SVG geometry/face/link contracts match. Broader SVG pixels remain partial; colors and full primitive f64/API contracts are not implied.

Checkpoint549 maps original MarkdownColorAccent to active B2 text paint and to B2 for existing synthetic-bold scene strokes. Fresh96 source/official/native captures match24,12 gains versus frozen548 and0 losses. Prior480 improves382 to394 with0 losses; broad remains157/159. Full nested-emphasis/code/font and legacy natural measurement contracts remain open.

Checkpoint550: markdown_measure/legacy_inline.rs maps original markdown.go measureMarkdownNode text-node sizing, hasPrev/hasNext space context, concrete font selection, inline box endings, code legacy padding and paragraph margins. Complete396 natural width/height controls match,124 gains versus frozen549;161 render tests pass. This does not certify all block trees, arbitrary HTML repair/custom fonts or complete LayoutMarkdown primitives/colors/API/errors.

Checkpoint551: inline code padding/text ownership, code owner href/title, decoration entry colors and nested group ownership map original markdown_svg.go collectInlineTokens/paintInlineNodes/childContext/SVG. Complete2070 original paragraph geometry/font/text/fill/link/title contracts match;161 renderer tests pass. Low-level paragraph API checks do not certify arbitrary HTML repair/custom fonts/full primitive floats or other blocks; raster gates are separate. Source comparison: .reference/markdown-code-spans-native-20261008/source-comparison.md.

Checkpoint551 strict pixels: fresh 630/816 versus frozen550 234, 396 gains and zero losses; prior natural 418 to 442/480, 24 gains and zero losses. Other prior/broad scopes unchanged. Source rich-label titles also feed addMarkdownLinkRegion/addAppendixItem in d2renderers/d2scenebuild/markdown.go; this native static-export appendix gap remains open. Current saved inventory verifies 1,751/2,031 scoped cases; none of these counts imply overall parity.

Checkpoint552: draw_native_markdown_layout_viewport maps original LayoutMarkdown natural paint width and d2svg markdown.render target viewport. All 90 source SVG paint/viewport contracts match (36 gains versus prior forced-width path); 162 renderer tests pass. The 3,248 retained original scene link/appendix/budget controls are source-only investigation. Current saved raster inventories verify 2,071/2,895 scoped cases with zero gains/losses; valid table captures remain 0/192 and TALA authored-zero errors are missing natively. Source comparison: .reference/markdown-scene-links-native-20261008/source-comparison.md. Nearly-full goal remains active; wider APIs/build/resources/PLAN are not certified.

Checkpoint870 maps code_shape.rs line origins to original
d2scenebuild/code_shape.go::buildCodeTextRuns. Compute lineAdvance once, then
fused lineIndex*lineAdvance+origin.Y, then add font size. Original ARM64
buildCodeTextRuns assembly and36 direct scene-builder captures establish this
order;206 renderer tests include bit-exact x/y and text for every captured run.
Evidence: .reference/code-line-baseline-20261016/. Raster/other compatibility
contracts and full D2 parity remain separate.

Checkpoint871 follows d2scenebuild/code_shape.go owner/line/token scene identities.
Native code precision IDs include the owning node or edge and local token address;
byte lengths of independently rendered buffers are not object identities.
Source-preserving glyph traces and captured native SVG prove three aliases in the
retained blank-line graph. A permanent independent-node/edge fragment test guards
uniqueness. Evidence: .reference/code-glyph-frame-20261016/.

Checkpoint872 maps SQL table text to d2scenebuild/special_shapes.go
buildSQLTableContents, lib/label GetPointOnBox and d2svg/table.go. Native SQL
header/name/type/constraint runs retain raw text and exact f64 origins under
owner/row/role identities. Row boxes advance cumulatively and center text before
adding the three-quarter-font baseline offset.48original scene Build captures
cover336TextRun origins/text across row counts0/1/2/5,heights99/101/512,positions0/21
andfonts16/20. Evidence: .reference/structured-height-raster-20261016/. Complete
populated SQL API metadata, broader shapes/fonts/effects and D2 parity remain open.

Checkpoint 875: Serialize typed SQL columns in target hash input in original struct field order; empty slices remain null. Source: pinned d2target/sqltable.go,
d2target/d2target.go::Text, d2compiler.compileSQLTable,
d2graph.Object.GetLabelSize/addSQLTableColumnIndices and d2exporter.toShape.

Checkpoint 875 completion: native SQL target DTO shares target hashing data;
source exporter accents, implicit intrinsic-label omission and int geometry
casts are retained. SQL scene text keeps parent font family while clearing
bold/italic (d2scenebuild.buildSQLTableContents and builder.font).

Source-byte observer identifies default structured stroke N7. SQL header ink
follows Shape.GetFontColor: semantic text colors use target stroke, literal
font colors retain precedence. Permanent30-case complete SQL fixture retains
28positive shapes and2source color-rejection outcomes.

Checkpoint 876: Typed complete native class target fields/methods, including one-prefix visibility stripping, underlines and source nil-slice behavior; same struct serialization order as d2target/class.go. Shape projection remains shared with Diagram.Bytes. Source: d2compiler/compile.go::compileClass, d2target/class.go, d2exporter/export.go and d2js/js/index.d.ts at pinned D2 v0.9.0.

Checkpoint 876: Publish source cloud contentAspectRatio and prettyLink in original struct order, including Go JSON invalid-byte escapes for truncated Unicode. Correct source c4-person name, sequence type/default stroke width/blend and ordinary default label position. Existing graph/scene data reused, no Go/JS algorithm paths.

Checkpoint 876: Publish source sequence shape/connection z-indices and synthetic lifelines after authored edges, using actual restoration order, identities, routes and actor stroke/dash inheritance. Sources: d2sequence.addLifelineEdges/LifelineEndID/constants, d2exporter.toConnection, d2target.Config.Data. Replace rejected v1 JSON escape prototype with actual pinned Go1.27 v2 string encoding.

Checkpoint 876: d2scenebuild/appendix.go::addAppendixItem validates raw prettyLink UTF-8 only when Appendix is enabled. Preserve normal source hashing/public JSON replacement while rejecting truncated invalid source strings in checked SVG/native raster appendix paths. Two original Go CLI errors supply evidence; valid replacement-rune text remains accepted.

Checkpoint 877: class sketch draw order and mono text follow pinned original Go d2scenebuild/sketch_shapes.go::buildSketchStructuredShape, compileSketchRectangle, compileSketchLine and special_shapes.go::buildClassContents. Rough scene operands follow d2sketch/scene.go::finiteRoughData (six-place truncation). Each drawable uses seed 1 and bowing 2. Header-only streaks reuse the source texture painter. Twelve direct public Build controls retain all 60 rough paths, 84 text runs and 12 streak boxes in tests/fixtures/class-sketch-scenes-source-v090.json. Reproduction source/module and raw outputs: .reference/class-sketch-20261016/source-module and source-scenes.json. Broader structured effects, SQL sketch and budgets remain open.

Checkpoint 878: root and connection projection shared between native hash records and public WASM targets. Original d2exporter.Export/applyStyles/toConnection, d2lib.Compile/applyConfigs/applyDefaults, d2target.Text/Diagram/Config and d2graph.SetDimensions supply defaults, omitempty, source classes, arrowhead Text records, fill/icon metadata, theme mono/no-corner/C4 defaults and config pointer semantics. Native connection routes retain source Point.TruncateDecimals then TruncateFloat32; the old target/raw-route equality assertion is replaced with 40 complete independently captured Go connections. Raw candidate and expanded Go API evidence: .reference/native-diagram-20261016. Wider API fields, legends, direct target rendering/editing and themes remain separately qualified or open.

Source continuation: d2graph.Object.GetLabelSize and SetDimensions override the inherited primary family with D2_FONT_TO_FAMILY when style.font is explicit. DEFAULT therefore measures Source Sans Pro even in a mono theme; only absent font inherits mono. Retain source config pad/center/layout-engine pointers on Graph alongside config data for Diagram.Bytes. Fresh Go empty-target byte controls qualify non-renderable empty diagrams directly; missing SVGs are not hash/pixel passes.

Checkpoint 879: preserve non-nil empty edge URL and icon position at the target exporter boundary (painting still uses the nonempty accessor); retain node integer iconBorderRadius with source omitempty semantics. Share pinned d2exporter.applyTheme/toShape C4 defaults between native paint and target serialization, with explicit source overrides authoritative. compileReserved applies icon style only with a scalar icon in that class/direct map; root IconStyle remains absent. Code GetFill defaults to N7, while text applyStyles defaults transparent. Original-Go full-diagram controls: crates/d2-rs-wasm/tests/fixtures/target-icons-c4-source-v090.json; raw27 source requests/results and oracle bindings in .reference/native-target-icons-c4-20261016. Legend and ELK route frontier remain separate failures.

Checkpoint 879 radius paint: d2scenebuild/assets.go::buildShapeIcon/buildShapeImage/buildImageNode/roundedBoxClip/imageShapeClip and build.go::clampBorderRadius. Ordinary icons use arc clips, image shapes use smooth cubics with endpoint first controls, both clamp to half the smaller box dimension. Structured icons ignore the radius. Six self-contained source/native PNG controls include zero controls, radius clamp and class ignore; raw files/scripts in .reference/native-target-icons-c4-20261016.

Checkpoint 880: edge icon placement follows pinned d2target.Connection.GetIconPosition and lib/label.Position.GetPointOnRoute/chopPrecision on exported truncated/float32 routes. Visible labels retain their label-left gap and vertical centering; empty text uses requested icon placement. Reuse the source native arc clip transport/clamp for connection icons instead of CSS inset. Direct64 Go position records, including absent icons, unsupported positions, fractional/negative routes, empty-label precedence and unlocked label positions: tests/fixtures/edge-icon-positions-source-v090.json and .reference/edge-icon-route-20261016/source-module. Source/native raster matrices remain separate qualification gates.

Checkpoint 881: native full legend target projection reuses ordinary compiler graph/styles/labels and typed exporter records. Sources: d2compiler.compileLegend/compileMap/setDefaultShapes, d2exporter.Export/toShape/toConnection. Legend graph stays unmeasured with source fixed boxes/routes; transparent objects are filtered at export without deleting edge endpoints. The source legend graph boundary stops ambient sequence semantics; host theme is applied only at export. Public24 Go cases include structured entries, styles/title, themes/sketch, boards, global classes, opacity and host-sequence scope; permanent fixture in d2-rs-wasm/tests/fixtures/legend-target-source-v090.json. Rendering/paint qualification remains separate.

Checkpoint 882: legend layout measurement/placement follows pinned d2scenebuild/legend.go::measureLegend/expandViewBox/buildLegend/legendRenderedText and d2graph.GetTextDimensions. Measure unstyled 14px rows using their explicit source font with Source Sans Pro fallback, preserve raw measurement strings, collapse only ASCII whitespace for painting, retain integer row halves and .3/.2 baseline offsets, connection-array presence, empty-label card behavior, and asymmetric padded canvas expansion before root strokes. Sixty-three direct public Go scene outcomes supply complete panel boxes, canvas boxes and label origins across default/mono/hand, multiline/control/wide text, empty rows and pad0/4/100. Source compilation now retains implicit/nested rows in ordinary compile order. Full legend shape/connection style/structured rendering, effects, asset admission and source errors remain separate work.

Legend source continuation: theme mono overrides shape row measurement at export; explicit edge font follows mono theme. Unknown target font aliases fall back to sans. Legend icon geometry uses 120×120 source boxes scaled1/5 with source default stroke2. Paint the fixed gray shadow/white panel/border and black text; native text transport retains the collapsed string and exact independent start-anchor origins. Generic shadow filter transport remains subject to strict raster qualification.

Checkpoint 883: legend connection icons use pinned d2scenebuild.legendConnectionIconTarget/buildLegendConnectionIcon: a48px route with empty endpoint-shape map, source endpoint trimming, all resolved arrow kinds, half-scale route/arrow geometry, round cap/join, source dash values and combined geometry opacity. Reuse ordinary Rust source-mapped arrow builders and their raster command transport instead of dangling SVG marker references. Retain original connection IDs, icon presence (including empty URL), endpoint labels, link/tooltip metadata and source preflight priority; empty main labels skip admission/painting. Public Go384 scene controls cover16 arrow kinds, widths0/1/2/4, source/target/both and static/animated tracks; tests compare route primitives, stroke/dash operands, parent transform and paint-leaf inventory. Raw full arrow/animation nodes remain evidence; these component checks do not assert complete pixel parity. Nine original-Go CLI cases qualify metadata errors and empty-label exceptions separately.

Legend shadow continuation: source buildLegend emits a gray rounded rect with black alpha26, sigma3 and offsetY2, then the white panel. Native raster SVG now carries an exact, qualified scene-shadow group and source rounded-box operands. Public SVG retains an equivalent filter; native export can avoid falling back for the entire board. Marker admission is limited to the known native canvas, unique identities, exact filter/child/paint attributes and matching panel geometry.

Legend shape scale continuation: give the source one-fifth geometry wrapper a stable native identity so the existing qualified group-transform transport retains literal0.2 as f64. The existing named-group transport keeps that operand distinct from usvg f32. Plain draw_shape lacks the ordinary object shape CSS class; explicitly inherit source buildShape CapButt/JoinRound on the legend geometry wrapper instead of SVG default miter joins. The repeatable16-pixel rectangle-corner difference remained after naming the transform and is separately checked by fresh full PNG captures. No fixture-specific positions or output overrides.

Canonical sketch SVG continuation (2026-10-18): sketch_svg.rs and
sketch_connection.rs follow original d2renderers/d2sketch/sketch.go Connection,
Arrowheads, cf* methods and rough-go0.2 Path. Local arrow paths retain source
six-place truncation, empty fills, whole-result unsupported-arrow behavior,
float32 Vector.Radians before degree conversion, and shared RNG across crow's
feet subpaths. Animated SVG uses original lib/svg/path.go SplitPath, including
chord-based selection and fixed cubic half-split, with source dash styles.
Canonical shape outlines use butt caps, compound local-coordinate streak paths
and exact SVG alpha; native scene paths retain round caps, split primitives and
quantized native alpha. Original d2fonts_embed.go maps HandDrawn italic to Fuzzy
Bubbles regular and d2svg emits no italic style: the SVG override avoids browser
synthetic slant without changing native raster transport. Direct Go fixtures
cover69 connection/arrow/animation controls and an actual rectangle/pattern SVG.
Full-browser comparisons are explicitly viewBox-viewport scoped; default
responsive-root sizing and the Dagre curved-graph layout divergence remain open.

Responsive SVG sizing continuation (2026-10-18): RenderOptions.scale is an
Option<f64>, matching original d2svg.RenderOpts.Scale pointer presence. Absent
public-SVG scale omits outer width/height; explicit scale retains source ceil
rounding and the unscaled viewBox. Native raster transport always has intrinsic
canvas dimensions and defaults absent scale to one. Native font-role tests use
that transport, since original native FontRef styles differ from public SVG's
concrete Fuzzy Bubbles face selection. Strict browser image failures remain
failures even where dimensions agree or identical source bytes vary between
captures. The responsive checkpoint records both complete capture attempts.

TALA step-edge publication (2026-10-23 run 12) follows pinned
`d2layouts/d2talalayout/patch.go:158–239` and
`internal/grouping/sequences.go:78–137`: sequence-defining connections are
permanently absent from the published edge inventory. Native LayoutResult
carries explicit `removed_edges` while retaining stable input arena IDs;
cache restoration and nested layout reinsertion remap these IDs alongside
routes. Target JSON/hash/SVG omit those edges, and the legacy SVG route gate
still rejects unrelated missing routes. Native D2 JSON publication uses the
same explicit inventory. Witness: `tests/regressions/e2e/tala-step-edge-removal.d2`
(exact pinned-Go SVG); scoreboard targets are TALA
`regression/nested_steps` (now exact) and `stable/investigate` (now renders;
geometry still differs). MPL-2.0 notices remain applicable.

## HandDrawn font without sketch (2026-10-23, Claude)

`RenderOptions::hand_drawn_font` mirrors d2lib `CompileOptions.FontFamily =
HandDrawn` without `RenderOpts.Sketch` (d2renderers/d2svg/dark_theme/
dark_theme_test.go:429–440): the exported font family is HandDrawn while the
hashed config keeps `sketch: false`. Witness: the d2svg dark_theme tier, 12/12.

## Raster canvas and border radius follow the Go target (2026-10-24, Claude)

- Outside node labels enter the canvas bounds as `int(labelTL)` plus the int
  label dimensions (d2target.go Diagram.BoundingBox), truncating toward zero
  like the inline label branch; flooring added a pixel
  (`txtar/width-smaller-than-label`, 604 vs 602 px).
- Rectangle border radius uses d2scenebuild clampBorderRadius (build.go:1121),
  `max(0, min(r, w/2, h/2))`, emitted as both rx and ry. SVG's per-axis
  clamping turned `border-radius: 999` into an ellipse instead of a stadium
  (`stable/border-radius-pill-shape`, `stable/array-classes`).
Raster scoreboard 866 → 875/943, zero regressions; e2e SVG unchanged at 933.
- Class and sql_table shapes round like d2scenebuild buildStructuredShape:
  the outer rect takes the clamped radius, the header is
  structuredHeaderPrimitive's path (top corners curve with min(r, w/2, h),
  all four when the header fills the shape; special_shapes.go:277–308), and
  the last SQL row rule / bottom class rule is inset by the radius
  (special_shapes.go:146–148, 263–265). `stable/class_and_sqlTable_border_radius`
  becomes pixel-identical; raster scoreboard 875 → 878/943.
- Fill-pattern overlays follow d2scenebuild interleavePattern/overlayPatternNode:
  every overlay carries the multiply `<pattern>-overlay` class (the 3D main
  face had lost it); `multiple` shapes pattern only the front copy
  (effectPatternNode), including multi-path shapes whose base and overlay
  interleave per path; class and sql_table draw their plain base paint, then
  a pattern overlay on the outer box and header only (structuredPatternNode),
  with row rules left unpatterned. Witnesses `themes/3d-sides`,
  `patterns/{multiple,real-lines}`, `themes/origami` (dagre); raster
  884 → 890/943.
- Animated connections with the same arrow state on both ends split at their
  chord-length midpoint for the raster path, as d2scenebuild buildConnection
  and splitAnimatedConnectionPath do (connection.go:96–123, 213–308): a
  reverse-animated first half with the source marker, a forward second half
  with the target marker, so the dash pattern restarts at the split. The
  exact raster sources split the same way (L lerp, C bisected at 0.5, S kept
  whole; provenance from the SVG tokens). Raster 890 → 901/943.
- Gradient-valued strokes and font colors now get gradient definitions and
  url() paints in the raster SVG (d2svg and d2scenebuild paint all three);
  they were emitted as raw CSS strings and silently dropped. `txtar/gradient`
  now shows its gradient border and label; it still differs by gradient-text
  interpolation and label antialiasing, so the board does not yet match.
- Markdown headings wrap through the shared wrapInlineTokens port, as
  markdown_svg.go paints them with paintInlineNodes: per-word atoms with
  separately measured spaces, not one kerned run. Heading inline code keeps
  PaddingLeftRight_heading_code_em (.2) of the heading size
  (inlineCodeHorizontalPadding), now an explicit wrap parameter.
  Witness `regression/markdown_em_dash_heading_2647`; raster 901 → 904/943.
- Outside Markdown labels (a person's default outside-bottom label) are placed
  with GetPointOnBox(label.PADDING) on the shape box, as
  markdownShapeLabelTopLeft does; they had been pinned to the shape's top-left
  corner, and the unset→inside-middle-center coercion now keys on the
  exported position. `txtar/md-label` 53147 → 4 differing pixels (TALA match).
- Connection routes default to radius 0 under NoCornerRadius themes
  (terminal, terminal grayscale, origami), as d2exporter toConnection exports
  (export.go:381–386); the raster path always rounded with 10. Witnesses
  `themes/{terminal,terminal_grayscale,origami}` (ELK, TALA); raster
  911 → 917/943.
- Raster gradients keep d2scenebuild linearGradientVector's unrounded degree
  end point, `(90-d)*π/180` (color.go:110–150). A single-line label's
  font-color gradient is restated in user space over the label's TextRun.Ink
  box (build.go:1008–1022: label top-left, LabelWidth by one line advance)
  instead of the glyph bounding box. Animated shapes carry
  `isolation:isolate` in raster mode (shape_effects.go finishShape). An unset
  label position leaves the label at the inner box's top-left, because
  label.Position.GetPointOnBox has no case for it (shapeLabelPlacement,
  labels.go:133–157). Witnesses `txtar/gradient`, `txtar/shape-animate`,
  `stable/text_font_sizes` (ELK).
- Non-default flag rendering (found by a board x flag probe against pinned Go):
  a mono theme replaces the sketch HandDrawn primary family with Source Code
  Pro, as d2exporter.Export applies SpecialRules.Mono after d2lib selects
  HandDrawn (export.go:32-37); arrowhead labels use that primary italic face
  ("default" family, d2scenebuild labels.go:236-270), so the terminal theme
  draws them in Source Code Pro; and the raster path's sketch tables follow
  d2scenebuild (HeaderPadding 10, CapRound/JoinRound outlines from sketch.go)
  while the target SVG keeps d2sketch.Table's 20px inset and default caps.
  Witnesses: `stable/arrowhead_labels --theme=300` (SVG with --sketch, PNG),
  `txtar/sql-table-reserved --sketch`, `stable/sql_tables --sketch` (PNG).

### D2 master 6a7b48378 SVG ids (2026-10-24)

- `lib/svg/markup.go` ScopedID → `d2svg::svgtext::scoped_id`: border-mask
  ids keep a safe shape id (`[A-Za-z0-9_.:-]`) and encode anything else, or a
  value starting with `d2e-`, as `d2e-<SVGID>`. Witness
  `stable/grid_outside_labels` (all engines).
- `d2svg/table.go` borderRadiusClipPathID: class/table header rects reference
  the clipPath by its encoded id (v0.9.0 referenced the raw shape id).
  Witness `stable/class_and_sqlTable_border_radius` (all engines).

### D2 master 6a7b48378 gradient contracts (2026-10-25, iso18)

- `lib/color/gradient.go:105–142` (`parseColorStops`,
  `canonicalGradientStopPosition`) → `d2svg/color.rs`: reject malformed
  entries and non-finite/non-numeric positions; emit shortest Go 'g' numbers,
  preserving the percentage suffix. The shared Go JSON float encoder supplies
  correctly rounded shortest digits before the 'g' exponent rule is applied.
- `gradient.go:152–174,238–252` (`LinearGradientToSVG`,
  `RadialGradientToSVG`) → the same module: Go `html.EscapeString` for IDs,
  caller-provided positions and stop colors, including constructed Gradients.
- Public `color::{Gradient,parse_gradient,gradient_to_svg}` exposes this same
  native implementation. Parsing returns `None` for the Go error boundary;
  exact Go error text is retained in the source capture, not claimed by this API.
- `tools/color-scoreboard/hook_test.go` observes all calls from the two new
  master gradient tests and the existing one-stop regression. The original
  test assertions execute unchanged in the manifest's trace checkout. Native
  ValidColor observations use the existing compiler's quoted fill consumer.
  Fixture: `tests/fixtures/color-api.json` (16 calls); original functions and
  source hashes are bound by `scripts/color-scoreboard.py`.

### D2 master 6a7b48378 typed SVG markup (2026-10-25, iso19)

- `lib/svg/markup.go:15–324` and `text.go:11–53` →
  `d2svg/markup.rs`, exported as `d2_rs_render::svg`: private attribute
  fields, validated non-event names, ordered escaped values, duplicate
  rejection, attribute parsing, local IDs/IRIs and text/trusted fragments.
  Byte-valued constructors preserve invalid UTF-8 until XML serialization;
  Go's one-byte invalid-rune replacement and XML character rules apply.
- Existing renderer `svgtext::scoped_id` delegates to the same ID constructor;
  text serialization reuses the existing `svgtext::escape_text` implementation.
  Parse failures use a Rust error; captured Go error text remains available,
  while the original tests assert the error/panic boundary rather than spelling.
- `tools/svg-api-scoreboard/hook_test.go` observes 51 original calls across
  all eleven master-added `lib/svg/text_test.go` functions, keeping original
  Go assertions active. Attribute bytes use base64 transport without repair.
  Reproduction: `scripts/svg-api-scoreboard.py`; fixture `tests/fixtures/svg-api.json`.


## Master raw-target SVG paint and link admission (run 67)

`d2svg/paint_validation.rs` translates pinned `6a7b48378`
`d2renderers/d2svg/paint_validation.go:15–161` and `lib/color/color.go:514–531`.
The color-name table mirrors that source; gradient stop parsing reuses the
existing `color::parse_gradient` and native csscolorparser port in d2-rs-math.
`render.rs` checks ordinary links before paint/geometry, following
`d2svg.go:2866–2900`; `legend.rs` follows RenderLegend's link check at
`d2svg.go:259–267`. `theme_css.rs` checks both override sets as ThemeCSS does
at `d2svg.go:3297–3302`.

`scripts/svg-admission-scoreboard.py` captures eight original test functions
with all Go assertions active: 60 public Render/RenderMultiboard calls.
The hook snapshots input JSON before rendering and retains complete output
bytes, nil outputs and error strings. The Go test binary uses the ordinary
release version linker stamp; no SVG/error normalization is applied.
`tests/fixtures/svg-admission-api.json` binds the same inputs, exact errors,
and SHA-256 of successful SVGs for the native example regression test.
Structural raw-target validation, cycles and isometric d2svg dispatch remain
separate queue items; this finite tier does not establish full target admission.

## Master raw-target structural admission (run 68, 2026-10-25)

`d2svg/target_validation.rs` translates upstream master `6a7b48378`
`d2target/render_validation.go:23–437` into validation of the owned target
model. `d2svg.go:2866–2905,3575–3635` supplies the single/multiboard entry
order: validate the entire tree before recursive hashing; ordinary single
renders check links first. Legend checks project the fields that survive
legend painting. Route length, cubic arity, null points, arrowheads, numeric
bounds, dash geometry, image icons and family admission report source errors.
`render_validation.go:199` uses rounded `dx*dx` plus fused `dy*dy` on the pinned
ARM64 compiler (assembly command and excerpt in `run68-asm/`).

Missing and empty family names use the same font defaults as
`d2svg.go:2492–2500`; Markdown's family selection follows that default too.
The original five JSON-representable test functions in
`render_target_validation_test.go` are observed with their assertions active.
Complete errors/nil outputs and successful SVG bytes improve **4/31 → 31/31**;
27 failures fixed, zero regressions. Fixture: `tests/fixtures/svg-structural-api.json`;
replay: `svg-admission-scoreboard --suite structural`. The old paint suite
remains **60/60**. Direct Go pointer cycles, direct nonfinite inputs, arbitrary
registered custom families and deeper tree limits are not qualified by these
JSON calls. Rust's owned tree cannot represent Go board allocation reuse.
## Raw-target label serialization and LaTeX admission (run 69)

Pinned master `6a7b4837838a3907604a1d9eec57ee98b6f3cd01`:

- `d2svg.go:1079–1089,1707–1723`: shape/connection group classes pass
  through `lib/svg` attribute escaping, including caller-provided classes.
  Native: `d2svg/render.rs`, shared `markup::escape_attribute`.
- `d2svg/table.go:82–111` and `d2sketch/sketch.go:536`: SQL constraints are
  text (`SetText`), including in sketch output. Native:
  `d2svg/{classtable,sketch}.rs`, `ThemableElement::set_text`.
- `d2latex/latex_common.go:8–15`: input admission precedes the MathJax
  parse-error wrapper. Native: `d2svg/latex.rs` reuses the math crate's
  translated validator before wrapping renderer errors.

Original tests: `class_escaping_test.go`, `clip_path_escaping_test.go`,
`id_reference_security_test.go`, `sql_table_constraint_escaping_test.go`,
`latex_limits_test.go`. The `labels` suite of
`scripts/svg-admission-scoreboard.py` captures all ten original calls with
their Go assertions active and compares complete SVG/error bytes.
`tests/fixtures/svg-labels-api.json` retains requests and expected error/output
hashes for native regression tests; it is test data, never renderer input.

## Master WASM rendering boundary (run70, 2026-10-25)

`target.rs::validate_render_target` exposes the existing full-tree port of
`d2target/render_validation.go`, so the WASM caller can validate before
`GetBoard` and child removal (`d2js/d2wasm/functions.go:363–405`).
`d2svg/mod.rs::render_wasm_api` shares rendering with the existing direct
entry and follows `functions.go:518–540,545–576`: renderSingleBoard and Render
both wrap SVG render errors; animation errors use their separate prefix.
`TargetDiagram::render_wasm_svg` selects this public API error contract;
`render_d2svg` retains direct errors. This changes no geometry or SVG
serialization. The five original WASM admission calls and prior direct
raw-target calls are the regression witnesses; see the WASM crate SOURCE.md.

### Run 72: exact document options for the isometric facade

Expose the existing d2svg `RenderOpts` as `SvgRenderOptions`, its palette
override type and the flat document/admission primitives. The
`d2-rs-isometric::svg` facade owns dispatch because isometric already depends
on this crate. Source: pinned `d2svg.go:2866–2909,3575–3628` and
`isometric.go:21–96`. Existing flat algorithms and output are unchanged.
Four original functions / 13 full calls are captured by
`scripts/svg-isometric-scoreboard.py`; see isometric/SOURCE.md for scope.

Run 73 (master 6a7b48378): direct `svg_theme_css` exposes the existing
`d2svg.go:3296–3318` implementation without routing through a diagram.
Raw-target font validation now checks the shared named registry, matching
`d2target/render_validation.go:264–277`; font embedding consumes those same
faces (`d2fonts_common.go:38–61,129–209`). Original registered-font render
and five direct theme-validation calls retain all assertions, complete SVG/
stylesheet bytes and errors: `scripts/svg-font-scoreboard.py`, original Go
hook `tools/svg-font-scoreboard/hook_test.go`, and native example/fixture
`svg-font-scoreboard` / `svg-font-api.json`. No new golden-board claim.

### Lane L81: direct animation admission and missing padding

Master `d2renderers/d2animate/d2animate.go:46–57` validates the complete
target before traversing it and defaults an absent padding option to
`d2svg.DEFAULT_PADDING` (100). `d2svg/animate.rs::wrap` now follows both
steps. `TargetDiagram::wrap_svg_frames` exposes that same implementation for
direct callers; explicit padding and the existing rendering paths are shared.
The two original functions in `d2animate_test.go:11–30` retain their assertions
through `tools/renderer-contract-scoreboard/observer.go.txt` and
`scripts/renderer-contract-scoreboard.py`. The combined five-function corpus
also observes `d2scene/aspect_test.go:68–119` and
`d2latex/latex_test.go:56–100`; complete SVGs, matrices, dimensions, failure
return values and errors compare without normalization. Native regression
fixture/runner: CLI `renderer-contract-api.json` / `renderer-contract-scoreboard`.

Lane L82 follows master `d2renderers/d2sketch/sketch.go:340–367`:
`d2svg/sketch.rs` parses legacy textual attributes and delegates to
`connection_with_attributes`, rejecting renderer-owned/duplicate names and
zero-value typed attributes before drawing or SVG serialization. The typed
attribute's `Default` represents Go's zero value; serialization still rejects it.
Original `TestConnectionRejectsConflictingAttributesWithoutPanic` calls are
retained in `tests/fixtures/renderer-admission-sketch.json` and replayed by the
`target::admission_api_tests` unit module. The same test-only module replays
original direct non-finite routes and board-depth/count/hash calls using a
flat graph transport with exact float bits; no production tracing is added.
Original Go assertions and explicit unsupported pointer-graph/string cases
are inventoried by `scripts/renderer-admission-scoreboard.py`.
