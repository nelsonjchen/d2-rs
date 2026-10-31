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

Checkpoint518 removes `known_mathjax_size`: `measure` now reads the actual
emitted SVG root and uses original mathjax-go:Measure's serialized ex*8/ceil
contract. Unparsed input retains an explicit visible text fallback; whole
formula render-witness routes remain a major compatibility gap. No new literal
formula dispatch is introduced. Original `internal/svg/mfrac.go` supplies
child-bbox-dependent shifts, display/text fraction style and rule/padding.
`internal/svg/msubsup.go` supplies display/prime superscript shifts;
`internal/svg/wrapper.go` supplies full 1/sqrt(2) script scale.
`internal/svg/texclass.go` and `internal/mml/node.go` supply binary demotion,
null fraction classes and the TeX spacing table. `internal/jscompat/number.go`
supplies exact binary tie direction for one-decimal root/placement serialization.
Generated normal/large font italic corrections follow the selected source
font; `scripts/generate-math-glyphs.py` preserves correction metadata.
Source-derived portions retain MathJax Consortium Apache-2.0 notices.
The original module remains pinned by pristine D2 go.mod/go.sum. Probes22
expressions,13 fraction-box contracts, complete CLI frames and the unqualified
radical are recorded under .reference/general-tex-raster-20261007/. Root matches,
scoped box/image tests and inherited witness matches do not qualify complete
MathJax parsing/layout, SVG topology/accessibility or unsupported font/style
paths. Exact radical sizing/stretching, full script italic handling, language
transport, errors/extensions and whole-formula witness removal remain open.

Checkpoint519 translates `internal/svg/msqrt.go`, fixed and quantized assembled
selection/baseline from `internal/svg/mo.go`, and top/bottom/clipped extender
from `internal/svg/stretch.go`. `src/radical_glyphs.rs` contains the five Source
fixed font variants plus Size4 assembly parts. `internal/svg/msubsup.go` and
`layout/parameters.go` supply actual script bounds, italic corrections and
shift constraints. All16 radical root/measure records match;11/12 additional
script roots match (large integral remains different). Complete164 CLI frames
retain32 failures despite root equality. Initial height>3em visible fallback
was replaced with native assembly; all raw failed candidates remain under
.reference/radical-tex-20261007/. Full nested SVG hierarchy/import/clip fidelity,
script-level and large-op semantics, extension regression, math connections
and whole-formula render witnesses remain open. Copyright/Apache2 notices
are retained. These scopes do not qualify all MathJax/D2 compatibility.

Checkpoint520 maps retained style/enclosure scopes to tex/commands.go,
svg/menclose.go, menclose_notation.go, menclose_shapes.go, mpadded.go,
wrapper.place/standardSVG and layout/lengths.go in pinned mathjax-go v0.1.0.
Named colors copy color/constants.go with unknown CSS passthrough from model.go.
Color declarations wrap the remaining row/group; textcolor is scoped; colorbox
uses normal text and original5px padding/background. Parent scales and local
coordinates are retained before SVG fixed() serialization. Operator class
spacing includes colored sin/cos. One whole-formula color/colorbox render route
is removed; other witnesses remain. The complete28 probe retains three
unsupported macro/model/option/error cases. Scoped25 root contracts,323 tests,
112new/164prior complete CLI submissions and broad evidence live under
.reference/cancel-color-tex-20261007/. Root equality is not raster/full-MathJax
parity; imported strokes/backgrounds/clips, transparent script-base/style/large-op
semantics, math connections, remaining extensions/witnesses and wider PLAN
are explicit gaps. Translated portions retain Apache2 attribution.

Checkpoint521 maps original `internal/tex/extensions/gensymb/config.go`'s
five normal-font unit identities (U+00B0, U+00B5, U+2126, U+2103, U+2030),
`internal/svg/wrapper.go:computeTextBBox/unknownText`'s non-CJK fallback
metrics and SVG text, and `internal/tex/commands.go:mathFont` plus
`parser.go:parseMathFontString/applyMathVariant`'s upright font environment
and ORD class. The normal glyph subset now includes degree directly from
`data_normal_gen.go`; no formula-specific paths or dimensions are added.
The inherited gensymb formula renderer is removed. Normal Greek identities
follow `source_tables_base_maps.go`; other font variants, complete multi-letter
MML topology, unit class markup and general unknown/CJK fallback remain open.
The degree's negative font depth is retained in glyph-only root bounds;
composite layout bounds remain authoritative. Complete raw Source/native
probes and accepted/error image inventories are retained under
.reference/gensymb-tex-20261007/. Root contracts are not full SVG parity.

Checkpoint522 translates original D2 v0.9.0
`d2renderers/d2svgimport/mathjax_text.go`'s generated-math text/root/unit
metadata checks, placement constraints, inherited paint validation and the
independently authored micro scene outline into `src/scene.rs`. Ordinary math
SVG rendering retains text; the checked native raster scene replaces the
accepted micro text with original local path commands. `d2scenebuild/assets.go`
preflights visible nonempty math shapes and skips opacity-zero labels;
`build.go` omits those labels. Renderer errors preserve source object/stage
context. Source units now retain class metadata, and script-base character
handling recognizes unit tokens. Source-direct37 controls and local outline
commands are permanent contracts. This generated-math adapter does not complete
the general SVG importer API, resource/style/limit/work/cancellation contracts,
math connections/legends or whole MathJax support. Full unit and inherited
render controls remain in .reference/mathjax-scene-20261007/.

Checkpoint523 retains `internal/svg/texatom.go`/`wrapper.go` local ORD scope
placements and `internal/tex/source_tables_base_maps.go` math-hyphen remapping.
These source-derived MathJax Consortium portions retain Apache-2.0 attribution.
The native scope keeps independent parent/child serialization, negative glyph
depth and embellished character script bases. Source28 measured/root contracts
include nesting, fractions, scripts, units and negative degree depth; complete
paint qualification is recorded separately in `.reference/texatom-scope-20261007/`.
CD remains an ordinary matrix fallback and inherited CD witness route; fifteen
fresh original controls expose fourteen failing root contracts. General text
commands, font variants and all other recorded extension/witness gaps remain
open. No whole-formula dispatch or size lookup was introduced.

Checkpoint524 translates generic CD object/arrow row parsing, struts, table
metrics and horizontal stretching, labeled arrow stacks, fixed/assembled
delimiters and minCDarrow dimension commands from pinned mathjax-go v0.1.0
internal/tex/{environments,amscd_handlers,commands}.go, internal/svg/
{mtable,munderover,stretch}.go and internal/layout/lengths.go. The inherited
CD whole-formula dispatch and renderer are removed. Native calligraphic font
selection inherits italic/normal data and preserves outer font scopes and
identifier runs following internal/tex/parser.go and font/{font,smp}.go.
Generated calligraphic/italic tables come from data_tex_calligraphic_gen.go
and data_italic_gen.go using scripts/generate-math-glyphs.py. U+23D0 and
small-operator U+2016 path data retain the same font attribution. Recoverable
CD errors use original merror text metrics and original D2 font-family style
rejection. Source32 root/measure contracts are permanent; full image evidence
is separate in .reference/amscd-tex-20261007. Nested SVG clip raster fidelity,
vertical row stretching, nested environment capture, complex text and wider
font/extension/error APIs remain open; this does not qualify full MathJax.

Checkpoint526 translates original mathjax-go v0.1.0 svg/mo.go deferred
stretch emission while mtable.go retains cached natural table bounds; CD
left-label offsets stay in their script-scale local frame following
tex/environments.go llap/rlap recipes and svg/mpadded.go. Vertical labels
use ordinary script style; only horizontal under-labels use prime style.
Scripts retain msubsup.go parent/child placement and independent RScale
serialization, child bounds and negative glyph depth. Original MathJax
Consortium portions remain Apache-2.0. Source12 varied root/measure
contracts are permanent; full strict capture retains two single-pixel
multirow equality failures and the other outstanding math/PLAN gaps.
Evidence: .reference/cd-paint-20261007/. No whole-formula geometry or
tolerance is added; other existing witnesses remain explicitly open.

Checkpoint527 translates original mathjax-go v0.1.0 internal/tex/
environments.go splitTable/captureEnvironment/parseCDRow and parser.go
readControlSequence/readArgument: immediate optional row annotations,
matching nested environment capture and earliest recoverable errors.
Original MathJax Consortium Apache-2.0 attribution is retained.
Eleven source root/measurement/error contracts are permanent; complete
render evidence is separate in .reference/cd-capture-20261007/.
Full font/extension/error and broader D2 compatibility remain open.

Checkpoint528 translates original mathjax-go v0.1.0 tex/parser.go numeric
mn runs, svg/wrapper.go token IC/unknown-font/isCJK metrics and
svg/msubsup.go numeric script core semantics. MathJax Consortium Apache-2.0
attribution is retained. number_unicode.rs derives Go1.27.0 Unicode17
IsDigit ranges; Go Authors BSD-3-Clause notices above apply. The complete
72-range table is verified by scripts/check-math-number-unicode.py against
the pinned original parser oracle. Thirty-two measured/root contracts are
permanent. Evidence: .reference/numeric-tex-20261007/. This does not qualify
full fonts, extensions, errors, API or D2 parity; wider gates remain open.

Checkpoint529 translates original mathjax-go v0.1.0 layout/lengths.go
relative/absolute/size-dependent dimensions, tex/commands.go horizontal
spacing, tex/environments.go raw CD minimum-label kern, svg/wrapper.go
mspace/signed bbox Append and svg/typesetter.go final viewport clamps.
MathJax Consortium Apache-2.0 attribution is retained. Native Unicode
classification adds Go1.27.0 digit/letter-union ranges; existing Go Authors
BSD-3-Clause notices apply. The72/736 ranges are checked against the pinned
original parser oracle. Thirty-six root/measurement/error contracts are
permanent; strict image qualification retains two single-pixel failures.
Evidence: .reference/length-tex-20261007/. Wider compatibility remains open.

Checkpoint530 stages a native translation of pinned mathjax-go v0.1.0
internal/mhchem/texify.go (mhchemparser4.1.1). Copyright2015-2021 Martin
Hensel,2026 luo-studio and2026 erweixin; Apache-2.0 AND MIT. The MIT notice
is retained in crates/d2-rs-math/licenses/mhchem-go-tex-MIT.txt. Original
PROVENANCE.md maps the pinned parser and initial Go translation. All718
node/error texifier contracts are source-qualified and permanent. The
state machine and host-TeX integration remain pending; the staged component
is not yet used by ce/pu. Evidence: .reference/mhchem-texifier-20261007/.

Checkpoint531 translates pinned mathjax-go v0.1.0 internal/mhchem/jsregexp.go
and patterns.go into native Rust. Its exact data/patterns.json is retained;
eleven inline patterns are extracted unchanged from original patterns.go.
Copyright2015-2021 Martin Hensel,2026 luo-studio and2026 erweixin;
Apache-2.0 AND MIT; the existing mhchem-go-tex-MIT.txt notice applies.
15984 regex and25688 token/remainder/error source contracts are permanent.
Only the matching/output stages are translated; state-machine/actions and
host integration remain pending. Evidence: .reference/mhchem-matcher-20261007/.

Checkpoint532 translates complete pinned mathjax-go v0.1.0 internal/mhchem
state-machine driver,nullable buffers and all action handlers into Rust.
data/machines.json is copied exactly (14 machines,51 states,1277 transitions).
Copyright2015-2021 Martin Hensel,2026 luo-studio and2026 erweixin;
Apache-2.0 AND MIT; existing mhchem-go-tex-MIT.txt is retained.
All117 original mhchemparser4.1.1 assertions plus retained/exploratory/errors
qualify973 exact expansion/error contracts. Host-TeX integration remains
pending. Evidence: .reference/mhchem-engine-20261007/.

Checkpoint533 translates pinned mathjax-go v0.1.0 tex/commands.go phantom,
smash and lap commands,parser.go argument/bracket reading, svg/mpadded.go,
mphantom.go and transparent script-core H/D from msubsup.go. Root bounds
retain msqrt.go em arithmetic before scaling when smash exposes isolated D.
Copyright2017-2022 The MathJax Consortium; Apache-2.0 notices apply.
All304 fresh root/measure contracts are permanent; image qualification is
recorded separately. Chemistry host integration and wider parity remain open.
Evidence: .reference/padded-tex-20261007/.

Checkpoint533 additionally translates original default mtext glyph advances and
local frames from mathjax-go svg/wrapper.go: no mi italic correction is applied.
The24 added literal-text controls complete304 permanent root/measure contracts.
Final native image evidence and explicit failures are recorded in
.reference/padded-tex-20261007/qualification.json; wider compatibility stays open.

Checkpoint534 translates mathjax-go v0.1.0 tex/commands.go mathClass,
parser.go groups/arguments/scripts, tex/inheritance.go and
svg/texclass.go,texatom.go,msubsup.go,munderover.go,mo.go,wrapper.go into
native classes.rs/lib.rs. Source normal-font long arrows and the reachable
SmallOp/-tex-variant font subsets are generated from data_normal_gen.go,
data_smallop_gen.go,data_tex_variant_gen.go. Copyright2017-2022/2018-2022
The MathJax Consortium; Apache-2.0 notices and original font attribution apply.
All275 permanent original root/measure/glyph/error contracts and actual image
qualification are recorded in .reference/class-tex-20261007/.
Chemistry injection and broader compatibility remain pending.

Checkpoint535 translates mathjax-go v0.1.0 internal/tex/parser.go scoped
styleDeclarations, commands.go mathChoice/styleAttributes and inheritance.go
choice/style/script/fraction/table contexts into native styles.rs/lib.rs/cd.rs.
Native style_classes.rs follows internal/svg/texclass.go transparent first/last
class transfer and wrapper.go script spacing; core script corrections retain
absolute style context. typesetter.go root H+D uses em arithmetic before units.
Copyright2017-2022 The MathJax Consortium; Apache-2.0 notices apply.
Evidence: .reference/style-choice-tex-20261007/source-comparison.md and complete
260 source controls. Permanent259 root/measure/glyph/error contracts pass;
generic matrix geometry remains an explicit retained failure. Image/broad
qualification is recorded separately. Chemistry injection and wider PLAN remain
open; this scoped corpus does not establish overall D2 compatibility.
Checkpoint535 qualification also preserves parent script/style paint frames,
msqrt.go scaled paint Rule independently of bounds, and msubsup.go single-sub
em/RScale arithmetic before serialization. Raw pixel failures are retained.

Checkpoint536 translates original mathjax-go internal/tex/base_ams_handlers.go
amsXArrow/amsArrowSizes and parser.go emLength/attachScript into native
xarrow.rs/lib.rs. Relative mpadded H/D/W/offset fields follow svg/mpadded.go.
Forced base scale, independent inherited labels, width selection/stack bounds,
inner-core class spacing and negative depth follow svg/munderover.go,
texclass.go,mo.go,wrapper.go,layout/bbox.go. CD delimiter painting now shares
horizontal recipes from font/data_delimiters_gen.go and the source overlapping
extender guard in svg/stretch.go. Copyright2017-2022/2018-2022 The MathJax
Consortium; existing Apache-2.0/font notices apply. Evidence:
.reference/xarrow-tex-20261007/source-comparison.md;286 permanent original
root/measure/glyph/error contracts. Images and broader compatibility are
qualified separately. Chemistry injection remains pending.

Checkpoint537 translates default alphabetic command recognition from pinned
mathjax-go tex command dispatch/maps, using1664 static source-name candidates
and an actual D2 oracle audit. The generated906-name command_registry.rs
separates recognized-but-untranslated handlers from UndefinedControlSequence;
758 original undefined-name root/glyph/error controls are permanent. Inactive
source macro records such as longrightleftharpoons and dots retain actual Go
undefined behavior. This recognition table does not implement those906 handlers.
Source symbols.go equiv/circ mappings and normal-font NBSP entry0xA0 follow
commands.go space/nobreakspace/text semantics. Normal font regeneration adds
that exact entry (1101 total); other retained tables stay28/26. msqrt.go
rootToSVG keeps paint arithmetic in em before conversion and preserves the
translated baseContainer independently of glyph placements. Copyright2009-2022
The MathJax Consortium; existing Apache-2.0/font notices apply. Evidence:
.reference/host-primitives-20261007/source-comparison.md, source probes,
regeneration bindings and permanent host-primitives/undefined-commands/commands
fixtures. Raw ce/pu injection and inherited chemical witness removal remain
pending; no overall D2 percentage is implied.

Checkpoint538 connects native mhchem expansion to parser.go/commands.go ce/pu
raw argument reading and insertion semantics, including parseOneToken/script
boundaries and empty insertion. The inherited chemical formula branch/helper
are removed. Generic script attachment preserves earlier atoms across empty
insertions. mfrac.go computeFractionBBox logical child combination and negative
depth retention now cover empty/negative children independently of paint.
Source mappings: native lib.rs/padded.rs/mhchem/mod.rs and already translated
mhchem driver/actions/tables. Existing Apache-2.0 AND MIT notices remain.
Evidence: .reference/mhchem-host-20261007/source-comparison.md and full1039
source controls;916 permanent qualified root/glyph/error contracts,123 explicit
outstanding host cases. Broader math/PLAN and other witnesses remain open.

## Checkpoint539: native letter font selection and symbol identity (2026-10-08)

Source: pinned mathjax-go v0.1.0 tex/parser.go parseCharacter/applyMathVariant,
tex/commands.go mathFont, base/AMS mathchar0mi source maps, svg/wrapper.go
getVariant/unknownText, font/font.go lookupOwn/lookupEffective/Lookup and
font/smp.go; original D2 d2svgimport/compile.go compileElement and style.go
presentationProperties before mathjax_text.go frozen validation.
Rust: lib.rs automatic_italic_atom/source_identifier/italic_glyph,
literal Unicode identity and inherited glyph serialization; classes.rs font
corrections; scene.rs attribute-order diagnostics; glyphs.rs all1278 normal
paths and italic_glyphs.rs all60 exact italic entries.20 normal Content
substitutions remain untranslated. Letter classification is exact Go1.27
Unicode17 (684 ranges/145672 codepoints), regenerated from the certified parser
union-minus-digits and independently checked against its immutable Go oracle.
Source evidence: .reference/literal-font-20261008/source-comparison.md,
font-qualification.json, complete1062 original/frozen math+scene inventories
and all capture/replay logs. API1062/1062, no baseline losses; new raster4248/4248,
prior9722/10192 with92 gains/zero losses, broad155/159. Aggregate qualification
is in .reference/literal-font-20261008/qualification.json when its verifier
completes successfully. Nearly-full goal remains active; plugins excluded.

## Checkpoint540: native compound glyph composition (2026-10-08)

Original mathjax-go v0.1.0 svg/wrapper.go placeChar/font.Lookup; all20
normal-font Content entries, operator dictionary classes, large-op metrics
and correction/axis/variant semantics. Rust compound_glyphs.rs/generated by
scripts/generate-math-compounds.py; lib.rs compound/glyph/layout/render helpers
and full49 large_glyphs.rs source paths.160/160 permanent API contracts include
actual compound-group child identity/transforms. New640/640 CLI/raster controls;
349 library tests,25 harness tests; prior20 retain13970/14440 with zero losses,
broad155/159 unchanged. Final strict qualification.json is the aggregate gate.
Evidence: .reference/compound-font-20261008/source-comparison.md and complete
original/frozen/capture/replay records. Nearly-full goal active, plugins excluded.

## Checkpoint541: generic table foundation

Pinned mathjax-go v0.1.0 tex/environments.go/nodes.go/inheritance.go,
mathtools handlers and svg/mtable.go/mtr.go/mo.go/stretch.go/texclass.go/mfrac.go;
font/delimiter.go and complete generated vertical/size3/size4 data. Native
table.rs and parser/class/style/fraction helpers replace inherited matrix and
aligned fixed geometry/serializers.293 permanent root/measurement/glyph/error
and row/cell/table affine contracts;350 library tests, zero ignored.
Source comparison/API receipt: .reference/matrix-native-20261008.
CLI/raster1162/1172 (112 source-expected errors), ten retained one-pixel double-bar failures. Broad155/159 unchanged. Prior21 retain14614/15080,4 gains and zero losses. Final aggregate certification is governed by qualification.json; absent/failed receipt is not a verified result. Nearly-full goal active.

## Checkpoint542: primes and script ownership

Original mathjax-go v0.1.0 tex/parser.go, base_ams_handlers.go, commands.go
and svg/msubsup.go/munderover.go/wrapper.go. Native parser retains side versus
under/over script kind, translates prime promotion/errors and operatorname
property/attribute ownership, and preserves negative prime depth in radicals.
582 permanent complete root/measurement/glyph/text/error affine contracts pass;
351 library tests, zero ignored. Empty math1×1 matches actual original API.
Evidence: .reference/prime-native-20261008/source-comparison.md. New2328/2328 capture, prior22 replay15856/16252 with80 gains and zero losses,
broad155/159 and final verified aggregate apply to the frozen542 aliases and
exact candidate-source snapshot. Current543 requires separate qualification.
Wider PLAN gates remain open.

## Checkpoint543 in progress: general fraction, operator and spacing frames

Original tex/operator_dictionary_generated.go canonical manifest, inheritance.go
operatorForms/refreshDynamicFlags, nodes.go raw MO construction, parser.go
control sequences, commands.go spacer and notchar source-map precedence;
svg/mo.go symmetric/large-op bounds and wrapper.go font fallback. Native tables
retain all1151 class/spacing/ordered-property payloads and50 ranges after
verifying every original hash. Native raw MO inheritance and rendering match
all4895 actual raw-spelling source controls; this is not exhaustive direct MML
property or script/font context coverage. Frozen5422481 -> current4895 exact
glyph-affine matches,2414 gains and zero losses. Explicit spacing matches all104
source controls,88 gains/no losses. All4999 are permanent full API regressions.
39 math tests pass, zero ignored. Fraction frames and four serializer removals
remain; complete55 controls reach55 root/path/text/error and53 affine matches.
Two tall-radical placement failures remain. CLI/pixels, replay and aggregate543
qualification remain pending. Evidence: .reference/operator-native-20261008/
source-comparison.md and api-progress.json; explicit-spacing-native-20261008
and fraction-frame-native-20261008 complete inventories. Source-derived data
are not expression output tables; algorithms remain native Rust.

### Checkpoint543 follow-up: table prime and retained raw script core

Static source comparison: tex/inheritance.go mtable takes data-cramped rather
than surrounding prime; svg/msubsup.go scriptBaseCore/baseIC/baseIsChar and
munderover.go retain actual mo attributes and variant bounds. Rust table.rs and
styles.rs reset prime on table children; styles::script_core retains RawOperator,
and classes.rs/lib.rs use its inherited largeop and variant correction. Complete
96 table controls and55 prior frame controls now match. Complete252 raw operator
script/font controls improve237 to252 affine matches, no losses. All original
4895 and104 controls remain exact. Word/sum fixed serializer removed;4 actual
source whitespace forms match. All5406 captured controls are permanent in tests,
with full rules and nested viewport attributes/placement added to the gate.
358 library tests (43math,158render,157export), zero ignored, and25 harness tests
pass. CLI build succeeds; frozen API qualification verifies all5406 controls and source/binary bindings. Complete21624 pixel capture is running; prior23/broad/aggregate543 pending;
this is API progress, not overall or pixel parity. Evidence directories:
.reference/{table-prime-native,operator-context-source,word-sum-native}-20261008.

### Checkpoint 715: original native SVG path boundary

`src/svg_path.rs` translates pinned D2 v0.9.0
`d2renderers/d2svgimport/path.go` and `d2renderers/d2scene/path.go` under MPL-2.0.
881 direct original controls qualify absolute geometry bits, separate parsed and
emitted command totals, byte diagnostics and bounded work. Full SVG import/scene
integration remains separate. Evidence: `.reference/svg-path-native-20261013/`.

### Checkpoint 716: SVG transform work and arithmetic

`svg_transform.rs` translates pinned D2 `d2svgimport/transform.go` and
`d2scene/geometry.go` under MPL-2.0. `svg_trig.rs` retains Go BSD and Cephes
notices for Go 1.27 Sincos/Tan reduced-input arithmetic. Original ARM64 compiled
FMA operations are preserved. 749 source controls qualify exact matrix bits,
function counts and errors; complete importer integration remains separate.
Evidence: `.reference/svg-transform-native-20261013/`.

### Checkpoint 718: local SVG references and use graph

`svg_use.rs` translates pinned D2 `compile.go` local-reference admission and
validateUseGraph, with `import.go` validSVGID, under MPL-2.0. 3,381 reference
and684 graph controls match original private stages; complete importer and
scene instance accounting remain separate.
Evidence: `.reference/svg-use-graph-native-20261014/`.

### Checkpoint 719: SVG lengths and use placement

`svg_length.rs` translates pinned D2 `d2svgimport/style.go` numeric helpers under
MPL-2.0; `svg_use.rs` completes `compileUse` reference/x/y/overflow admission.
1,610 numeric and672 placement controls match source bits/errors. Untyped Go
decimal ratios use algebraically equivalent integer ratios. Full scene import
and accounting remain separate. Evidence: `.reference/svg-length-native-20261014/`.

### Checkpoint 720: primitive SVG geometry

`svg_geometry.rs` translates pinned D2 geometry.go and style.go list/trim helpers
under MPL-2.0. 1,490 direct source controls qualify non-image/non-text primitive
fields, path commands, shared counters and errors, including20 context controls.
Complete paint/scene instantiation and importer integration remain separate.
Evidence: `.reference/svg-geometry-native-20261014/`.

### Checkpoint 721: primitive paint and emitted admission

`svg_primitive.rs` translates pinned D2 compile.go/style.go paint/emission
portions and d2scene paint fields under MPL-2.0. 3,072 typed primitive and324
emitted-budget controls match; complete scene instantiation remains separate.
Evidence: `.reference/svg-primitive-native-20261014/`.

### Checkpoint 724: pinned CSS color dependency

`svg_color.rs` and `css_named_colors.json` translate csscolorparser v0.1.8 under
MIT (Copyright 2020 Nor Khasyatillah; see THIRD_PARTY_NOTICES.md). 900 original
parseSolidColor controls match straight NRGBA bytes/errors. Source ARM64 FMA and
signed32-to-byte conversion are retained; complete float/platform qualification
and computed-style integration remain separate.
Evidence: `.reference/svg-color-native-20261014/`.

### Checkpoint 725: computed style and resource lookup

`svg_style.rs` and `svg_resources.rs` translate pinned D2 style.go/gradient.go/
clip.go computed style and local-reference portions under MPL-2.0. 478 style and
504 resource controls match logical values/errors, using validated templates.
Full XML/resource compilation, scene instantiation and renderer integration remain
separate. Evidence: `.reference/svg-style-native-20261014/`.

### Checkpoint 816: prescript multiscripts

`multiscripts.rs` translates pinned mathjax-go internal/svg/mmultiscripts.go,
msubsup.go and tex/commands.go prescript parsing (Apache-2.0, MathJax Consortium).
It preserves primary-marker relative scale, first/sub/sup baseline rules, right
script alignment and live inherited font/mathsize/style semantics. Native root
BBox.Combine arithmetic preserves the placement offset before adding surd height.
276 direct source controls cover complete painted geometry, dimensions and errors;
these do not qualify arbitrary multiscript MML or all TeX/mathtools behavior.
Evidence: `.reference/math-prescript-native-20261015/`.

### Checkpoint 817: sideset and both multiscript pairs

`multiscripts.rs` now handles both pairs used by source AMS sideset, translating
tex/base_ams_handlers.go292–346 and svg/mmultiscripts.go. It retains ignored
argument bodies, braced ORD script frames, primary post-subscript scale/bounds,
left alignment, base/post paint order and property-versus-core-attribute limits.
554 source API controls and prior math regressions compare dimensions, errors
and complete ordered painted geometry. Evidence:
`.reference/math-sideset-native-20261015/`. Full TeX/MML and broader D2 gates
remain separate.

### Checkpoint 818: splitfrac and splitdfrac

`split_fraction.rs` translates pinned mathjax-go tex/mathtools_handlers_commands.go
273–299 and svg/mfrac.go ruleless UVQ, alignment, padding and bounds (MathJax
Consortium, Apache-2.0). The parser retains explicit style/scriptlevel wrappers,
empty identifiers and 1em row padding. Absolute child scale remains distinct from
relative scale in gap calculations. Shared fraction visitors handle both children;
396 source controls compare complete ordered painted geometry, roots and errors.
Evidence: `.reference/math-splitfrac-native-20261015/`. Other fraction/TeX handlers
and whole-D2/platform gates remain separate.

### Checkpoint 865: generic math environment tags and reference classes

`lib.rs` now follows pinned mathjax-go v0.1.0 `tex/environments.go` and
`tex/ams_tags_environments.go`: only equation/equation* introduce the AMS
Equation scope; math/displaymath parse into their surrounding tag scope.
This preserves outer style inheritance when a generic environment contributes a
starred tag. Reference groups retain the exact leading-space MathJax_ref class
from `tex/ams_tags.go` and `svg/wrapper.go::addElementClass`.

The permanent equation-split-nested fixture records 1,008 actual original Go
Measure/Render calls across generic/equation/split environments, nested alignments,
styles, tags, labels, references, malformed input, row gaps and matrices.
Tests compare measurement/error results, root attributes, full painted primitive
inventories, and raw id/href/reference-class attributes. These are explicit
contracts: they do not establish byte-identical SVG markup. Evidence and remaining
raw markup differences are retained in
`.reference/math-equation-split-continuation-20261016/`.

### Checkpoint 866: token character and structural SVG emission

The native emitter follows original svg/wrapper.go textToSVG/placeChar/place:
a token owns its character paths; positions are suppressed only when their
serialized coordinates are zero. dom.go supplies distinct text/attribute escaping.
Script ownership comes from parsed nodes, never inferred glyph coordinates.
Original tex/parser.go attachPrimes creates operator text, while the named prime
and backprime commands remain variant-form identifiers. The latter is now handled
natively. New prime-origin controls and the existing 1,008 equation/split controls
compare complete SVG strings as well as Measure output.

Structural source mappings also include svg/mmultiscripts.go anonymous script rows,
tex/base_ams_handlers.go absent-side none nodes, general-fraction strut rows,
svg/texatom.go attribute installation order, svg/mo.go centering ownership,
menclose.go anonymous child block placement, and AMS table label-width metadata.
Explicit spaces remain mspace nodes without inventing painted primitives. Under/
over marks and mathtoolsVDots retain their mo role independently of named symbols.
Evidence: `.reference/math-token-character-emission-20261016/`. Broader raw SVG
serialization still has explicit failures; source process-panic gaps are unchanged.

### Checkpoint 867: bracket child structure

`brackets.rs` follows original tex/mathtools_handlers_commands.go and
tex/nodes.go::row: a singleton parseString result is the child itself. The
mpadded border retains an empty mphantom SVG child, matching
svg/mpadded.go::paddedToSVG and mphantom.go::phantomToSVG. Its copied argument
still supplies layout bounds while remaining unpainted. The 278 retained original
bracket controls now check ordered padded child structure as well as dimensions,
paint, border CSS and scene admission; basic single/multiple-token brackets also
check whole SVG. Evidence: `.reference/math-bracket-node-ownership-20261016/`.
Other script placement, operator centering and relative-scale differences remain.

### Checkpoint 868: transparent script child placement

`lib.rs` follows original svg/msubsup.go child placement: native parser policy
wrappers such as MovesScripts introduce no MathML row. A single character child
without groups/rules retains its glyph offset when placed into the script frame,
using the existing classes.rs placement helper. This removes synthetic mrow
wrappers without altering measured or painted output. The original 278 bracket
controls now compare both padded children and stack child kinds; eight basic and
summation bracket cases require whole SVG equality. Evidence:
`.reference/math-script-child-placement-20261016/`. Other operator centering,
relative-scale and whole-D2 contracts remain separate.

### Exact d2latex render boundary (render_exact)

`render_exact` is the entry point used by the d2svg LaTeX labels
(`crates/d2-rs-render/src/d2svg/latex.rs`, porting d2svg.go drawShape and
drawConnection's `Language == "latex"` branches around `d2latex.Render`). It
returns the parsed MathJax layout or `MathError::Unsupported`; unlike `render`
it never returns the visible-text fallback. Contract and evidence:

* Oracle: `tools/mathjax-oracle` (own go.mod, requiring the
  `github.com/d2lang/mathjax-go` version of the pinned D2 go.mod) prints
  `mathjax.Render` for a JSON list of TeX strings, or, given the outcome
  ledger, writes the fixture.
* Fixture: `tests/fixtures/mathjax-exact-v090.json` is regenerated (oracle
  tier) by the recipe `mathtext-latex-mathjax-exact`
  (`tools/fixtures/mathtext-latex-exact.toml`) from the committed outcome
  ledger `tools/fixtures/inputs/mathtext-latex/mathjax-exact.json`;
  `scripts/mathjax-exact-fixtures.py` re-derives the outcome classes from
  that oracle and `examples/math-oracle`. `src/mathjax_exact_tests.rs` asserts every case's
  class (match by SHA-256 of the Go SVG, explicit refusal, or a recorded
  mismatch that must still differ until fixed). 2116 match, 30 refused, 3
  recorded mismatches (float-tie rounding of stretched-accent/radical
  viewBox coordinates computed from 1000-unit widths instead of the source's
  em widths).
* Generated tables: `src/source_symbols.rs` is generated by
  `scripts/mathjax-generate-symbols.py` from mathjax-go
  `internal/tex/source_tables_*_maps.go`, resolved as
  `tex/source_runtime.go lookupMJSourceSymbol` (later maps/entries win; names
  owned by the earlier ams-tag/mathtools/physics/amscd/base-AMS handler case
  labels are omitted). `src/tex_variant_glyphs.rs`, `src/large_glyphs.rs`
  and `src/smallop_glyphs.rs` are regenerated with
  `scripts/generate-math-glyphs.py --all-glyphs` from `data_tex_variant_gen.go`,
  `data_largeop_gen.go` and `data_smallop_gen.go` (complete tables).
* Source mapping of the behaviours added for exact output: svg/mo.go moToSVG
  symmetric/largeop `translate(0 u)` (`own_translate`); svg/wrapper.go
  standardSVG `RScale` scale emission, handleAttributes `data-cramped`,
  placeChar/place attribute order (`d` before `transform`, no transform at
  0,0); svg/stretch.go vertical/horizontal assemblies in em; svg/mpadded.go
  paddedToSVG content `g` (colorbox, CD labels); svg/msqrt.go mroot
  (`Atom::Root`: rootDimensions, `w.dx` placement); svg/texclass.go
  setTeXClass early returns (empty mstyle, struts) and setBaseTeXClass;
  tex/parser.go \middle empty CLOSE/OPEN TeXAtoms, "." fences as empty
  symmetric mo, missing close brace; tex/commands.go functionNames
  (commandNamedFunction/ApplyFunction), simpleMacros (mod, bmod, pmod, iff,
  implies, impliedby, not), declareMathOperator, \textbf/\textit mtext,
  physics vectorBold, vectorAccent (\vu), operatorApplication (\grad,
  \divergence, \curl), physicsNabla, physicsDifferential, quickQuadText,
  commutator, expectation value, matrixQuantity; tex/physics_handlers.go
  eval, outer products, matrix elements (parseExpansion); tex/amscd
  CD vertical arrows/labels and munderover child order; tex/inheritance.go
  cleanMathMLAttributes (inherited mathcolor) and operator-form flags
  (mtext spacelike, tex/mml_definitions.go).
* Explicitly refused (no approximation): starred cases, \va, small matrix
  quantities, sized (\Huge etc.) mroot, unknown commands and every other
  path the native parser does not translate.

The exact upstream `LICENSE`, `NOTICE`, and `LICENSES/MIT-Luo-Studio.txt`
are copied into this crate's `licenses/mathjax-go-*.txt`; their bytes can be
verified against the canonical module cache for mathjax-go v0.1.0.

### Master viewport rounding (run 50, 2026-10-25)

`svg_viewport::aspect_matrix` now follows master 6a7b48378
`d2renderers/d2scene/aspect.go:28–54`: separately rounded dimension products
and direct translation subtraction, without FMA or matrix composition.
This resolves five original LaTeX Build calls with residual translation
mismatches. The existing 2,280-row viewport fixture was refreshed only for
AspectRatioMatrix results using the pinned public Go API; viewport.go's
admission logic is unchanged from v0.9.0. Reproduction: recipe
`mathtext-img-svg-viewport` (`tools/fixtures/mathtext-img.toml`) reruns the
whole viewport/AspectRatioMatrix sweep at the pin into the oracle cache as
`svg-viewport-master.json` with exact matrix bits; the v0.9.0 capture and its
refresh tool were retired (base 9b706ac1, docs/fixture-retirements/mathtext.md).

## Run 53: shared imageasset SVG admission (master 6a7b48378)

`src/image_svg/` now owns the existing native Go-compatible XML lexical,
entity, namespace, name-range, document-envelope and four-codec helpers moved
from d2-rs-export. Exporter call sites use the same functions and unchanged
legacy entry point; the 342 original codec outcomes remain their regression
fixture. MPL/BSD/MIT notices moved with the source and remain in the exporter.

`resource.rs` and `svg_asset_xml::validate_bounded` map to master
`lib/imageasset/probe.go:39–227,418–590`: byte-derived finite XML work,
256-level depth, caller SVG/encoded/decompressed limits, raw versus declared
compressed precedence, source-byte-preserving ISO-8859-1 conversion,
canonical external doctypes with entities disabled, and gzip/zlib/raw-DEFLATE/
Brotli expansion. The shared legacy entry point retains its previous limits
and diagnostic behavior. No geometry or floating-point operations changed.

Original source calls and final measured counts are recorded in run53's
imageasset scoreboards and d2-rs-scenebuild fixtures; this is acquisition
admission, not arbitrary SVG importer support or full HTTP imageasset parity.

## Run 54: HTTP content encoding layers (master 6a7b48378)

`image_svg/http_content.rs` translates `lib/imageasset/resolve.go:870–902`
`decodeContentEncodings`: reverse application order, identity, gzip/x-gzip,
zlib/raw-DEFLATE fallback and Brotli, with a caller ceiling on every intermediate
layer and cancellation between reads. It reuses the retained Brotli adapter
and its existing notices. Existing SVG/export entry points are unchanged.
Six original codec Resolve calls, stacked intermediates and decompression/encoded
limit calls are retained by scenebuild's `imageasset-http-master.jsonl`.
Malformed-codec error strings beyond those original calls remain unqualified.

## Master LaTeX input admission (Lane L26)

`src/input_limits.rs` translates master `d2renderers/d2latex/limits.go:8–67`
(6a7b48378). Render/exact-render validate before recursive parsing; Measure
shares Render's admission. `latex_common.go:10–30` validates outside the
MathJax parse-error wrapper, so `d2-rs-language` preserves InputLimit errors
without adding `latex failed to parse`. UTF-8 validity is guaranteed by Rust
strings. Tests cover bytes, exact/over-depth, comments, escaping parity and
unmatched closes. Original `d2lib/latex_limits_test.go` calls are bound by
`scripts/library-api-scoreboard.py` alongside the original active assertions.
