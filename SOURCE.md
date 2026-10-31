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
DEPENDENCY_NOTICES.md and this file with every binary/WASM distribution and
crate. Source recipients must receive the MPL-covered modifications needed to
build their version. Third-party material retains its applicable terms.

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
CLI/raster and aggregate gates remain pending; nearly-full goal active.

## Native ELK Coffman–Graham and first model-order sweep (2026-10-17)

Pinned elk-go v0.2.0 internal/layered/p2layers/coffman_graham.go
and javacompat GWT priority queue semantics map to
crates/d2-rs-elk/src/layered/coffman_graham.rs and gwt_priority_queue.rs.
The source transitive reduction, reverse history comparator, heap ties,
layer capacity and component scopes remain native Rust. layered.rs imports
source registry ordinal3/name COFFMAN_GRAHAM and layerBound, dispatches shared
external constraints, and reads finite decimal/suffixed DoubleOption forms.
Hex/nonfinite and broader option parsing remain unqualified.
SimpleRowGraphPlacer/AbstractGraphPlacer map to the single-component publication
transfer; node+port then graph offset association applies even with one component.
LayerSweepCrossingMinimizer.sweepReducingCrossings passes its model-order flags
through BarycenterHeuristic.MinimizeCrossings' negated firstSweep argument;
the first initial-order attempt preserves unknown barycenters. Evidence and
retained failures: .reference/elk-coffman-graham-20261017. Final qualification
is recorded separately; these selected contracts do not establish full ELK parity.

## ELK component P3 lifecycle and stored BK block coordinates (2026-10-17)

See crates/d2-rs-elk/SOURCE.md checkpoint907. Native component/root sweeps share
source trial/reverse-sweep semantics, including zero thoroughness and two-sided
greedy port distribution. BK candidate scoring retains stored block Y rather
than reconstructing it from rounded published coordinates. Source mappings,
ordinary-Go fixture bindings and retained larger-graph failures are in
.reference/elk-component-p3-trials-20261017. Selected contracts do not establish
full ELK or full D2 parity.

## ELK P1 component discovery and saved-sweep port constraints (2026-10-17)

See crates/d2-rs-elk/SOURCE.md checkpoint908 and
.reference/elk-p1-component-order-20261017. Source component DFS/port adjacency
and independent model-order ties correct P1 reversal choices; fused randomized
barycenter fill and saved-sweep port-order publication preserve Go behavior.
Whole-component scheduling/RNG and broader D2 gates remain open; selected output
and phase contracts do not establish full trace or overall parity.
