# Corresponding source

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

SVG import element/attribute, namespace and metadata contracts in src/svg_import.rs
are mapped to pinned D2 d2svgimport/import.go, compile.go, gradient.go and
boilerplate.go. src/go_xml_names.rs translates encoding/xml character ranges
from the actual Go 1.27.0 oracle toolchain, with BSD attribution in
THIRD_PARTY_NOTICES.md. Full importer geometry/style/resource/limits and all XML
token diagnostics remain separate qualification work.

Metadata tree placement, leaf constraints, dc:format text and cardinality checks
map to boilerplate.go:validateNonRenderingTree/validateMetadataCardinality.
Parse-level non-space character data and style-comment admission map to
import.go:parse/hasNonXMLSpace; lexical text/CDATA boundaries retain original
Go InputOffset diagnostics despite the native parser's joined text nodes.

SVG envelope validation in src/svg_import_envelope.rs maps to pinned
d2svgimport/import.go:parse/validateXMLDeclaration and lib/svg/doctype.go.
Decoder field lookup maps to Go 1.27.0 encoding/xml:procInst, with BSD notice.
The parser shadow preserves byte positions without DTD resolution; raw XML
remains authoritative for diagnostics. General malformed-token and non-ASCII
ISO-8859-1 transport qualification remain open.
The earlier imageasset resolver envelope stage maps separately to
lib/imageasset/probe.go:probeResource/validateSVGWithLimits, retaining its
error wording and precedence before strict SVG importer admission.

Raw SVG asset XML validation in src/svg_asset_xml.rs maps to Go1.27.0
encoding/xml:rawToken/name/nsname/attrval/text/Token/popElement and D2
imageasset:validateSVGWithLimits. It retains Go lexical/entity/stack admission
without applying roxmltree namespace/duplicate-attribute restrictions. General
codec probing, byte charset transport, budgets and importer token/geometry
compilation remain separate qualification stages.
Go-compatible expanded token events in src/svg_asset_xml.rs map to
encoding/xml:Token/translate/pushNs/popElement. The strict importer token
preflight in src/svg_import.rs maps to d2svgimport:parse/elementName/attributes/
validateNamespaceDeclaration. It preserves duplicates, declaration order and
namespace scopes before native document-parser admission.

SVG numeric geometry and viewport admission in src/svg_import_geometry.rs maps
to pinned D2 v0.9.0 d2svgimport style.go parseSVGNumber/parseSVGLength and
parseMathJaxEXLength, geometry.go compileGeometry/lengthAttribute/radiusAttribute,
viewport.go viewport/nestedViewport/parseViewBoxFor/parseAspectRatioFor, and
d2scene aspect.go AspectRatioMatrix. These D2 sources retain MPL-2.0 notices.
This implements scoped admission and error ordering; it does not claim complete
style, geometry construction, resource, streaming or cancellation parity.

Renderer-only adjacent SVG attribute transport in svg_asset_xml.rs uses the
same reachable Go1.27 encoding/xml rawToken/attrval lexical rules as admission.
svg_precision.rs rewrites only embedded SVG image payloads for the renderer,
inserting missing separators without changing names, namespaces, values or
other token bytes. Original assets remain authoritative for Source diagnostics.
The Go-derived lexer retains BSD-3-Clause attribution alongside MPL-2.0.

SVG resource codec preparation in src/svg_asset_codec.rs maps to pinned
lib/imageasset/probe.go probeResource/expandDeclaredSVG/looksLikeXML and
resolve.go readBounded. Brotli reader boundaries and structured error names map
to andybalholm/brotli v1.2.2 reader.go/decode.go (MIT notice retained). All
codecs execute native Rust; source/metadata transport, cancellation and complete
resolver/cumulative budgets remain separate qualification work.

Host fallback's `font_cmap.rs` follows pinned x/image v0.45.0 sfnt
platformEncodingWidth/supportedCmapFormat/initializeCmap and x/text Macintosh
encoding. `font_fallback.rs` now requires the outline-parser-selected glyph to
agree with the shaping parser before admitting host coverage, following
d2fonts:probeFontFileCoverage and fontface:SupportsRenderableRune. Thirteen
pristine supported/unsupported encoding controls are retained under
`.reference/grid-edge-investigation-20261006/`. Full glyph renderability,
whole-scene discovery/limits and grapheme-affine shaping remain open.

Native mixed-font shaping in font_shaping.rs follows D2 fontface/shaping.go:
resolveCluster, splitInputsByAssignedFace, orderShapedRunsLTR, glyph-zero
replacement and fixed advance/offset placement. Default-ignorable ranges map
to fontface/unicode.go. Script subdivision/delimiters, nominal shaping scale,
zero mark widths and GPOS anchor rounding follow go-text/typesetting v0.3.4.
The scanline adapter reshapes mixed-font uniform source chunks, retaining
source font choices and its own fixed transforms. Forty-four bundled-font
and five macOS host-font glyph controls are retained. Font renderability,
resolved-language/Unicode-version completeness, complex style chunks/vertical
text/color glyphs, full budgets/cancellation and complete frame qualification
remain separate work; these controls are not full text/D2 compatibility.

Color placement now follows d2raster/text.go positionGlyphs/prepareText's shared
monochrome/color advances and anchor offsets. Supported COLR-only paint groups
map by face, glyph ID and order to native grapheme placements; primary spaces
retain their original face. Computed glyph transforms preserve stored f32 bits
rather than SVG decimal snapping. Twenty-four pristine Noto/Source Sans Pro
glyph contracts and renderer placement controls cover spaces, ZWJ, regional
flags, keycaps, variation selectors and mixed ASCII at three sizes. Paint tree
coverage/gradients remain on the prior Rust backend, and exact color pixels,
shared mixed monochrome/color paint/decorations and complete budgets/cancellation
remain open. This does not qualify full color text rendering.

colr_plan.rs translates fontface/colrv1.go's authenticated immutable bundled
COLRv1 graph compiler. Raw COLR/CPAL layouts and Float1616 parsing follow pinned
go-text tables, retaining float32-to-f64 promotion for affine fields. It resolves
palette alpha, linear/radial stops, layers, glyph outlines, static transforms,
SourceIn/SoftLight composites, clips and fixed node/depth/layer/stop ceilings.
A first-match binary lookup follows Source baseGlyphList.paintForGlyph. Native
color-group admission compiles authenticated plans; exact Source painting is
still required instead of the prior backend's best-effort paint conversion.
Every bundled glyph lookup (41,865), including 3,993 full plans with 311,086
paint nodes, 37,869 absent paints and three complete range errors, has a pristine
Source fixture. Graphs, palette bytes, clips, usage and every float bit compare;
this is graph preparation, not qualified color pixels or full D2 compatibility.

Native `colr_paint.rs` translates d2raster/colrv1.go, geometry.go,
text.go, gradient.go, color_space.go and effects.go for authenticated bundled
color graphs. It retains font-unit flatten tolerance, separate solid DrawRGBA
and sampled-gradient compositors, linear-light COLR color interpolation,
SourceIn masks, SoftLight isolation and clip coverage. `scanline.rs` consumes
native glyph placements and paints mixed monochrome/color runs in Source order.
Complete pristine frame controls cover six glyphs at four sizes and four
transforms (120 frames including the separate identity controls). Decorations,
full-font pixel coverage and cumulative scene work/cancellation remain open;
this scoped evidence does not qualify overall D2 compatibility.

Mixed color runs in scanline.rs now follow text.go prepareText's non-isolated
vector root: opacity-one monochrome parts retain the current destination's
DrawRGBA arithmetic, with a staged commit on fully supported runs. Bundled
fallback reconstruction locates authenticated already-loaded Noto bytes before
host lookup, following d2fonts/bundled_fallback.go; color classification uses
reconstructed glyphs. Empty retained cluster text no longer drops a real mark
outline. A pristine 72-frame fixture compares glyph IDs/faces/positions and full
NRGBA pixels for four mixed texts, three sizes, three background alphas and two
opacities. General decorated/complex text, full-font coverage and cumulative
scene work/cancellation remain open.

Positioned-tooltip continuation487 preserves original d2scenebuild.validateShape admission/error context. CFF outlines in mixed color text follow native fixed_glyph_path, matching Source sfnt.LoadGlyph; TrueType retains its qualified raw arithmetic. Actual384PNGmatches and96errors are bounded controls. All48Unicode PNG outputs have permanent complete decoded-pixel regressions. Evidence: .reference/positioned-tooltip-20261006/paint-source-comparison.md.

Opacity-text continuation489 follows d2raster.finishPrimitive/subpathPixelBounds and bands.setBandReferenceBounds: the parent transform applies once to local glyph ink. usvg0.48.1 flattened Group bounding_box and abs_bounding_box have distinct coordinate frames; canvas ancestry now maps the local rectangle once, while legacy absolute mode retains the absolute rectangle. All96authored fullPNG controls and72new original-Go fullNRGBA controls match; all147default export tests and25raster-harness tests pass. Fresh broad remains145/159with0regressions. Broader effect prepared-bounds transport and fullD2API/platform qualification remain open. Evidence: .reference/opacity-label-20261006/source-comparison.md.

Checkpoint496 follows original d2raster/gradient.go inverse paint transforms, linear/radial spread/cone sampling and Go1.27 ARM64 fused colorAt/lerpNRGBA order; svg_precision.rs preserves raw presentation stop offsets/RGBA before usvg shifts repeated offsets. The288 Source controls compare1728 exact inverse operands and442368RGBA samples. All148export tests pass; full121root CLI controls become exact. Evidence and remaining CSS/import/stroke/extreme/architecture/PLAN limits: .reference/root-gradient-boundary-20261007/source-comparison.md. D2-derived changes retain MPL-2.0.

Font-coverage continuation497 follows original fontface.ShapeTextTransient and go-text/typesetting v0.3.4 set_digest.go/newGPOSApplicable/lookupGPOS/applyGPOSPair2. Local Rustybuzz retains original4/0/6masks,early range saturation,cached union/per-subtable GPOS filters and explicit ClassDef2 admission;old period/ligature-specific adjustments removed.All90Source controls639glyphs/90centered origins and150exporttests pass.Fresh broad147→148/159,sole tpmjs_architecture gain0losses.Local dependency186tests pass,1Unicode-version check fails and remains retained.Complete Unicode/GSUB/contextual/device/variable/shaping work/cancellation and broaderPLAN limits open.Evidence:.reference/remaining-broad-triage-20261007/source-comparison.md.

Unicode continuation498 follows actual pinned go-text Unicode17 data,not a uniform inherited16version expectation.Local shaper ISO tags/Source CCC/mirror/composition tables/Telugu modified4/5/Hangul admission match1,112,064scalars six properties,13,253decompositions/reverse compositions and13,559explicit composition controls.188shaper+150exporttests and no-default-features librarycheck pass.Fresh148/159broad0losses.The retained16decomposition table is qualified by complete values;grapheme/bidi/emoji/ignorable/vertical/complex-font/variation/device/work/cancellation/native/WASM/platform/API and broaderPLAN remain open.Evidence:.reference/unicode-shaping-boundary-20261007/source-comparison.md.

Checkpoint500 maps original d2raster/bands.go setBandReferenceBounds: effect/clip reference bounds include complete visible paint, including filled backgrounds and images. Native group reference origin now includes their upper bounds and derives horizontal extents from visible primitives instead of empty geometry/font metric boxes. All24 filled Markdown viewport controls match complete Source raster surfaces, with original target dimensions supplied as test inputs; integrated CLI qualification remains separate. Full151export tests pass0ignored. Four new no-background text/opacity controls retain one changed pixel each. Scope/provenance/actual Source CLI and broad results: .reference/markdown-background-layer-20261007/source-comparison.md. D2-derived changes retain MPL-2.0 notices.

Checkpoint501 follows original d2raster/raster.go child-bound union before clip
preparation and bands.go prepared-bound reference transport. Exact numeric native
Markdown viewport identity accepts equivalent SVG number serialization. Unknown
effects remain unqualified. All153 export tests pass; fresh broad149/159 and
authored opacity60/64 are unchanged, with all known failures retained. Evidence:
.reference/opacity-paint-bounds-20261007/source-comparison.md and qualification.json.
This is a scoped contract correction, not nearly-full D2 qualification.

Checkpoint513 maps original d2cli/raster_png_output.go's256-row PNG bands, d2raster/bands.go:setBandReferenceBounds, raster.go:drawPrimitive and effects.go:renderEffectNode into destination-band transport. Native text/path float32 rounding retains the complete source reference independently of the band origin; mask/clip/opacity buffers use the band dimensions. Sharp-shadow geometry uses its prepared crop with an estimated simultaneous pixel-buffer budget, rather than rejecting a large final frame. Existing full native rendering remains for non-shadow trees; images/color fonts/static appendix and off-canvas sharp-shadow admission through bands remain explicit unqualified cases. Forty-two fresh original-Go complete RGBA controls qualify seven effect modes across scales1/2 and full/64/256-row renders. All154export tests pass. Evidence: .reference/raster-bands-20261007/ and tools/probes/raster-bands-20261007.go. Existing notices retained; these scopes do not qualify full rendering, bounded process memory or overall D2 parity.

Checkpoint515 maps original `d2svgimport/geometry.go` ellipse/polygon/polyline
operands and `d2raster/geometry.go:242` ellipse cubic construction to
`svg_precision::append_simple_svg_path`. Imported opaque fill primitives retain
finite float64 scene coordinates through the native path flattener. Ordinal
binding requires a complete retained primitive inventory; unsupported geometry,
resources and units retain their existing path. The MPL-2.0 original public
ImportNode/Render overlay in `tools/probes/imported-primitives-20261007.go`
produces12 complete RGBA frames at scales1/2. The permanent
`imported-primitives-source-v090.json` regression checks all frame hashes.
Source comparison and unqualified import contracts remain in
`.reference/imported-icon-raster-20261007/source-comparison.md`.

Checkpoint517 maps original `d2svgimport/style.go:mathJaxEXPixels` (8px/ex),
`d2svgimport/viewport.go`, `d2scene.AspectRatioMatrix/Matrix.Mul`,
`d2raster/image.go:vectorImage` and `d2scenebuild/latex.go:resolveLatexAsset`.
Qualified intrinsic math assets retain finite float64 primitive coordinates,
complete painted-path inventories (empty paths paint nothing), original local
transform order, original scene matrix and both asset/Image viewport clips.
Rect fraction bars retain source commands. ARM64 matrix products preserve
original FMA order. Metadata is private and finite-validated; centered outer
viewport and unresolved resource/style transforms remain unqualified.
The original Go overlay `tools/probes/math-vector-frames-20261007.go` supplies12
complete imported math RGBA frames, including empty invisible-operator paths.
These tests use original asset SVG and qualify import/raster, not Native TeX
parsing/layout. Failed candidates and CLI qualification are retained under
`.reference/grid-math-raster-20261007/`.

Checkpoint525 translates original D2 v0.9.0 d2svgimport/compile.go nested
viewport placement/content nodes and after-child clip composition,
d2svgimport/viewport.go and d2scene/aspect.go viewport/aspect matrices, and
d2raster clipping order into native float64 embedded-math path transport
and grouped raster layers. Nested viewport IDs bind clip boundaries to
their actual retained groups; overlapping children composite before their
clip mask. Source48 frame contracts (24 frames and equivalent zoomed
viewport cases) cover CD assemblies and independent overlap, nesting,
rotation, opacity and meet/slice/none controls. This is a bounded math
asset path, not completion of general SVG resources/effects/styles/text,
API/limits/work/cancellation or broader PLAN parity. Evidence is in
.reference/nested-math-clip-20261007/. Original D2 portions remain MPL-2.0.

## Checkpoint544 WIP: SVG lines and retained stroke geometry

Pinned Go mathjax tex/commands.go cancel, svg/menclose.go encloseToSVG and
D2 d2raster/geometry.go keep complete prepared coordinates and f64 transforms
for both fills and strokes. svg_precision.rs now retains finite painted line
primitives in the complete source inventory; unresolved/unpainted lines retain
the conversion fallback. scanline.rs transports the source path through direct
SVG stroke fallback and shares the precise intrinsic transform with fills.
81 actual source cancellation SVGs plus unresolved-paint correspondence tests
are permanent. Full360 library tests and25 harness tests pass;324/324 complete
source/official/native theme/scale pixels match. Prior unit replay improves
84 to88/88 with4 gains/no losses; final-candidate full bindings and broader
replay are pending. Frozen543 source snapshots/aliases remain independent.
Evidence: .reference/line-precision-native-20261008/source-comparison.md,
units-line-regression-report.json and authored-qualified-report.json. Original
D2 portions MPL-2.0; MathJax source fixtures retain Apache-2.0 provenance.
This scoped progress does not complete SVG imports, errors, or broader PLAN.

Checkpoint547 maps original d2raster preflight.node/finishPrimitive, effects.unionRect and bands.setRasterizerReference: empty SVG groups contribute no ink or reference origin. Native direct vector bounds union actual visible painted paths; hidden/unpainted nodes consume neither source geometry ordinals nor drawing slots. Original d2svgimport.svgGeometry.primitive supplies the visibility/paint contract. Complete48 RGBA controls over24 fresh original frames,160 export tests and25 harness tests pass. Strict frozen CLI replays recover2 pixel cases with0 regressions in raw160/cancel324/broad159; broad156/159 remains scoped evidence. Source maps, complete retained failures, trace transparency and qualification limits: .reference/escaped-space-scene-20261008/source-comparison.md. General SVG/CSS/effects and full D2 parity remain open; derived changes retain MPL-2.0 notices.

Checkpoint869 follows original d2scenebuild/build.go::buildShapeText and
internal/fontface/shaping.go: scene text runs shape raw LF-split label content.
The native SVG bridge now retains original XML character data beside private
text origins, gated by the explicit raw-scene-text marker on ordinary node/edge
TextRuns; rich Markdown/code SVG text keeps its existing semantics. Scanline uses that raw content for single-chunk simple native text
when SVG whitespace processing changed it. Ordinary imported SVG still uses its
SVG text semantics. Ninety original Go shaping captures cover embedded controls
across three Source Sans Pro faces and sizes10/16/24. Evidence:
.reference/control-glyph-shaping-20261016/. Full scene/font budgets, effects,
platform and broader D2 compatibility remain separate.

Checkpoint871 precision admission refuses duplicate private text IDs for exact
origin/content/font-size lookup. Source scene objects retain independent identity;
a duplicate SVG ID must not alias another TextRun's native metadata. Positional
source records remain available for the existing ordinary SVG fallback. Evidence
and source-preserving trace qualification: .reference/code-glyph-frame-20261016/.

Checkpoint872 follows d2scenebuild/font_fallback.go::resolveFontFallbacks:
discover missing-rune faces in first-occurrence text order. A sorted set of
codepoints changed which previously discovered face owns CR before TAB. Actual
Go/native traces establish differing glyph IDs/advances; a macOS regression
compares all9glyph IDs/positions for the retained raw datatype string. The
production discovery algorithm remains native Rust. Evidence:
.reference/sql-row-precision-20261016/raw-type-source-trace/ and
.reference/sql-fallback-order-20261016/. Other font/platform/budget gates remain
separate from this host-qualified observation.

Checkpoint 879: d2raster/effects.go::renderEffectNode/applyClip applies actual clip geometry, not its enclosing bounds. The native viewport-cover optimization now requires a genuine orthogonal four-corner rectangle, retaining curved, triangular and compound clips as effect layers. Regression: clip_coverage_source_contract, plus six source/native icon/image radius PNGs in .reference/native-target-icons-c4-20261016.

Checkpoint 879 clip transport: preserve native image clip commands through a uniquely identified plain clipPath under the qualified native canvas. Feed original ArcTo operands to the existing source-mapped geometry.rs::flattenArc path instead of usvg's f32 cubic approximation. The same bounded arc flattener is already used by owned SVG assets. Source d2scenebuild/assets.go::roundedBoxClip and d2raster/geometry.go::flattenArc/effects.go::applyClip. Both marker admission and arc retention have direct regressions; raw six PNG controls retain the rejected cubic mask candidate.

Checkpoint 883: source legend-only connection tracks count as animated content. Split legend connections retain source reverse/forward order. Native dash sampling honors reverse CSS track direction using source start+(end-start)*progress operand order, integer time.Duration modulo and nanosecond duration truncation. Source d2scenebuild.animatedConnectionTrack/animatedConnectionTiming and d2raster.interpolateRendererAnimationValue; public Go384 legend scene fixture retains original tracks and exact sampled values, including wrap/near-wrap nanosecond boundaries. This does not qualify all legend animation, sketch, budget or export behavior.

Dash sampling continuation: preserve sampled f64 phase, dash and gap in private operand attributes even when a primitive already has an ID. Retain source operands over six-place SVG serialization when reconstructing duration/period. Source sampled near-wrap values otherwise lose precision in the existing-ID path; both identity cases are checked through the same native dash-parameter collector consumed by rasterization.

Pinned Go1.27 ARM64 interpolates dash tracks with a fused multiply-add. Explicit Rust f64::mul_add preserves the observed single-rounding contract; reverse near-wrap controls expose a difference otherwise (none-w1-mode4 at999999999ns). Source fixture values and native collector results are compared by exact bits, with no tolerance.

Legend shadow continuation: source d2scenebuild/legend.go::buildLegend and d2raster/filter.go::prepareFilters/prepareBlurPasses/paintDropShadow use transformed sigma3 three-box blur support and transformed offsetY2 with black alpha26. Reuse the existing source integer expanded-alpha blur and two-stage premultiplied composition; preserve appendix parameters unchanged. Qualify the generated legend shadow/panel only through exact native SVG ownership/operands. Native support is currently limited to in-frame alpha input, integer transformed offsets and non-banded bounded frames; other cases retain the prior fallback. This gate prevents authored/altered filters from acquiring scene semantics. Fresh strict whole-image/board comparisons remain the authority for pixel claims.

Document metadata in src/document_links.rs follows pinned
 d2scenebuild/metadata.go:shapeLinkRegion/connectionLinkRegion and
 d2cli/paged.go:mapPagedLinks/pdfLinkShapes/addPPTXLinks. Target shape publication,
 full-stroke expansion, connection-label hit boxes, destination provenance and
 tooltip text precede the document coordinate mapping. PDF annotations follow
 lib/pdf/pdf.go and the pinned fpdf CellFormat/newLink serialization; PowerPoint
 uses lib/pptx's distinct position/extent truncation and header-aware image fit.
 The native Rust target projection and serde_json transport execute in process.
 Source-derived CLI fixtures retain complete slide-frame pixel hashes and parsed
 PDF/PPTX metadata, with raw initial captures in the ignored qualification store.

Run 20: PDF image streams follow lib/pdf/pdf.go:86 (PNG registration), pinned
codeberg.org/go-pdf/fpdf v0.12.0 png.go:85,170–246 (PNG predictor/color and alpha
streams) and fpdf.go:4779–4823 (FlateDecode/DecodeParms and grayscale soft mask).
The native writer compresses decoded raster rows with PNG filter 0, preserving
RGB and alpha pixels and declaring predictor 15 with each plane's channel count.
Compression bytes are incidental; complete decoded streams are regression
tested. This targets the five e2etests-cli PDF inspector failures, without
changing the raster renderer or weakening the strict artifact scoreboard.

Run 23: PPTX package assembly follows lib/pptx/pptx.go:203–233,322–350:
write [Content_Types].xml first, raw-copy the complete template inventory
(including directories and compressed member metadata), skip the root and
slide-master relationships, and regenerate those two parts. The slide-master
relationships follow lib/pptx/templates/slidemaster_rels.xml. The single-slide
regression checks the first member, unique names, full template inventory,
member bytes/compression and Go's generated-member count. This targets
how_to_solve_problems_pptx, pptx-theme-overrides, stdout_pptx and no-nav-pptx;
strict package bytes remain a separate scoreboard tier.

Run 26: `src/gif_quantize.rs` follows lib/xgif/xgif.go:160–232,506–548,887–978
(QuantizeImage, palette padding, exact Floyd–Steinberg),
lib/xgif/encode.go:396–406 (normalize white before centering), and pinned
go-quantize ae555eb2afa4 quantize/{mediancut,bucket}.go (occupied-slot order,
weighted median partition and first mode on ties). Sparse ordered storage
preserves the original two-slots-per-pixel collision/traversal order.
GIF package paths quantize each original board before shared-canvas centering.
The single-frame stream omits the loop extension (lib/xgif/encode.go:224–245);
CLI static and animated boards use the same interval sample/delay schedule
(d2cli/raster.go:633–666, lib/xgif/xgif.go:1592–1653). This targets
CLI one-layer-gif, how_to_solve_problems_gif, and animated-gif.

Run 30: explicit `PptxDocumentProperties` follow `lib/pptx/pptx.go:269–302`
and `templates/{core,app}.xml`. Core title/subject/creator/description and last
modifier are per presentation; the app part includes the full ordered final
board IDs, heading pairs, counts and version. XML escaping follows Go
`encoding/xml.EscapeText`, including numeric quote/whitespace references and
replacement of invalid XML characters. Complete generated core/app parts match
pinned Go fixtures in `tests/fixtures/pptx-properties-{core,app}.xml`;
reproducer: `.reference/codex-runs/witness-inputs/run30-pptx-properties.go`.
This closes stdout_pptx's title assertion. Other generated XML and archive/PNG
encoding differences remain non-passing under the unchanged strict byte gate.

Run 31: complete generated PPTX XML follows `lib/pptx/pptx.go:203–289,
360–458,472–516` and `lib/pptx/templates/{content_types,presentation,
rels_presentation,root_rels,slide.xml,slide.xml.rels}`. The native writer
retains every layout content type, all default text levels and guides, header
text-box properties, link hit-region fill/line properties and hyperlink
attributes. Link shape IDs are zero-based, relationship IDs start at `link1`,
and link relationships precede navigation relationships. Template interpolation
is translated into native Rust appends; authored strings are escaped once and
never evaluated as template directives. Package entry order follows writeTo;
no-navigation image fitting also uses AddSlide's IMAGE_WIDTH threshold and
float64/truncation arithmetic (`pptx.go:106–158`).

Targets: CLI `how_to_solve_problems_pptx`, `pptx-theme-overrides`, `no-nav-pptx`
and `stdout_pptx`. Complete Go fixtures, including escaped titles/URLs and
internal/external links with navigation on/off, live under
`tests/fixtures/pptx-parts/`; reproducer:
`.reference/codex-runs/witness-inputs/run31-pptx-parts.go`. The new complete-XML
test fails against the pre-fix writer. The strict byte comparator remains
unchanged: ZIP/PNG encodings and the Go development library's PPTX AppVersion
(0.8.1 versus native 0.9.0) remain separately visible failures.

Run 32: PDF board/header internal destinations follow lib/pdf/pdf.go:149–150,
184–187 and codeberg.org/go-pdf/fpdf v0.12.0 fpdf.go:4304–4323. Each link uses
`/XYZ 0 <destination page height> null`, including that page's navigation band,
so navigation preserves zoom and opens the destination at its top. Separator
stroke color uses fpdf.rgbColorValue's three-decimal components (fpdf.go:951–971)
as SetDrawColor does. Unequal-height source fixtures cover navigation on/off,
forward/backward board links and breadcrumb destinations; generator:
`.reference/codex-runs/witness-inputs/run32-pdf-navigation.go`.

Run 34: `go_png.rs` follows pinned Go 1.27.0 `image/png/writer.go:96–130,
207–301,548–587` and `bufio/bufio.go` Writer.Write/Flush: signed-byte filter
costs with Up/Paeth/None/Sub/Average tie order, opaque NRGBA to RGB selection,
BestSpeed zlib and write-preserving 32 KiB IDAT buffering. Both document RGBA
and standalone RGB PNG paths use it. FPDF image streams retain the filters
chosen on the original RGBA rows before splitting RGB/alpha
(`fpdf v0.12.0 png.go:170–246`); `xcompr.go:25–44` compresses each plane with
BestSpeed. The existing `d2-rs-goflate` implementation supplies native Go 1.27
DEFLATE/zlib behavior; Go is only the fixture oracle.

`go_zip.rs` follows `archive/zip/writer.go` Create/CreateHeader/CreateRaw/Copy,
writeHeader/writeDataDescriptor/Close and `register.go:35` (level 5). It retains
template compressed members and metadata, uses Go's zero timestamps, creator/
reader versions and data descriptors for generated members, and writes the
central directory in insertion order. ZIP64 headers/end records follow the
same source. The parser is private to the immutable checked-in ZIP32 template.
Fixtures: `png-encoding-go127.json` (regenerated, not committed: recipe
`renderexport-png-encoding`, `tools/png-encoding-oracle`, which rejects
toolchains other than Go 1.27.0; harvested from the historical reproducer
`.reference/codex-runs/witness-inputs/run34-encoders.go`) and the committed
`zip-encoding-go127.zip`. Targets: the five CLI PDF cases and
four PPTX cases, plus all PNG outputs. Strict PDF/PPTX artifact verdicts still
require complete byte identity. The scoreboard now explicitly reports the
pinned Go development AppVersion (0.8.1) versus native release (0.9.0), while
keeping that artifact non-passing and leaving the CLI release version intact.

Run 36: `pdf_font.rs` follows FPDF v0.12.0 `fpdf.go:4503–4582`
(putfonts UTF8), `4590–4711` (generateCIDFontMap), `xcompr.go:25–44`,
and `utf8fontfile.go:335–447` (descriptor metrics). Both source fonts are
registered by D2 `lib/pdf/pdf.go:34–35` even with navigation disabled.
The native writer retains the seven-object font structure, UTF-8 base names,
fixed 131072-byte CID map, source width grouping, truncating descriptor
metrics and Go BestSpeed-compressed map/font streams. `makeSubsetRange(57)`
initializes values to zero; it does not select 57 characters. The literal
text subsetter exposes FPDF's older cmap length formula separately from
D2's corrected SVG formula; the default SVG path is unchanged.

Targets: CLI `internal_linked_pdf`, `renamed-board`, `no-nav-pdf`, `theme-pdf`,
`stdout_pdf`. Six complete font-record fixtures cover empty usage, actual
board-title text and Unicode with punctuation/missing glyphs. Rebuild with
`.reference/codex-runs/witness-inputs/run36-pdf-fonts.go`, then
`run36-extract-fonts.py` (pypdf); Go is only the test oracle. Outer document
serialization, page streams/resource identifiers and metadata remain separate
strict artifact failures. Full font fixtures compare all seven raw objects,
not decoded or normalized approximations.

Run 37: `pdf_document.rs` follows pinned `lib/pdf/pdf.go:AddPDFPage` and
FPDF v0.12.0 `fpdf.go:AddPageFormat/CellFormat/imageOut/putpages/putimages/
putimage/putresourcedict/putresources/putcatalog/enddoc` and
`def.go:generateImageID/generateFontID`. Page streams and font/image resource
identifiers are generated natively from metadata and bytes. Object order,
shared resource dictionaries, inline links, image deduplication, stream
compression and xref offsets follow FPDF. The native writer chooses FPDF's
supported deterministic catalog order; Go's ordinary unsorted maps and current
clock fields remain visible to the strict scoreboard. No lossy PDF comparator
is used. The fixed full-document fixtures select FPDF's existing catalog/date
controls only in the witness generator:
`.reference/codex-runs/witness-inputs/run37-pdf-documents.go` (Go 1.27.0).
The complete fixtures use the BestSpeed PNG input produced by D2's own raster
path. The earlier opaque PNG IDAT gap is retained as negative baselines
`run37-opaque-default-{compression,native}.pdf`
under `.reference/codex-runs/witness-inputs/`. Unix timestamps use local wall time;
hosts without a native timezone interface currently fall back to UTC.

Run 40: `go_png::with_exif` translates pinned D2 `lib/png/png.go:22–39,
66–87,128–136,199–229`: big-endian TIFF Make/Model entries, inline strings
below four bytes, external Model strings, terminating NUL and PNG CRC. The
public `add_png_exif_to_encoder_output` consumes metadata-free trusted encoder
output and checks its IHDR/signature/CRC/IEND envelope, matching the source's
encoder-output entry point. It is not the arbitrary-input AddExif rewriter.
The CLI alone adds its release `v0.9.0` model; PDF/PPTX internal images retain
their original metadata-free bytes. Fourteen complete Go PNG fixtures cover
RGB/RGBA and byte-length boundaries, generated by
`.reference/codex-runs/witness-inputs/run40-png-exif.go` with Go 1.27.0.

Run 41: `decode_pdf_png` retains the concatenated IDAT payload of validated,
non-interlaced 8-bit RGB inputs, following FPDF v0.12.0 `png.go:120–124,247`.
The shared `PdfRasterPage::image_streams` supplies those bytes to both PDF
writers, including `pdf_document::Image::new`'s content-derived image ID and
deduplication. Generated rasters and the existing RGBA plane-splitting path
retain their Go BestSpeed encoder. Interlaced input remains on the previous
decoded-pixel path; FPDF itself rejects interlacing. This does not claim broader
grayscale/indexed/16-bit PNG API parity. Complete Go documents and input PNGs:
`tests/fixtures/pdf-png-idat-fpdf.json`; generator:
`.reference/codex-runs/witness-inputs/run41-pdf-png.go` (Go 1.27.0).
`tests/fixtures/pdf-opaque-default-fpdf.pdf` is run 37's original 6992-byte
negative source document, now checked against the fixed decoder/writer without
changing a byte. Rebuild with `run37-pdf-documents.go`, replacing
`png.BestSpeed` with `png.DefaultCompression` as its retained reproduction
record specifies. The original negative native PDF remains in `.reference`.
Fixed dates/catalog order are selected only by the source witness, using
FPDF's existing controls; the product clock and strict scoreboard are unchanged.

Run 42: reproducible PDF exports opt in with `SOURCE_DATE_EPOCH` (decimal Unix
seconds 0 through 253402300799). Both metadata dates use that instant in UTC;
without it the existing clocks are preserved. This documented injection is a
native extension for the strict comparison allowed by brief Update 12. It is
shared by the navigation writer and the older packaging helpers; malformed or
out-of-range values return a PDF export error. The Gregorian conversion is the
existing native UTC conversion. The trace-only hook configures each source
FPDF instance through its existing `SetCreationDate`, `SetModificationDate` and
`SetCatalogSort` methods (`fpdf.go:4178–4220`); the source algorithm and all PDF
bytes still reach the unchanged strict comparator. Targets: the CLI
`internal_linked_pdf`, `stdout_pdf`, `theme-pdf`, `renamed-board`, `no-nav-pdf`
artifact mismatches. The harness requires an explicit `--pdf-epoch` profile and
records the controls in its manifest; historical negative captures are retained.

Run 44: all PDF convenience entry points now use `pdf_document::encode`,
including `render_pdf`, `package_pdf_pages` and
`package_pdf_pages_with_link_regions`. Static comparison found the older
serializer started with unconditional PDF 1.4, omitted the registered font
resources, used a separate timestamp/Info encoding and misplaced centered-image
link X coordinates. D2 `lib/pdf/pdf.go:30–40,72–118,168–190` and FPDF v0.12.0
`fpdf.go:209,4271–4349,4979–5059` define the shared document path. The duplicate
writer and its object allocator were removed. Public packaging APIs now match
the retained complete source documents for default/none/speed/best/split-IDAT
PNGs, a six-page deduplication document, and the opaque-default linked document
(13 API/document pairs). Those fixtures retain their original bytes and Go
generators from runs 37/41; tests select the documented shared UTC clock in an
isolated subprocess. This is supplementary API evidence; upstream CLI totals
do not increase. Ordinary convenience exports now use the same FPDF local-time
date syntax as the navigation writer.

## Empty glyphs in native color text (2026-10-24, Claude)

`draw_color_text_native` aborted the whole run when a shaped glyph had an
empty glyf record, so a ZWJ emoji sequence (whose joiners shape to empty
glyphs) fell back to resvg's `.notdef` boxes. d2raster/text.go positions
these as `empty` glyphs and draws nothing for them; an empty record is now
skipped. Witness `unicode/japanese-mixed` (👩‍👩‍👧‍👶) becomes pixel-identical;
raster scoreboard 878 → 881/943.

## Color glyphs in Markdown paint bounds (2026-10-24, Claude)

`native_prepared_bounds` gave up on a whole Markdown layer when a glyph had
no outline, so a color emoji made the layer lose its paint reference and the
other text runs fell back to an unrelated legacy origin (their glyphs then
rounded off-canvas: "hello 🛡" drew no "hello"). fontface GlyphRenderBounds
(face.go:552–567) bounds a trusted COLRv1 glyph by its static clip box; that
box, mapped through the glyph placement, now joins the prepared bounds.
Witness `regression/markdown_emoji_cutoff_2680`; raster 881 → 884/943.

## Scene-wide font fallback buckets (2026-10-24, Claude)

d2scenebuild resolveFontFallbacks (font_fallback.go:131–420) gathers every
text's uncovered grapheme clusters into one bucket per text font, resolves
each bucket once through SystemFallbackResolver.ResolveFallbacks
(d2fonts/fallback.go:121–282: one request profile from all missing runes,
font paths by priority, greedy best face per path), and gives each text the
bucket faces covering one of its missing runes. The raster path resolved one
rune at a time, so a lone U+203B got a CJK face where the scene's other
symbols select a symbol face, and Armenian took a different weight.
`scene_fallback_buckets` (cached per rendered tree) and
`font_fallback::select_fallbacks` now follow source; bundled faces keep
precedence. Witnesses `txtar/unicode`, `unicode/mixed-language-2`; raster
905 → 911/943; CLI scoreboard unchanged.

### Raster WebP, isolated layers and non-solid paint (2026-10-24)

- Static WebP icons decode through x/image/webp's image model before the
  native PNG sampler: VP8 as YCbCr 4:2:0 expanded to sixteen-bit RGBA
  (`owned_webp_png`), VP8L as NRGBA. They had reached resvg's own decoder.
  Witness `txtar/sql-table-reserved` (all engines; its icon URL serves WebP).
- An isolated group with no other effect is one offscreen layer composited
  source-over at its opacity, as renderEffectNode does (effects.go:675–716).
  d2scenebuild finishShape isolates animated shapes; witness
  `txtar/shape-animate`.
- Non-solid fill, stroke and gradient-text paint follow drawRasterizedPaint /
  drawPaintCoverage (gradient.go:649–988): samples composite with
  compositeNRGBAOverRGBA, the rasterizer origin is subpathPixelBounds' floored
  minimum, and x-only linear coverage evaluates `xC*x + 0*y + offset`
  (product rounded before the offset, unlike colorAt's fused form). Native
  strokes and glyph runs now accept gradient paint instead of sending the
  whole tree to resvg. `RawGradient` carries the document's f64 linear vector
  next to the f64 stops. Witness `txtar/gradient` (all engines).

### PNG band emulation (2026-10-24)

d2cli writes PNG through `d2raster.RenderBands` with 256-row bands
(`raster_png_output.go` pngBandHeight); GIF and paged frames use full-frame
`Render`. Under bands (`regionFilters`), each primitive is rasterized once per
destination: the band canvas, or an effect node's layer tightened by
`effectRenderBounds` / `strokeRegionBounds` (stroke_regions.go), and its
vertices are rounded relative to `referenceBounds` from
`setBandReferenceBounds` (bands.go) — node.bounds within the enclosing
effect's reference bounds. The scanline rasterizer clips to its destination:
edges left of it accumulate in `difference[0]`, rows outside are not
scanned, and `flattenCube` drops cubics whose hull misses it.

For PNG export (`GoPngBandsScope`) the native full-frame renderer now
reproduces that: every rasterizer paint runs once per Go destination window
with edges translated by reference − destination (setRasterizerReference),
cubic-derived edges carry their leaf hull for the same culling, and an effect
group is first drawn in a record pass that captures each leaf's prepared
bounds (fills: subpath bounds ±1; strokes: run segments with strokeExtent+1,
or strokeRegionExtent+1 for stroke-only primitives; images and vector-image
viewports: transformedImagePixelBounds; clip and mask: their root bounds ±1,
intersected as in raster.go's node bounds) to derive node.bounds,
per-band layer windows and the reference origin its primitives use. The
layer composites over the whole surface (its pixels lie only in Go's
windows). Markdown viewport/text lookup ids are now unique across objects;
offset-only ids collided between labels and lost their exact f64 placement.
Witnesses: `txtar/theme-overrides`, `regression/opacity-on-label`,
`stable/animated`, `txtar/c4-person-label`, `txtar/md-label`,
`txtar/icon-style` (raster 934 → 943/943 renderable).


### Master PDF backgrounds and consumer fixtures (run 52, 2026-10-25)

`pdf_document::encode` follows pinned master `lib/pdf/pdf.go:119–125`:
a page-wide fill rectangle is emitted after AddPage even when navigation is
disabled. FPDF's fill state persists into the next page's preamble in either
mode. This closes complete-PDF mismatches in the shared-clock CLI cases
`internal_linked_pdf`, `stdout_pdf`, `theme-pdf` and `no-nav-pdf`.
The change follows the upstream movement of SetFillColor/Rect outside the
includeNav branch; image streams, annotations and navigation remain covered
by complete source documents.

The exporter consumers of run 50's master `svg_viewport::aspect_matrix` now
use the pinned public `d2scene.AspectRatioMatrix` expectations as well:
17 of the original 120 placement records change exact result bits under
master's explicit product rounding (`d2scene/aspect.go:28–54`). Authored inputs
are unchanged. The v0.9.0 files were retired as unread orphans (their bytes
remain at `9b706ac1`); no old expected bytes are overwritten or normalized.

Reproduction: recipe `renderexport-public-pdf-master`
(`tools/fixtures/renderexport-export-public.toml`, hook
`tools/fixtures/hooks/renderexport-public-api/`), which replaced
`tools/pdf-master-probe/{capture.py,png/main.go,documents/main.go,matrix/main.go}`. The capture reads the manifest pin and
trace checkout through `scripts/references.py`, calls the original public
Go APIs, checks identical PNG/matrix inputs and the unchanged navigation PDF,
and publishes the `*-master.*` fixtures. Complete PDFs use FPDF's explicit
fixed UTC clock/catalog controls; all bytes are compared. These fixtures cover
both navigation modes, transparent/opaque images, alternating page fill state,
multiple pages and caller PNG compression. Existing tests consume the new
master fixtures; original v0.9.0 negatives remain retained.

## Run 53: shared SVG acquisition helpers

`go_xml_names.rs`, `svg_import_envelope.rs`, `svg_asset_xml.rs` and
`svg_asset_codec.rs` moved to `d2-rs-math/src/image_svg/`. The exporter imports
those shared modules; its existing legacy preparation API, limits, bytes and
diagnostics are unchanged. The source mappings and retained notices above
still apply. The 342-case legacy codec fixture remains in this crate and is
executed by the moved implementation's tests.

The scene curve predicates in `scanline.rs` map to master 6a7b48378
`d2renderers/d2raster/geometry.go:274–315,376–417`. Go 1.27 ARM64 assembly
for `pointLineDistance` (382) and `controlsProgress` (400–401) rounds the
first product and fuses the second with FMADDD/FMSUBD. The shared Go float
helpers preserve that order; other architectures keep separate operations.
The `stable/chaos2` theme-300 glyph witness hits the 0.125 tolerance boundary:
separate products omit a quadratic subdivision, changing seven PNG pixels.
The native curve regression preserves the exact Go subdivision point.

## Oracle fixture recipes (2026-10-31)

Every Go-generated fixture under `tests/fixtures/` is now regenerated into
`oracle-cache/` by `tools/fixtures/renderexport-export*.toml` (docs/FIXTURES.md);
the run/witness generators named above are the provenance of those recipes'
hooks, not the way to rebuild them. Hand-authored request inputs live under
`tools/fixtures/inputs/renderexport/`.
