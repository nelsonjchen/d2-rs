# d2-rs-isometric source mapping

## Physical Page source perimeter (Lane I, iso42)

`tests/page_glyph.rs` replays the original
`d2isometricimg/native_page_test.go:50–92` call to
`native_page.go:60–82` (`img/native_page.rs::native_remove_page_glyph`).
The Go test builds its real dashed, patterned Page scene and keeps every
original assertion active. The native replay takes that complete source root
as input, compares the entire resulting root, and verifies that the outer
stroke stays in place while the inner flat fold loses its stroke. Scene
construction and Go allocation layout are outside this observation.

`scripts/isometric-page-glyph-scoreboard.py` uses the existing bounded capture
and strict structural comparator. The complete fixture is
`tests/fixtures/page-glyph-api.json`; no runtime change or golden gain was
needed. The separate scene URL/Userinfo input limitation remains open.

## Exact-time capture for GIF/timeline (Lane F, run62)

`img/gif_capture.rs` translates master `render.go:117–150` (CaptureFrames),
`native.go:24–90` (capture ownership, independent scene/packet clocks, frame
encoding), `geometry.go:687–728` (traffic spheres) and
`source_panels.go:281–346` (authored node motion and animated panel textures).
The shared NativeScene now retains packets, panel documents, animated triangle
ranges and pixel scale from finishNativeScene. Existing still/SVG consumers
keep the same geometry. The ARM64 `source_panels.go:293` FMSUBD uses
`go_fmsub(4*scale, pulse, z)`; see the retained fma-sites inventory.

`raster.rs::render_frame` anchors view and both shadow cameras to the original
mesh and draws dynamic packets after static depth, combining opacity groups
as upstream renderStrips does. The current capture recomputes static rasters
per requested frame; a retained raster cache/performance claim is not made.
`backends.rs::raster_surface_at` forwards exact nanosecond time into d2raster.
Each PNG is caller-owned; callback failures retain their typed error source,
and cancellation is checked after every callback, including the final one.

`scripts/isometric-frame-scoreboard.py` observes original CaptureFrames calls
with original Go assertions active, plus five named whole-board witnesses
using upstream captureDiagram/sourcePanelFixture inputs. It compares every
PNG through the shared raster gates and then complete encoded bytes, including
prefix frames on cancellation/callback failure. The compact native regression
is `tests/fixtures/gif-frames-api.json`. This qualifies exact-time capture;
GIF palette/loop encoding, RenderTimeline and CLI GIF remain unimplemented.

Lane I iso14 adds whole-image observations of master
`page_test.go:172` (board-target/file destination distinctions) and
`animation_page_regression_test.go:105` (outside Markdown links), with
their original assertions active. The `links` API suite compares all SVG,
PNG and RenderPage bytes/annotations; `tests/fixtures/link-boards-api.json`
retains the compact native regression contract. The `still` suite observes
the original public Render calls from `TestNativeDeterministicCapture` and
`TestSVGVisibilityLargeSources`. CaptureFrames/GIF assertions are not native
coverage from this suite. No renderer algorithm changes were needed.

Port of D2 master 6a7b48378 `d2renderers/d2isometric` (and, as it lands,
`d2isometricimg`). Input is the compiled `d2target.Diagram` JSON; Go's
BuildScene itself round-trips the diagram through encoding/json.

| Go | Rust |
|---|---|
| d2target/d2target.go Diagram, Config, ThemeOverrides, Shape (embedded Class, SQLTable, Text), Connection, net/url.URL + URL.String | `src/target.rs` |
| d2themescatalog.Find, Theme.ApplyOverrides, d2themes.ResolveThemeColor, lib/color.IsThemeColor | `src/theme.rs` (catalog from d2-rs-core) |
| d2isometric/scene.go BuildScene, budget, sourceOrder | `src/scene.rs` build_scene, Budget, source_order |
| d2isometric/hierarchy.go, layout.go | `src/scene.rs` compose_hierarchy, hierarchy_route/fallback, module_size, board_label_metrics |
| d2isometric/sequence.go classifySequence | `src/scene.rs` classify_sequence (LifelineEndID: FNV-1a 32) |

Floating point: ARM64 contraction sites come from the pinned compiler's
`-gcflags=-S` output, tabulated in `.reference/isometric-port/fma-sites.txt`
(regenerate: `go build -o /dev/null -gcflags='github.com/d2lang/d2/d2renderers/d2isometric/...=-S' ./d2renderers/d2isometric/...`
in ../d2 with the pinned toolchain and caches). Products by 0.5 are exact and
written plainly. Go math functions come from `d2_rs_render::go_float`.

Verification: `scripts/isometric-scoreboard.py` (tiers `scene`, `svg`) runs
`tools/isometric-probe/main.go` (built in ../d2-trace as
`.reference/bin/d2-isometric-probe-6a7b4837`) and `examples/isometric-probe.rs`
over every e2e golden board.

## Retained vector surfaces (Lane I, iso1)

| Go at master 6a7b48378 | Rust |
|---|---|
| d2isometricimg/vector_surface_svg.go nativeSurfaceSVG, nativeSVGNumber/Matrix/Attr, nativeSurfaceSVGWriter node/path/paint/style/primitive/surfaceBounds | `src/img/vector_surface_svg.rs` |
| vector_surface_animation.go nativeVectorInitialNode/Paint | `vector_surface_svg.rs` initial_node |
| vector_surface_text.go font, text, nativeSVGGlyphPath, glyphDefinition/glyph | `src/img/vector_surface_text.rs` |
| vector_surface_color.go colorGlyph/colorPaint | `src/img/vector_surface_color.rs` |
| vector_surface.go nativeVectorViewport and nativeVectorSurface fields | `vector_surface_svg.rs` viewport; `vector_surface.rs` (mesh registry remains WIP) |
| internal/rasterimage/firstframe.go static PNG DecodeFirst; Go 1.27 image/png/writer.go Encode, writePLTEAndTRNS, writeImage/writeIDATs; Go bufio.Writer | `src/img/backends.rs` normalized_png; uses d2-rs-goflate's native default-compression zlib |

The PNG row predictor and 32 KiB IDAT sink reuse the existing translated
Go implementation in d2-rs-export/src/go_png.rs, adapted to default
compression and Go's grayscale/palette/16-bit output selection. That Go
stdlib portion is BSD-3-Clause (Copyright 2009 The Go Authors), covered by
the root THIRD_PARTY_NOTICES.md. Isometric and D2-source portions are MPL-2.0.
This adapter is local because the foundation crate's first-frame asset
normalizer is private. JPEG, GIF, WebP and animated PNG remain explicit
unsupported branches here; do not infer full raster-asset API parity from
the built-in grain PNG witness.

Number formatting follows strconv's shortest digits and decimal ties to
even, including the float32-derived glyph coordinates that differ from
Rust's default shortest formatter. COLRv1 colorPaint fuses the **first**
product at Go lines 82 and 87 and subtracts fused products at line 88;
confirmed with the pinned Go compiler's ARM64 assembly. Exact scaling by
1/64 in glyph paths and by 1/2 in strikethrough decorations preserves the
corresponding Go fused results.

`scripts/isometric-scoreboard.py --tier surfaces` feeds Go's meshes and
retained d2scene documents into the native writer, and compares complete
SVG bytes only after Go independently reproduces the upstream golden. It
qualifies surface serialization plus the existing writer, **not native mesh
construction**. The `writer` tier still replaces surfaces explicitly. The
new hook JSON uses the same d2scene interface tagging as tools/scene-probe;
font bytes are stored once per capture by SHA-256, with DataFile references.
Successful source/native SVGs are bound to the pristine golden hashes and
retired immediately; initial stubbed negatives have exact reconstruction
records (`stub_surfaces(golden)`) and hashes. Failed meshes remain witnesses.


## Plain surface labels (Lane I, iso2)

| Go at master 6a7b48378 | Rust |
|---|---|
| d2isometricimg/text.go newTextPainter, face, texture, normalizeLabel, fitNativeLabel | `src/img/text.rs` (bundled plain faces; fallback remains explicit unsupported) |
| surface_document.go surfaceTextureDimensions[AtDensity] | `text.rs` surface_texture_dimensions_at_density |
| vector_surface.go nativeVectorContext, retainNativeVectorSurface, nativeVectorForTexture | `vector_surface.rs` serial registry (the mesh builder's shared mutable apertures/coverage remain gated) |
| surface_document.go rasterNativeSurfaceDocument | `backends.rs` raster_surface (native d2-rs-raster, premultiplied texture conversion) |
| golang.org/x/image v0.45.0/vector/{vector,raster_fixed,raster_floating}.go | `text_raster.rs`: plain-label paths, 9-bit fixed/floating coverage and Uniform Over |

The x/image translation is Copyright 2016 The Go Authors, BSD-3-Clause,
covered by the existing Go notices in THIRD_PARTY_NOTICES.md. Its curve
interpolation contracts differ from d2raster's scene scan converter. Pinned
Go 1.27 ARM64 instructions fuse each de Casteljau interpolation at vector.go:52
and the **second** squared deviation at line 260. The fixed/floating line
integrators retain explicit rounded products. text.go's 1/2 and 1/64 fused
products are exact scalings; the products before glyph translation are
explicitly rounded in Go and remain unfused. Extracted evidence:
`.reference/codex-runs/iso2-{vector,text}-instructions.txt`.

`isometric-scoreboard.py --tier text` captures actual `textPainter.texture`
calls during source mesh construction, independently reproduces each complete
Go SVG golden, then replays those calls natively. Every call compares layout,
texture dimensions, the complete premultiplied RGBA SHA-256, and all retained
SVG bytes. Empty plain-call inventories are reported separately; Go resource
errors never pass. This is a label-painter tier, not native mesh construction
or whole isometric SVG/PNG parity. The trace-only test wrapper changes only
the original method name while installed and restores the exact source bytes
on exit. A fresh export registry keeps texture identities alive for the
capture; no host font discovery or external engine is used.

Complex/color fallback remains unsupported, currently witnessed by all three
`unicode/emojis` boards. d2-rs-scenebuild's public FontFallbackOptions has only
a resolver boolean and rejects configured resolvers. The remaining source
surface_fonts.go fallback/document path needs a bundled font fallback API
from the foundation lane (or an explicit local adapter). Custom registered
font families, rich-label and icon painters are not qualified by this tier.

## Rich-label and fallback prerequisites (Lane I, iso3)

| Pinned Go source (6a7b48378) | Rust |
|---|---|
| d2isometricimg/rich_labels.go:20–210,222–393 | `img/rich_labels.rs`: painter/source/pixel budgets, minimal owned target, theme/family selection, viewport fitting and raster options |
| surface_fonts.go:60–183 | `img/surface_document.rs`: bounded native bundled resolver and strict source-glyph coverage |
| surface_fonts.go:210–315 | `surface_document.rs` fallback_text_texture, native scene shaping, ink bounds/alignment, RGBA composition and retained surface |
| surface_document.go:21–51,153–181 | `surface_document.rs` native_font_family, `text.rs` texture allocation |
| vector_surface.go:93–150 | `vector_surface.rs` retain_styled_native_vector_surface: preserve physical viewport fitting before normalizing |
| rich_labels.go:123–164 / surface_fonts.go:285–315 / Go image/draw drawNRGBAOver, drawCopyOver | `surface_document.rs` straight/premultiplied composition and object-opacity fade |
| vector_surface_svg.go:455–496 | `backends.rs` normalized_raster_png delegates first-frame PNG encoding to d2-rs-raster (supersedes the iso1 local encoder) |

fitRichViewport's half scaling is exact; scene matrices use the qualified
shared Matrix. surface_fonts.go:248–249's synthetic bold/italic ink bounds
use the single-product FMSUBD and FMADDD recorded in the pinned ARM64 site
inventory. readableSurfaceInk (used for an absent structured
color) preserves geometry.go:383/389's audited second-product and running-sum
FMAs from `.reference/isometric-port/fma-sites.txt`. Custom registered font
families still require a foundation registration API; this adapter never scans
host fonts. The original Go wrapper bodies are unchanged while capturing, and
are restored byte-exactly after the serial producer exits.

The `--tier rich` scoreboard measures actual `richLabelPainter.texture` calls
from mesh construction, complete viewport values, premultiplied RGBA hashes,
and every retained SVG byte. Source SVGs independently reproduce upstream
isometric goldens. Empty inventories and source errors remain explicit. This
captures Markdown/code/LaTeX calls; class/SQL cell generation bypasses this
painter and is **not qualified** by this tier. Its minimal document construction
is ported for the pending structured mesh path. Full native meshes and end-to-end
SVG remain gated. Fixtures retain actual Markdown/code and emoji board calls,
without changing their source requests or expectations.

## Native tessellation core (Lane I, iso4)

| Pinned Go source (6a7b48378) | Rust |
|---|---|
| d2isometricimg/geometry.go:140–276 triangle, flat, box, sphere, cylinder | `img/geometry_core.rs` MeshGeometry append/error budget and primitives |
| native_shapes.go:642–747 nativePolygonArea, extrudedProfile | `geometry_core.rs` winding, smooth convex wall normals and bounded ordered ear clipping |
| geometry.go meshBuilder.triangles/err | the gated higher-level `geometry.rs` builder owns and delegates to MeshGeometry; no second tessellator |

`MeshGeometry::with_prefix_count` carries the source mesh's already-retained
triangle count into serial replay without copying its earlier triangles.
A normal builder has a zero prefix. The million-triangle ceiling, transparent
material rejection, first-error latch and four-million-work triangulation
ceiling follow the source. Context cancellation is not yet exposed by this
core; these golden calls use an un-canceled source context.

`isometric-scoreboard.py --tier primitives` observes actual **outermost**
box/sphere/cylinder/flat/extrudedProfile calls during the complete source
mesh build. Nested calls execute their original bodies and are represented
in the outer call's complete ordered output, avoiding duplicate replay.
Every material value/identity, vertex/normal/UV, shadow flag and error compares
as JSON float64 values without tolerance, after Go independently reproduces
the full upstream SVG. Input material pointer aliasing and nils are explicit;
textures retain dimensions and vector pointer identities, not their pixels
(the tessellator does not read those pixels). Source wrappers rename only
method declarations in the trace checkout, restoring both files exactly on
exit. Nonempty call boards, empty inventories and source resource errors are
reported separately. This is a tessellation prerequisite for the failing
native mesh/SVG tier; complete scene assembly is still gated. Static SVG
boards do not call sphere/cylinder, so this sweep does not qualify those
operations' outputs or animated frames.

ARM64 sites: geometry.go:78 in sphere.func1 fuses each scaled coordinate into
its center. native_shapes.go:646/669/705 subtract the second product with
FMSUBD for signed area, convex turns and ear tests. Explicitly rounded
vector products/normals stay unfused. Regenerate the assembly using the
manifest toolchain and absolute shared GOMODCACHE/GOCACHE, after the capture
has restored its test-only method names; extract with
`tools/isometric-port/fma-asm.py`.

## Whole-board native SVG assembly (Lane I, iso5)

The active `img::render::render_svg` path builds native geometry and retained
surface documents before invoking the existing SVG writer. It currently
rejects nonempty edge inventories explicitly; route assembly, endpoint
visibility, icons, raster/GIF export and general context APIs remain open.
The whole SVG scoreboard continues to inventory those failures. The earlier
statements that **all** native mesh assembly is gated are superseded by this
slice; no new component qualification tier was added.

| Pinned Go source (6a7b48378) | Rust |
|---|---|
| render.go:78–121; svg.go:23–87 | `img/render.rs` vector options, dimensions, metadata preflight, scene/build/writer boundary |
| geometry.go:420–692; sequence_geometry.go | `render.rs` source-order boards, node dispatch, hierarchy headers, animation ranges, links, sequence roles and paint/shadow ownership |
| native_structured.go | `img/native_structured.rs` source-coordinate class/SQL rows, rails, independent text cells, source and pixel admission |
| opacity_groups.go:10–49 | `img/opacity_groups.rs` shared paint-owner identity and opacity ownership |
| source_panels.go:22–276 | `img/source_panels.rs` legends, root captions/frames, background paint and final native camera (static SVG path) |
| ground_receivers.go:35–102 | `img/ground_receivers.rs` containment domains and independent ground receivers |
| gradient_paints.go:23–119,165–216 | `img/gradient_paints.rs` retained gradient documents, material extents/UVs, summary color and organizational opacity |
| vector_surface.go | `img/vector_surface.rs` shared mutable document/aperture/coverage state, preserving Rc identities across material references |
| geometry_routes.go:21–48,138–167 | `img/geometry_routes.rs` path lengths/interpolation and watertight tube tessellation used by physical outline ink; routed-edge assembly is still open |

Foundation boundaries stay in `backends.rs`: face-only scenebuild, native
raster-to-premultiplied texture conversion, icon-size and gradient-parser
adapters for currently private foundation APIs. No foundation crate is edited.
The local `math.rs` adapters use the shared Go trig kernels; asin follows Go
math/asin.go using the same positive-quadrant atan kernel. Existing WIP
floating-point contraction annotations remain in the activated shape/hierarchy
modules. New contraction sites are checked against the pinned ARM64 compiler:
`iso5-source-panels-instructions.txt` and `iso5-structured-instructions.txt`.
Background composition fuses the **first** color product, after rounding the
backdrop product; structured copies, cells and outside icons follow the
recorded FMADDD/FMSUBD sites. Explicit `float64(...)` products stay rounded;
half products are exact in the admitted normal coordinate range.

The root-frame/legend witness exposed `nativeBoundsTarget`'s use of
`d2target.BaseShape` (d2target.go:886): its invisible measuring shape still
has a two-unit stroke. Its source bounds are therefore wider than a zero
Rust-default shape. `rasterNativeSurfaceDocument` also retains its **final**
logical pixel size and centered meet viewport (surface_document.go:123–152),
which the source-panel adapter must preserve. The per-board mesh comparison
records the first differing panel vertices, camera and document viewport.
`isometric-mesh` (requiring `--features diagnostic-traces`) is an example-only debugging adapter for
TestD2rsMeshDump, not a scoreboard tier or a library trace path. Original
content-addressed font bytes are materialized losslessly when comparing the
complete retained documents.

`tests/fixtures/whole-svg.json` retains original diagrams and independently
reproduced complete SVGs for measured/empty-shape, patterns/3d and
root/stroke-width, including source pin and theme. `whole_svg.rs` exercises
that complete public boundary and verifies that unported routed boards fail
explicitly instead of returning partial diagrams. Full-corpus results and
remaining failures are recorded in STATUS and report-iso5.md.

## Whole-board route assembly (Lane I, iso6)

All references are `d2renderers/d2isometric/d2isometricimg` at master
**6a7b48378**. Routes consume the existing scene's source-owned coordinates;
all lane/bridge/contact output paths are independent writable vectors.

| Pinned Go source | Rust |
|---|---|
| geometry_routes.go:20–285 | `img/geometry_routes.rs`: route lengths/sections/dashes, rounded corners, tubes, marker dimensions/clearance and authored marker geometry |
| geometry_routes.go:332–555 | `img/routes.rs`: two-pass reservations, dependency wire/casing/markers, authored and automatic captions, animated packet bounds |
| route_lanes.go | `img/route_lanes.rs`: bounded shared-run sweep, obstacles, fan-outs and corner joins in stable edge-ID order |
| route_bridges.go | `img/route_bridges.rs`: transverse crossing sweep, merged local ramps and lower-first clearance |
| arrow_visibility.go | `img/arrow_visibility.rs`: endpoint-owned solid/cap proxies, bounded marker/cache queries and swept convex overlap intervals |
| route_style.go | `img/route_style.rs`: FNV route tint and flat unlit ink/casing |
| route_caption_style.go, route_caption_mask.go | matching `img/*.rs`: compiled anchors/dimensions, projected half-plane subtraction of the actual dashed triangles |
| sequence_routes.go | `img/sequence_routes.rs`: sequence/ordinary partition, source-aligned caption APIs, flat lifelines/messages, source dash attributes and markers |
| geometry.go:450–465,599,616–627; source_panels.go:250 | `img/render.rs` edge rich-label counting and completed arrow-owner bindings; `source_panels.rs` packet framing bounds |

`d2target` connection caption/icon/arrow-dimension helpers are thin local
adapters because the existing renderer's methods are private. They reuse the
foundation's public `golib::{geo,label,svgpath}` algorithms. Exposing those
shared target helpers would remove the adapters; no foundation paths change.
Nil side materials for printed marker/cap polygons use an alpha-zero material
with the tessellator's existing no-output contract.

ARM64 source mapping is checked against a fresh pinned Go 1.27 `-S` dump.
Compact instruction witnesses are `iso6-{lanes,arrow,caption}-instructions.txt`
and the route/marker/mask excerpts recorded with the run. In particular,
bridge intersection and projected-mask sums fuse the second product, while
route samples, bridge lifts, source caption anchors and visibility projections
retain their source-defined contraction sites. Diagnostic mesh construction
remains confined to the `diagnostic-traces` example; normal builds construct
no diagnostic arguments. Context/cancellation, general asset resolution and
non-SVG image/timeline APIs remain separate open work.

## Surface icons and explicit assets (Lane I, iso7)

`img/surface_icons.rs` translates `surface_icons.go:23–301` at master
**6a7b48378**: per-render source/content/texture identity caches, explicit
external-resolver admission, data-URI defaults, source/decoded/pixel budgets,
caller-tightened SVG import limits, aspect-fit dimensions, rounded clips and
retained vector documents. `img/render.rs` mirrors `geometry.go:436–477`
for source-order icon counting and painter/output-density configuration.
`Options.assets` hands the caller's foundation resolver through unchanged.
No external I/O occurs by default.

The foundation importer/arena ownership bridge is private. `img/backends.rs`
therefore contains a thin conversion from the public native math SVG arena
to d2scene nodes/assets; parsing, compilation, viewport math, resource
admission, first-frame normalization and rasterization stay in the shared
foundation crates. Exposing that ownership bridge from scenebuild would
remove this adapter. No foundation crate changes are included.

The painter has no new multiply-add contraction sites: its dimension/radius
calculations preserve the separate source products. Existing icon layout and
rounded-clip ARM64 annotations remain unchanged. The mesh debugger remains
behind `diagnostic-traces`.

`tests/fixtures/whole-assets.json` (regenerated by recipe
`isometric-whole-assets`, tools/fixtures/renderexport-isometric.toml, from the
seed `tools/fixtures/seeds/isometric-whole-assets.json`) holds three original
complete diagrams, their original SVG/JPEG resources and independently
produced Go SVGs. The
whole-board test also checks each unconfigured source error. Probe
`assetSources` maps unchanged original HTTP locators to hash-bound files;
its injected transport cannot contact the network. Scoreboard
`--asset-manifest` records `source-match` separately from golden matches,
because upstream's unconfigured corpus expects these external-source errors.
All nine Dagre/ELK/TALA resource-profile boards are recorded in iso7 evidence;
the ordinary corpus remains 870 SVGs plus 138 exact source errors.
General cancellation/timeout APIs and all possible image/import formats are
not established by this finite corpus.

## Static mesh rasterization and PNG (Lane F, run56)

Pinned Go source: `d2renderers/d2isometric/d2isometricimg` at **6a7b48378**.

| Go source | Rust |
|---|---|
| raster.go:165–258,396–472 | `img/raster.rs`: admission, orthographic camera, 2x sample buffers and bounded 64-row strip rendering |
| raster.go:475–571,623–673 | `raster.rs`: mesh/texture storage and aggregate budgets, canonical oriented edges, top-left ownership, row-span clipping |
| raster.go:691–844 | `raster.rs`: directional depth/opacity shadow maps, implicit per-hierarchy ground receivers and opaque background |
| raster.go:885–1119 | `raster.rs`: stable opaque/transparent passes, barycentric depth/normals/UVs, linear-light shading, premultiplied texture interpolation, source-over/multiply and rounded downsampling |
| opacity_groups.go:51–212 | `raster.rs`: shared fade/paint-owner units, depth ordering, bounded backdrop copies and one final object fade |
| render.go:95–112; native.go:24–48,73–90 | `raster.rs::render_png`: native BuildScene/mesh to static PNG, defaults/dimension limits and bounded default PNG encoding; `render.rs::build_native_scene_for_raster` selects ordinary solid caps |

The already translated `mesh.rs` camera/material/light helpers and
`svg.rs::raster_ground_triangles` are shared. ARM64 witnesses:
`.reference/codex-runs/run56-{raster,opacity}-instructions.txt`. Rebuild the
listing in the manifest's trace checkout with shared absolute caches:
`go build -gcflags='github.com/d2lang/d2/d2renderers/d2isometric/d2isometricimg=-S' ./d2renderers/d2isometric/d2isometricimg`.
Canonical edge subtraction fuses its second product; barycentric/weighted
sums round the middle product then fuse the first and third. Opacity-group
interpolation and gradient-background composition fuse the backdrop product.

`isometric-scoreboard.py --tier png` inventories all 20 upstream TestPNG
goldens, requires independent Go reproduction, and compares exact decoded
pixels through both prescribed raster gates. `png_hook_test.go` captures
ordinary raster meshes and complete premultiplied texture bytes in the trace
checkout; its helper names do not share state with other capture hooks.
`examples/isometric-raster` scores those meshes; `examples/isometric-png`
exercises the native whole-board path. `tests/fixtures/png.json` retains all
11 original compiled inputs and all 20 independently reproduced PNG hashes;
`whole_png.rs` tests complete native output. No compositor tolerance,
normalization or SVG-only coverage mesh is used. Static PNG does not qualify
Raster.Frame, GIF/timeline, caller-context cancellation or complete image APIs;
those ports and CLI wiring remain open.

## Public page export (Lane I, iso8)

At the same **6a7b48378** pin, `img/render.rs::normalize` follows
`render.go:61–96`: format and dimension admission, GIF aggregate pixels,
signed timeout limits and link-budget validation in source order. The SVG
convenience API selects SVG and shares that admission and caller link budget.
`img/page.rs::{render_page,render_page_with_context}` translates
`page.go:21–131,285–316`: PNG plus final fitted dimensions, preflight metadata,
shape/connection/inline annotations, semantic board destinations, clipping and
stable decreasing-area order. Mesh annotations use the caller's link budget.
PNG pixels and encoding use the existing native raster foundation. No new
multiply-add sites were introduced; projection uses the qualified camera.

`tools/isometric-probe/api_hook_test.go` observes the six original public
page/unsafe-link test functions with every assertion intact, installed only
in the manifest's trace checkout. The `api` scoreboard tier checks all 48
calls, complete PNG bytes and annotation records/errors, with no normalization.
It validates every successful PNG, nonempty/complete call inventories, pinned
input/expectation hashes, process completion and streaming storage limits.
`tests/fixtures/page-api.json` preserves all 29 original RenderPage calls;
`tests/page_api.rs` also verifies unchanged input and typed cancellation causes.

This establishes these static page contracts, including already canceled
callers. Context polling inside synchronous mesh, raster and encoding stages
is still open; this API checks between those stages and within page metadata
and projection loops. It does not establish full renderer context parity,
CaptureFrames, GIF or timeline support. These limitations are explicit at the
public context API, rather than inferred from successful static boards.

## SVG caller context and timeout (Lane I iso9)

At the manifest's master pin, `img/context.rs` and
`render.rs::render_svg_with_context` translate the SVG branch of
`render.go:99–107` and `svg.go:21–85`: normalize before rendering, establish a
caller-owned child context with the admitted timeout, reject a canceled caller
before nil-diagram/metadata checks, and retain typed canceled/deadline causes.
The existing `render_svg` convenience API delegates to that path.

Cooperative polling follows `geometry.go:139–155,436–518` (every 1024 appended
triangles and between boards), `text.go`/`rich_labels.go`/`surface_icons.go`
(texture admission, including cache hits), `svg_visibility.go:48–64,343–359`
(shared work budget; the work limit precedes cancellation), and
`svg.go:94–108`/`svg_order.go`/`svg_lighting.go` (writer, paint-order and shadow
visibility work). Native stage messages retain their prefixes and typed
context sources. No numeric geometry or contraction sites change.

Evidence: original `TestNativeSVGVectorGeometryAndText`,
`TestNativeSVGAdmissionAndCancellation`, and
`TestNativeSVGLargeRichTextAndPaperStayWithinBudget` calls plus three bounded
public-SVG deadline witnesses on `stable/all_shapes` (ELK),
`stable/giant_markdown_test` and `patterns/paper` (Dagre). The Go API tier compares
complete SVG bytes; the portable `svg-api.json` regression fixture retains
complete requests and original byte counts/SHA-256 expectations. In-stage
writer cancellation, cached text cancellation, geometry polling and typed
caller causes are additional native contracts.

This is cooperative SVG interruption. It does not interrupt a foundation
resolver/importer/font/raster call while that synchronous call is executing,
and does not establish independent wall-clock race precedence without a poll.
PNG/Page in-stage cancellation, CaptureFrames and GIF/timeline remain separate.

## PNG and page capture context (Lane I iso10)

At the same master pin, `render.rs::render_png_with_context` and the shared
`render_static_png` path translate `native.go:24–90` for still frames: option
admission and the caller check precede the child timeout; BuildScene errors
precede the first child-deadline poll; native mesh construction receives the
same context as its painters and geometry. Frame and encoder errors retain
their source stage prefixes. `page.rs` reuses this path after its caller-only
metadata preflight, and preserves typed causes even on mesh-stage errors.
The child deadline never cancels the caller. No numeric geometry changed.

The PNG API suite observes the public Render calls from original
`TestAdmissionBeforeRendering` and
`TestContentFramingFitsFinalGeometryAndKeepsFixedAPI` with all original
assertions active. Other internal/CaptureFrames assertions in those tests
are Go-only, not claimed as native matches. Six additional bounded deadline
calls use the existing `stable/all_shapes` (ELK),
`stable/giant_markdown_test` and `patterns/paper` (Dagre) inputs: one Render
and one RenderPage call per board. Before the fix, all three Render calls
published PNGs after the timeout; the Page calls already rejected them.
The permanent `png-api.json` fixture retains complete requests and exact
errors or PNG byte count/SHA-256. All twenty native PNG goldens also exercise
the new context entry point. A native resolver callback contract verifies
that PNG and Page retain typed cancellation from the existing post-resolver
mesh poll before decoding or publishing output.

Rasterization, PNG encoding and foundation resolvers/importers/fonts remain
synchronous: cancellation is observed between these calls and inside the
cooperative mesh path, not during a blocking foundation call. The older
Lane F-owned `raster::render_png` convenience entry point delegates to this
same path (run60), preserving the String error boundary and format selection.
`tests/raster.rs` repeats I10's three named-board 1 ns deadline witnesses
through that convenience API. CaptureFrames and GIF/timeline remain Lane F work.

## Lane I11: container-gradient admission and whole-image paint calls

`img/backends.rs::gradient_parts` is the thin `lib/color/gradient.go:27–144`
`ParseGradient` adapter used by `gradient_paints.go:194–220`
`nativeGradientOpacity`, called for organizational container washes by
`hierarchy_boards.go:297–324`. It now rejects malformed/empty stops and
nonfinite or nonnumeric positions before the opacity rewrite, preserving the
original stop index, quoted source and stage error. Valid numeric/percentage
positions use the existing native Go float formatter, including shortest
rounding and signed zero. The foundation crates are unchanged.

`tools/isometric-probe/api_scoreboard.py --suite paint` captures every public
image call in four original upstream functions (25 calls): authored gradient
roles on `txtar/gradient/dagre`, container gradients on
`regression/cylinder_grid_label/elk`, the magnified-icon translation test and
the icon-footprint test. Their original assertions stay active. Its separate
`gradient_admission_test.go` witness uses the cylinder-grid board with an
authored dashed container and malformed/valid gradient stops, through SVG,
PNG and RenderPage (9 calls). The six prior failures include three successful
images silently missing the invalid stop and three incorrect late errors.
The permanent `tests/fixtures/paint-api.json` binds original requests and
complete expected byte hashes/counts, exact errors and page annotations;
`tests/paint_api.rs` checks the public native result and source immutability.
This finite public-call coverage does not qualify all internal paint APIs.

## Lane I12: caller assets on legend and root-frame panels

`source_panels.rs::surface_document_with_assets` translates
`surface_document.go:64–111` and `surface_fonts.go:19–54`: validate every
root/shape/connection/legend icon before creating the embedded-only default
resolver, then pass the explicitly supplied resolver and tightened import
limits/budgets into the shared scene builder. `SurfaceIconPainter::surface_assets`
reuses `surface_document.go:46–58`'s construction and the existing
`surface_icons.go:60–133` limit clamp. `root_decorations` passes the render's
caller assets to both panels, following `source_panels.go:128–145,210–223`.
No foundation implementation, mesh geometry or raster algorithm changes.

The public `panels` API suite uses the original `txtar/legend-mono` Dagre,
ELK and TALA board inputs with one authored legend icon: embedded SVG,
external SVG without a resolver, or embedded SVG with MaxElements=1. All
27 whole-image SVG/PNG/RenderPage calls failed before the handoff fix. Go's
complete bytes, annotations and errors are captured with assertions active
by `panel_assets_test.go`; these separate board witnesses are source matches,
not extra golden matches. `tests/panel_assets_api.rs` binds their complete
requests and byte hashes, checks source immutability, and independently
verifies that a supplied resolver reaches the legend while caller limits
remain enforced. The replay adapter preserves every serialized import limit
and budget, rejecting opaque resolver objects rather than silently ignoring
them. Original public-call suites retain their existing inventories.

## Configured-family admission and source composition (Lane I, iso13)

`surface_document.go:30–49` resolves `DEFAULT` and `mono` before
`text.go:103–121` selects regular/bold/italic and looks up that concrete face.
`text.rs::TextPainter::face` now shares the existing role resolver, preserving
Go's distinction between an unknown explicit family and an unavailable
configured face (`isometric text font <family>/<style> is not loaded`).
No font discovery or substitution is introduced. Caller-registered custom
font bytes still need the shared foundation registration API.

`tools/isometric-probe/font_admission_test.go` calls SVG/PNG/RenderPage on the
original unicode/with-style and txtar/legend-mono boards (all three engines),
with an unavailable caller family and unchanged source font roles. Complete
errors compare; failed calls must publish no image/page. The composition
hook additionally observes SVG/PNG/RenderPage on all five original helper
inputs in `source_panels_test.go:64–167`, retaining the original tests and
assertions. Complete bytes and annotations compare; these public-boundary
witnesses do not add upstream goldens or qualify animated frames.

## Rich-content whole-board API observations (Lane I, iso15)

`tools/isometric-probe/rich_boards_test.go` observes the exact owned targets
constructed by `rich_labels.go:222–350` from six original tests in
`rich_labels_test.go` and `surface_fidelity_test.go`: long Markdown, light/dark
code, class/SQL content, opacity admission and LaTeX. It delegates the original
scene Build unchanged, then sends each successful outer target through public
SVG, PNG and RenderPage. A context marker prevents recursive observations;
every recursive renderer call still runs the original source. Original test
assertions remain active, and trace edits are restored after capture.

`--tier api --suite rich-boards` compares all 30 complete images/pages against
the frozen native API runner, with exact bytes, annotations and inventories.
These constructed whole-board observations add no golden claims and do not
qualify all helper assertions. No production renderer change was required.
# GIF image API (run63)

`img/gif.rs` translates pinned master `6a7b48378`
`d2renderers/d2isometric/d2isometricimg/gif.go:20–190`: four-phase shared
median-cut palette, 18-bit immutable RGB lookup, opaque frame deltas,
cumulative centisecond delays, infinite looping and bounded output.
The existing native go-quantize translation in d2-rs-export is shared through
`gif_median_cut_palette`; palette/index behavior follows Go image/color.
`img/gif_loop.rs` follows `gif_loop.go:16–87`, privately cloning panel/mask
tracks and fitting repeating durations to the chosen loop. Owned Rust scene
nodes cannot contain Go's pointer cycles; depth/node/track budgets remain.
`render::render_with_context` dispatches the public Render formats after
the existing admission checks (`render.go:99–114`).

Whole-image witnesses: original `TestNativeGIFWithoutTraffic`, plus traffic,
bidirectional traffic, authored-node, static-route and animated legend boards
from `render_test.go:128–153`, `gif_loop_test.go:18–113`. Captures preserve
original Go assertions and all admission errors. GIF pixel/timing comparison
reuses the existing CLI gate; native tests additionally bind every source
palette, delta rectangle and composited frame hash. Timeline and CLI animated
isometric output are separate outstanding paths.

`img/timeline.rs` translates pinned master `d2isometricimg/timeline.go:20–190`
and `framing.go:253–270`: bounded ordered-board GIFs use a union camera,
shared four-phase palette, original scene/packet time and cumulative delays.
Capture sessions share the outer cancellation/deadline. The timeline API
keeps source diagrams and animation timing unchanged. The env-gated
`tools/isometric-timeline-probe` hook retains original Go assertions; native
`tests/timeline_api.rs` binds every palette, delta rectangle, frame and delay
from all twelve captured calls (three complete GIFs / ten frames).

Run 65's CLI folder/held-leaf witnesses expose the one-frame timeline loop
contract: `timeline.go:93,126` delegates to Go 1.27 `image/gif/writer.go:173`,
which writes NETSCAPE2.0 only when there is more than one frame. The native
timeline encoder now follows that condition; the regression test also checks
the single frame's authored hold delay and complete frame count.

Lane I24 translates `scene.go:344–399` (`budget.check`) into the native
`scene/admission.rs` scalar visitor. It checks floats before any JSON value
conversion can replace NaN or infinity with null; strings and collection
lengths share the source admission budgets. Render options remain a struct,
so their fields do not invent collection entries. Original scene validation
rejects both infinite route coordinates and NaN content aspect ratios, with
the complete source error retained. The observer stores a valid diagram before
applying each original typed mutation. Nil references and invalid UTF-8 are
explicitly unsupported at the safe native representation boundary.

## Lane I25: source visibility and cubic-route contracts

At master `6a7b4837838a3907604a1d9eec57ee98b6f3cd01`,
`scripts/isometric-visibility-scoreboard.py` captures all eight original
functions in `d2isometricimg/svg_visibility_test.go` (27 calls). Its hook keeps
every original assertion and records complete ordered fragments, errors,
typed nonfinite input scalars, cancellation, work allowances, and group/owner
equality. The Rust adapter calls `svg_visibility.rs` directly. All calls
already match; no visibility algorithm change was needed.

`scripts/isometric-cubic-scoreboard.py` observes both original public calls
in `hierarchy_test.go:219–263` against `hierarchy.go:139–204`. The complete
scene and malformed-control error already match. The private shared-point
budget assertion is retained in Go and separately tested in native
`scene.rs::cubic_budget_tests`; it is not claimed as a cross-language call
observation. No allocation-address parity or new golden-board gain is claimed.

## Lane I26: atomic SVG paint order

At the same master pin, `scripts/isometric-order-scoreboard.py` observes all
six original `d2isometricimg/svg_order_test.go:25–143` functions (12 calls)
against `svg_order.go:18–197`, using `img/svg_order.rs` directly. The hook
keeps every original assertion. Ordered unit/batch identities, authored
opacity/depth/source indices, polygon coordinates, errors, cancellation and
consumed ordering statistics all match. The 1,200-fragment spatial-index test
retains its explicit 400,000-work allowance. Rust's immutable ordering view
borrows its geometry and returns a permutation; caller-owned batch data stays
outside the algorithm's mutation surface.

Go allocation addresses, unused batch rendering fields and the statistics
discarded by original error-only calls remain unqualified. No algorithm fix
or new whole-board gain is claimed. The harness tests reject missing/extra
calls and changed order, opacity, batch identity, coordinates, stats or errors.

### Lane L29 — low-level raster frame observations (master 6a7b48378)

`img/raster.rs::render_mesh_with_dynamic` exposes the existing native frame
renderer with static-mesh camera fitting, dynamic depth tests and explicit
cancellation. It returns independently owned pixels. Source mapping:
`d2isometricimg/raster.go:148–257` (`NewRaster`/`newRaster`) and `260–308`
(`Frame`). This complete-render entry point recomputes preparation each call;
it does not implement Go's retained Raster cache or its parallel-use contract.

`tools/isometric-raster-api-scoreboard/hook_test.go` observes all 28 original
NewRaster/Frame calls in seven functions from `raster_test.go` and
`raster_multiply_test.go`, retaining every original Go assertion. The native
example stores each captured static mesh and calls the native renderer for
each operation. Complete ordered dimensions, stride, premultiplied RGBA bytes
and errors compare without normalization. Five static functions already match;
the new entry point closes the two unsupported dynamic-overlay functions.
The private scalar sampler assertion remains a Go assertion, separately from
these cross-language frame observations. Arbitrary material mutation, malformed
polymorphic Go image storage, prepared-cache performance and concurrent Frame
calls remain unqualified. Reproduce with
`scripts/isometric-raster-api-scoreboard.py`; pinned capture, negative/final
reports and source bindings are in `.reference/codex-runs/lang29-*`.

## Lane I27: SVG fragment retention and precision contracts

At master `6a7b48378`, `scripts/isometric-precision-scoreboard.py` observes six
original functions in `svg_fragments_test.go`, `svg_precision_test.go` and
`svg_polygon_precision_test.go` (36 operations), preserving every original Go
assertion. Static mapping: `svg_fragments.go:33–114` → the private
`svg_visibility.rs::FragmentSet`; `svg_visibility.go:85–105,209–262,290–295`
→ depth/area/split/edge predicates; `svg.go:119–218` →
`svg.rs::svg_polygon_path` and collinear-split omission. No algorithm changes
were needed. Complete live index/cell state, reference release, rejected-add
errors, work counts, exact predicate float bits, split coordinates, path bytes
and unchanged source polygons match. Map enumeration is sorted by integer ID;
allocation addresses remain unqualified. Replay lives entirely behind
`cfg(test)` in `svg_precision_tests.rs`, without adding public renderer APIs.
The retained fixture is a direct copy of the original Go observations;
`scripts/test_isometric_precision_scoreboard.py` checks failure gates.

### Lane L30 — surface painter admission and original test batch

`surface_icons.rs::SurfaceIconPainter::new_with_context` follows master
`d2isometricimg/surface_icons.go:59–66`: return cancellation before count
admission or resolver allocation. The existing `new` entry is a background
context convenience; `render.rs` now passes its explicit context at admission.
`TestSurfaceIconCancellationAndBudgets` exposes the missing entry: a cancelled
constructor previously succeeded. No paint, allocation or raster algorithm
changes are involved.

`scripts/isometric-surface-scoreboard.py` captures 24 original functions from
`text_test.go`, `rich_labels_test.go`, `surface_fidelity_test.go`,
`surface_icons_test.go`, `output_density_test.go` and `vector_surface_test.go`.
All original Go assertions remain active. Its hook temporarily wraps the
original constructor/texture functions only in the manifest's trace checkout;
it observes direct sizing, viewport fitting and icon layout calls as well.
The 2,910 ordered observations compare complete premultiplied RGBA buffers,
layouts, optional vector SVG, inline-link metadata, errors and texture reuse
identities. Source mappings: `text.go:75–88,145–405,412–447`,
`rich_labels.go:43–59,79–162,355–379`,
`surface_icons.go:59–202,204–235,306–431`, and
`surface_document.go:149–215`. Native replay/private budget setters compile
only in library tests (`text_api_tests.rs`, `surface_icons_api_tests.rs`);
no captured expected output is compiled into the implementation.

The batch improves **23/24 → 24/24 functions**, zero regressions. The private
Go cache-size assertions and vector-fragment reimport/pixel tolerances remain
original Go assertions; named observations do not independently prove all
those internals or the complete APIs. See `.reference/codex-runs/report-lang30.md`
for results, reproduction commands, retention and storage measurements.

### Lane I28 — crossing-route context and header contracts (2026-10-25)

Pinned master `6a7b4837838a3907604a1d9eec57ee98b6f3cd01`:
`d2renderers/d2isometric/d2isometricimg/route_bridges.go:36–85` maps to
`img/route_bridges.rs::{Work,native_resolve_bridge_routes}`. Every bridge
work debit checks the caller's context before its remaining allowance;
`geometry_routes.go:342` maps to `img/routes.rs`, which now passes the mesh
builder's context into that stage. Default-context wrappers preserve existing
native callers. Lane I does not claim cancellation inside the preceding lane
resolver, which remains a separate contract.

`tools/isometric-geometry-scoreboard/hook_test.go` observes six original
`hierarchy_headers_test.go` functions and six `route_bridges_test.go`
functions, retaining all original assertions. `hierarchy_headers.go:17–267`
maps to the existing `img/hierarchy_headers.rs`; header placement already
matches. Twelve functions / 43 operations compare complete ordered paths,
placements, error text and supplied source records, including nonfinite scalar
tags and opacity/dash inputs. Test-only replay lives in
`img/geometry_api_tests.rs`, with `tests/fixtures/geometry-api.json` as the
captured input/expected-output boundary. No production diagnostic JSON is
constructed. Allocation addresses and Go error-interface identity are not
qualified by this observer. Original Go test files are restored after capture.
The context-admission function changes from unsupported to matching; the other
11 functions preserve matches. Native tests additionally check cancellation
after bridge work starts and context propagation through the edge pipeline.


## Original native content/retained-surface contracts (Lane L31, master)

`isometric-content-scoreboard.py` captures 20 original functions / 83 operations
from the pinned d2isometricimg readability, presentation_quality,
surface_content, output_density, surface_fidelity, vector_surface,
vector_surface_mask, vector_surface_svg_precision and vector_surface_cap tests.
All original assertions/subtests execute. The observer is installed only in the
trace checkout and restored in `finally`; its scene encoding derives from
`tools/scene-probe/scenejson.go` with isolated helper names.

Mapping: `route_style.go:12–76`, `geometry_routes.go:15–68` and
`geometry.go` → route tints/sections, tube/casing/ink and native node geometry;
`lighting.go:25–53` / `raster.go:846–865` → `mesh.rs` shading;
`native_shapes.go:104–157` → native face density and split-opacity compensation;
`surface_document.go:65–156` → source document admission and bounded exact-time
planar rasterization; `vector_surface.go:32–45,134–159` → solid caps and
apertures; `vector_surface_svg.go:38–110,111–125` → retained SVG and admission.
Requests carry actual original source values. Full ordered vertices, material
identities/fields, texture pixels, source documents, SVG fragments, float
values and complete errors compare without tolerances or output normalization.
The 14 small complete functions / 67 operations are retained in
`tests/fixtures/content-api.json`; the six larger functions retain their full
capture under `.reference/codex-runs/lang31-content-capture-qualified`.

Found and fixed `TestNativeVectorCaptureIsScopedAndCancelable`: the native
surface SVG writer ignored the caller's cancellation. Added
`native_surface_svg_with_context`, kept the background-context convenience,
passed the export context at `render_svg_with_context`, and checked its first
cause in every writer admission, including recursive ink-coverage surfaces.
This follows `vector_surface_svg.go:111–119`; invalid-prefix/viewport precedence
is preserved. The initial 78 operations give 77 → 78 exact matches; five newly
captured direct aperture/opacity mutations also match. Functions: 19/20 → 20/20.

All JSON replay/adapters compile only in tests. Unobserved Go registry-isolation
and allocation internals, private fragment-reimport tolerances and packet
internals remain original Go assertions, not separately proven native checks.
No complete API or new whole-board-golden gain is inferred from named coverage.
Reproduce the original capture with the combined scoreboard; `--reuse` verifies
source/assertions/inventories/hashes before native replay into fresh outputs.
The harness requires nonempty complete inventories and premultiplied buffers,
and enforces 1 GiB total / 128 MiB per file / 20 GiB free plus finite deadlines.


## Shared isometric SVG document wrapper / WASM clock (run71)

`src/document.rs::wrap_svg` is the existing CLI source translation of pinned
`d2renderers/d2svg/isometric.go:99–218`, shared with the native WASM target
boundary. It preserves resource namespacing, salted hashes, padding, scaling,
document metadata and output limits. CLI source contracts remain 50/50.
`img/context.rs` and `img/raster.rs` select web-time 1.1.0's Instant only on
wasm32, retaining std::time::Instant on native targets. The WASM target checks;
rebuilt browser/Node package execution remains separate. Original WASM
compile/render observations and all regression evidence are in report-run71.md.

### Lane I29: cooperative route-lane admission (2026-10-25)

`route_lanes.rs::native_resolve_lane_routes` follows `route_lanes.go:43–75`:
require a context, admit scene sizes, and check the caller's cancellation
before every work debit. The explicit limit reaches all existing work sites;
`routes.rs` supplies the mesh builder's context. `route_bridges.rs::Work`
shares this mechanism while preserving each stage's error wording. The
background-context convenience remains. No route geometry or FMA changes.

Eight original `route_lanes_test.go` functions / 28 calls retain every Go
assertion. Test-only replay compares full ordered paths, packet lengths,
errors and supplied edge/node/board records, including nonfinite coordinates
and Go nil-slice boundaries. Admission improves from 25/28 to 28/28 calls;
seven matching functions become eight, with zero regressions. Native tests
also check cancellation after a successful debit and cancellation precedence
through the edge pipeline. Allocation addresses and Go error-interface identity
remain unqualified. Evidence and rebuild commands: `.reference/codex-runs/`
`report-iso29.md`, `iso29-deltas.json`, and `iso29-verify.py`.

### Lane I30 — endpoint visibility cancellation (2026-10-25)

Pinned `6a7b4837838a3907604a1d9eec57ee98b6f3cd01`,
`d2renderers/d2isometric/d2isometricimg/arrow_visibility.go:35–47,75,114`:
`MeshBuilder::arrow_budget` checks the caller context before debiting work,
including cached queries, and the marker/cap helper builders inherit it.
No coordinate or contraction changes. Original
`TestArrowVisibilityBoundsCachesWorkAndCancellation` exposed the missing check.

`tools/isometric-arrow-scoreboard/hook_test.go` observes eight original
functions with all assertions active. `scripts/isometric-arrow-scoreboard.py`
replays 51 input-only queries through a cfg(test) adapter: complete route/source
coordinates, errors, work debits, cache counts and receiver-cache dimensions
compare exactly. Query inputs retain marker vertices, receiver polygons, owner
records and the triangle/material fields actually consumed by visibility;
other material fields and Go allocation identities are outside this projection.
Original overlap/pipeline and direct cache-cap assertions remain Go assertions,
beyond these query observations. The native cache/budget fixture preserves all four original calls;
an additional native test checks cancellation after a debit, precedence over
budget exhaustion and preservation of an earlier failure. Evidence and final
board deltas: `.reference/codex-runs/report-iso30.md`.

## Run 72 — direct SVG document API

`src/svg.rs` ports the mode-dispatch boundary from pinned master
`d2renderers/d2svg/d2svg.go:2866–2909,3575–3628` and
`d2renderers/d2svg/isometric.go:21–96`. It reuses `document::wrap_svg`
(the existing translation of `isometric.go:99–218`) and the native mesh
renderer. The facade lives in this crate because the mesh renderer already
uses the lower-level d2-rs-render geometry; reversing that dependency would
create a cycle. The flat-only primitive and exact document options are
public in d2-rs-render. Native context, asset resolver, unscaled dimensions,
absent versus empty override pointers and integer padding remain explicit.
The WASM isometric boundary now calls the same facade and retains its own
request admission and two render-error wrappers.

`tools/svg-isometric-scoreboard/hook_test.go` observes four original test
functions, with all original assertions active. The fixture retains all
13 complete requests and responses, including IEEE scale bits for Inf,
ordered multiboard arrays and complete error text. The negative comparator
uses the pre-existing flat renderer (3/13), not a claimed pre-existing
RenderIsometric implementation; the native facade matches 13/13. This
finite corpus does not qualify the remaining raw-target validation function
or every native option. Rebuild and test:

```sh
cargo build --profile diagnostic -p d2-rs-isometric --example svg-isometric-scoreboard
cargo test --profile diagnostic -p d2-rs-isometric --example svg-isometric-scoreboard
uv run --no-project python scripts/svg-isometric-scoreboard.py --capture <fresh> --rust target/diagnostic/examples/svg-isometric-scoreboard --output <report.json>
```

### Lane L32 — raw raster storage and deferred snapshots (2026-10-25)

`img/mesh.rs` admits caller-owned RGBA, NRGBA and Alpha row storage with an
explicit stride. `Texture::validate_storage` follows master's
`d2renderers/d2isometric/d2isometricimg/raster.go:557–569`: compare the stride
against divided available bytes, avoiding overflowing row-count products and
admitting a valid single row even with an enormous stride. Packed RGBA painters
retain their existing representation. `raster::texture_sample` follows
`raster.go:1069–1107` and Go `image/color.NRGBA.RGBA`: preserve 16-bit integer
premultiplication before interpolation rather than round-trip through RGBA8.
`raster::snapshot_triangles` follows `raster.go:307–393`, preserving sharing
inside an owned mesh while copying raster bytes, materials, groups and owners;
public deferred renders use this snapshot (`raster.go:158–171`). Source-vector
references remain shared exactly as in the material copy upstream.

Test-only `img/raster_api_tests.rs`, `tests/fixtures/raster-api.json`,
`tools/isometric-raster-scoreboard/hook_test.go` and
`scripts/isometric-raster-scoreboard.py` replay ten original functions with
all Go assertions active. They compare 15,153 observed operations, including
complete raw image inventories and exact bytes/hashes, 15,113 scanline spans,
texture samples and snapshots. Three Go typed-nil calls remain explicitly
unsupported; that admission function is not credited as covered. Go's private
cache layout, forced-strip policies and unobserved capture-camera internals
remain Go assertions, not native API claims. The two-function fixture covers
12 complete precision/compositor observations; native ownership/overflow and
cancellation tests independently exercise shared mutable inputs. JSON replay
and diagnostic request construction compile only under `cfg(test)`.

### Lane I31: outline and solid-ink cancellation (2026-10-25)

Pinned master `6a7b48378`, `native_edges.go:102–126,270–279,305–312,471`:
`classic_ink_segments_with_context` preserves the input-size admission and
polls the caller every 1,024 triangles, edges and support-adjacency visits.
`MeshBuilder` passes its render context; the background convenience entry
remains available. `native_solids.go:115–118,133–139`: `solid_ink_fit` checks
that same context every 64 rows, records its error and returns the source's
factor of one. Nil/empty textures retain their original admission behavior.
No geometry, material, sorting or floating-point expression changes.

`scripts/isometric-ink-scoreboard.py` observes the two original cancellation
functions with all Go assertions active. Test-only replay compares complete
errors, segment count and fit factor on input-only requests. It preserves all
consumed triangle fields and raw texture bytes; the original 100,001 zero
triangles are represented by their exact count after checking every Go member
is zero. Successful segment/support geometry and asynchronous cancellation
timing are outside this admission projection; whole-board SVG/PNG guards
separately check ordinary rendering. Compact fixture: `tests/fixtures/ink-api.json`.
Evidence and staged negative witnesses: `.reference/codex-runs/report-iso31.md`.

### Lane L33: animation and shadow raster-call observations (2026-10-25)

`isometric-animation-raster-scoreboard.py` observes the original calls to
`newRaster` / `Raster.Frame` (`raster.go:165–260`) in seven master-added
animation, GIF-loop, decal and shadow-framing functions. All 250 operations
match complete RGBA bytes, dimensions, strides and errors. Request-only replay
keeps supplied geometry, raw texture bytes and view/shadow/ground cameras;
the test transport now admits file-backed texture bytes and the explicit ground
camera. Both changes compile only in `cfg(test)`. No renderer algorithm changes.

Caller motion, loop adaptation and camera/framing construction remain original
Go assertions in this tier. It does not independently qualify those callers,
GIF encoding or new goldens. The source observers preserve every original
assertion. Streaming 1 GiB total / 128 MiB file / 20 GiB free limits apply.
The first repeated-texture capture exceeded its per-file limit and is excluded;
exact texture-byte references reduce the qualified capture to 82 MiB total,
49 MiB largest file. Evidence and remaining scope: `report-lang33.md`.

## Lane I32: route-caption and physical-outline observations

At master `6a7b4837838a3907604a1d9eec57ee98b6f3cd01`, the test-only
`img/caption_api_tests.rs` replay observes 20 original functions / 74 calls.
The inventory is `scripts/isometric-caption-scoreboard.py`; its Go hook lives
in `tools/isometric-caption-scoreboard/hook_test.go`. Original assertions stay
active. Source mapping: `route_labels.go:58–316` (placement/state/budgets),
`route_caption_style.go:33–194` (metrics/anchors), `geometry_routes.go:59–105`
(dashes/rounding), `native_edges.go:68–84,102–373,594–705` (support depth,
ordered outline segments/facets and cyclic support clipping). All observed
calls already match; this adds no production algorithm or FMA change.

Consecutive identical obstacle rectangles use exact run-length encoding;
all supplied fields, obstacle order, work/reference counts and every returned
surface/path/facet compare by exact JSON numeric values. Outline extraction
consumes material alpha and texture presence, so its transport retains those
without copying unused image bytes. Face clipping retains material fields and
checks owner/identity invariants. Go grid-map layout, pointer addresses,
allocation behavior, unobserved painters and original raster assertions are
outside this replay. The source assertion suite remains the Go-side guard.

## Lane L34: GIF output and mask clone state

At master `6a7b4837838a3907604a1d9eec57ee98b6f3cd01`,
`img/output_api_tests.rs` observes the original palette/delta, writer-limit,
animated-mask clone and non-casting shadow-framing tests. Its transport and
JSON construction compile only in tests. Source: `gif.go:105–189` (entire
RGB lookup, indexed frames, delta rectangles/palettes/pixels, delays and
bounded writer state), `gif_loop.go:39–68` (owned animation/mask trees),
and `framing.go:66–131` (projected ground-shadow extents).

The clone now allocates empty child lists exactly as `gif_loop.go:49`, for
both the root and animated masks, preserving the caller's absent lists.
Original Go assertions stay active. Full documents and independent ownership
compare; pointer addresses and cyclic Go pointer graphs are outside this
owned-node replay. The palette's complete 262,144 lookup bytes compare; the
three original RGBA indexing inputs are opaque and explicitly checked. This
finite writer corpus uses bytes.Buffer, not arbitrary writer callbacks.
No encoded-GIF byte identity or full API coverage is inferred.

### Lane I33: solid contacts and projected clearance (2026-10-25)

Pinned master `6a7b4837838a3907604a1d9eec57ee98b6f3cd01`, package
`d2renderers/d2isometric/d2isometricimg`: `solid_contacts.go:79–137`
checks cancellation before each output route. `img/native_solids.rs` now
accepts the caller's `RenderContext` at that stage; `img/routes.rs` carries
it through the final contact pass. The context-free convenience wrapper
retains background-context behavior. Geometry calculations are unchanged.

`tools/isometric-contact-scoreboard/hook_test.go` observes 20 original Test
functions / 223 calls without removing their Go assertions. Input-only
`cfg(test)` replay: `img/contact_api_tests.rs`, included by `route_labels.rs`.
The repository corpus is `tests/fixtures/contact-api.json`; captures,
negative/final outputs and the reversible implementation patch are under
`.reference/codex-runs/iso33/`. Cancellation was the only differing call:
222/223 before, 223/223 after. This is named native observation, not proof
of unobserved caller, allocation, pointer-address or raster assertions.

Other source mapping: `solid_contacts.go:22–277` (actual solid contacts and
print crowns), `projection_clearance.go:24–180` and `route_labels.go:42–316`
(projected obstacles/placement/work), `route_style.go:29–78` and
`hierarchy_boards.go:241–262` (casing), `native_shapes.go:66–126,510–735`
(source paint/profile/sidewall normals), and `native_style.go:10–20`
(classic palette). Full numeric geometry, ordered paths/obstacles, generated
vertices/materials and complete errors compare without tolerance. Projection
inputs preserve shared-material identity, alpha and texture presence without
unused texture bytes; nonfinite input/returned points retain classification.

## Shadow alpha and retained mask/gradient observations (Lane L35)

`tools/isometric-fidelity-scoreboard` observes three original master tests
from `surface_depth_test.go`, `vector_surface_mask_test.go` and
`vector_surface_test.go`, retaining all Go assertions. Eight calls already
match: `raster.go:478–492,691–800` admission/shadow building,
`vector_surface_svg.go:183–201` typed masks and
`vector_surface_color.go:26–105` source-in masks and linear-light gradients.
The entire shadow float32-depth and opacity buffers, camera, bias, work
budget, complete errors and SVG/writer state compare without tolerance.
NaN remains an explicit input value; test-only raster decoding now preserves
it in optional shadow opacity. No renderer algorithm changes were required.

The input-only replay is `img/fidelity_api_tests.rs`, with test-only bridges
in raster and color painting. Its compact fixture stores full SVG/error
expectations and SHA-256/lengths for the complete 20 MiB shadow buffers;
the external scoreboard compares their complete bytes. The original
font-free COLRv1 plans use no glyph lookup. The owned Rust writer supplies
an unused bundled face to its nonoptional face argument; font lookup,
Go allocation/pointer identity and arbitrary paint plans are unqualified.

### Lane I34: solid caps and caption knockout rounding (2026-10-25)

Pinned D2 master 6a7b48378, Go 1.27 ARM64. Original tests from
`native_solids_test.go`, `native_symbol_relief_test.go`, `native_page_test.go`
and `route_caption_mask_test.go` retain all assertions under the test-only
observer in `tools/isometric-solid-scoreboard/hook_test.go`. Twenty functions
produce 260 calls. Native input-only replay compares full ordered vertices,
normals, UVs, triangle metadata, shared material/group/owner identities,
material fields, complete premultiplied RGBA texture hashes, source values
and builder error/face-budget state. Vector document bodies and uncaptured
caller/raster assertions are outside this projection.

Two earliest divergences are corrected: `native_solids.go:130` rounds
`ny*y`/`ny*ay`, then fuses `nx*x`/`nx*ax`; `route_caption_mask.go:39` rounds
`dz*sin`, then fuses `dx*cos`. Compiler register/instruction excerpts and
rebuild command are retained in `.reference/codex-runs/iso34/asm/`.
`native_solids.rs` and `route_caption_mask.rs` now follow these operations.
The original failures are TestNativeSolidsKeepGeometryAcrossPaintStyles,
TestRouteCaptionKnockoutPreservesProjectedDashesAndAlpha and
TestConnectionCaptionGapRevealsUnderlyingSurface: 256/260 -> 260/260,
17/20 -> 20/20, zero regressions. Compact native fixtures retain the four
failing calls and metadata ownership witness. No diagnostics are added
outside `cfg(test)`. Full reports: `iso34/solid-{negative,final}.json`.

### Lane L36: configured and explicit surface fonts (master 6a7b48378)

`text.go:103–151` reads named `d2fonts.FontFaces` entries, including caller
registrations; `surface_document.go:30–49` admits exact registered family
names after its built-in aliases. `img/text.rs` and `img/surface_document.rs`
now use the shared owned registry lookup. This supersedes the historical
bundled-only plain-face limitation above. Rich documents use the same named
registrations through d2scenebuild (`rich_labels.go:222–247`).

`tools/isometric-font-scoreboard` observes the original
`TestNativePlainAndRichUseConfiguredAndExplicitFontFamilies` and
`TestNativeTextUsesExplicitFallbackResolver`, preserving their assertions.
Six calls compare complete RGBA buffers, layouts, face-cache keys, rich scene
JSON/font bytes, errors, fallback requests and resolver state. The standalone
`font-api.json` fixture deduplicates ten exact font inputs and binds every
output texture/font byte by length/SHA-256. All transport and fixture code
compiles only in tests. This finite corpus does not qualify arbitrary
registration mutation during a render or every font format.

### Lane I35: original source-shape/sequence calls and cancellation

`native_shapes.rs::native_face_layer` now checks its RenderContext before the
face-document adapter. Upstream `d2isometricimg/native_shapes.go:191–199`
passes that context into `d2scenebuild.Build`; canceled calls return
`native shape face: context canceled`, no texture, zero area and no pixel spend.
The former adapter built and rendered the face with an uncanceled context.

`scripts/isometric-shape-scoreboard.py` observes 20 original functions / 187
calls from native_shapes, native_face_ink, native_page_hinge, geometry and
sequence_geometry tests. Original Go assertions remain active. The new
`shape_api_tests.rs` bridge and shared solid replay helpers compile only in
tests. Ordered mesh/material identity, source contours/label areas, texture
pixel hashes, complete errors and budgets compare exactly; private animation
ranges are explicitly serialized. Uncaptured raster assertions and retained
vector document bodies remain outside the observation projection. Evidence:
`.reference/codex-runs/iso35/`, including the complete cancellation negative
witness, original expected values and source-mapped six-line fix.

### Lane I36: original structured-node and sequence-route observations

Pinned master `6a7b48378`, `d2isometricimg/native_structured_test.go` (six
functions), `sequence_routes_test.go` (six), and
`svg_precision_test.go:198–224` (`TestStructuredRailRoundsSourceContourTransform`)
are observed without removing any original assertions. The test-only
`structured_api_tests.rs` bridge and `sequence_routes::caption_api_value`
replay 50 input-only calls from 13 functions against native
`native_structured.go`/`sequence_routes.go`/`geometry_routes.go` translations.
Ordered mesh geometry, material/group/owner identities, every texture pixel
hash, rich budgets, source nodes/edges, complete text runs and viewports,
routes/errors, captions, packet materials/tracks and placer state compare
exactly. Concrete text paint values are projected explicitly from Rust's
closed enum into Go's interface JSON fields, preserving all fields; unsupported
pattern/stroked text is rejected by this test bridge. Full document trees and
uncaptured caller/raster assertions remain outside this projection. All bridges
compile only under `cfg(test)`; no production algorithm change was needed.
Evidence: `scripts/isometric-structured-scoreboard.py`, its gate tests,
`tools/isometric-structured-scoreboard/hook_test.go`, and
`.reference/codex-runs/iso36/`. The warm shared diagnostic library test
executable replays both external requests and the retained complete fixture.

### Lane L37: source-panel reservations and caption/artwork observations

Pinned master `6a7b48378`, `d2isometricimg/source_panels.go:69–110`,
`source_panel_density_test.go`, `surface_fidelity_test.go:326–399` and
`presentation_caption_test.go:13–123`. Three original test functions retain
every Go assertion while `scripts/isometric-panel-scoreboard.py` observes
eleven source-panel, hierarchy-node and node calls. The test-only
`source_panels::panel_api_tests` bridge replays their input documents, source
nodes, builder state and shared painter constructor counts. Complete ordered
mesh/material/group/owner identities, every texture pixel hash, source-panel
documents/ranges, errors and pixel budgets match. The caller's independent
camera and post-call negative-control deformation assertions are outside this
call projection. No production algorithm changed; no extra golden is claimed.
Initial one-label replay construction was corrected to preserve the original
two-label painter and its sharing, without changing original expectations.
Evidence: `.reference/codex-runs/lang37/panel-sessions`, `panel-final.json`,
`observer-setup.json`; retained fixture `tests/fixtures/panel-api.json`.

### Lane I37 — original hierarchy and SVG lighting contracts

At master `6a7b4837838a3907604a1d9eec57ee98b6f3cd01`,
`img/hierarchy_api_tests.rs` replays 358 input-only calls from 21 original
functions in `hierarchy_{boards,solid,support,terraces}_test.go` and
`svg_lighting_test.go`. Production mappings are `hierarchy_boards.go`,
`hierarchy_support.go`, `hierarchy_headers.go`, `native_shapes.go`,
`svg_lighting.go` and `svg.go`. All original Go assertions remain active.
`tools/isometric-hierarchy-scoreboard/hook_test.go` installs only into the
pinned trace checkout; the Rust bridge is included only under `cfg(test)`.
It compares full mesh/material/group/owner values, texture pixel hashes,
support/budget state, hierarchy plans, lighting coefficients/gradient patches,
and complete shadow SVG bytes. The typed bridge preserves the input nil
node-ID marker for unchanged empty Rust Vecs; nonempty native values retain
their complete contents. Full retained vector document trees/assets and
uncaptured caller/raster assertions remain outside this projection. All 21
functions already match; no runtime algorithm fix or new golden gain.

### Lane L38: explicit icon resolver policy and vector capture

Pinned master `6a7b48378`, `d2isometricimg/surface_icons.go:59–90`,
`surface_icons_test.go:70–130`, `geometry.go:431–478`, and
`vector_surface_test.go:337–379`. The default painter's imageasset resolver
must preserve the zero-value denied local-file policy. Rust previously used
unrestricted files, so explicitly sharing that otherwise private resolver
admitted a local icon that Go rejects. The default now uses
`ImageLocalFiles::Denied`; caller-supplied rooted resolvers still render the
same complete raster texture. The negative witness retains the actual local
PNG bytes and complete call sequence, including the erroneous successful read.

`render::build_native_scene_with_options` factors the existing mesh assembly
so private callers can retain their explicit density/framing settings,
including legacy density zero. Ordinary render/capture callers still derive
their options through the existing wrapper. No duplicated mesh algorithm or
fixture-conditioned behavior is introduced.

`scripts/isometric-artwork-scoreboard.py` preserves every original assertion
in two original tests while observing twelve calls. The cfg(test)-only
`surface_icons::artwork_api_tests` bridge compares complete scene/source values,
ordered mesh geometry, material/group/owner/vector sharing, every texture's
pixel hash and all retained SVG fragments across four physical/structured
boards, plus actual local-file admission and exact errors. Caller file bytes
and resolver limits are inputs; expected values never enter replay requests.
Internal resolver allocation counts and unobserved caller assertions remain
outside the projection. Fixture: `tests/fixtures/artwork-api.json`; evidence:
`.reference/codex-runs/lang38/`, including `negative-icon-witness.json` and
`runtime-fix.patch`. This qualifies the captured contracts, not full API parity.


### Lane I38: original SVG coverage and independent receiver observations

Pinned master `6a7b48378`: `ground_receivers.go:9,71`, `raster.go`
receiver validation/snapshot, `svg_seams.go:19,57,120`, `svg.go:237,477`,
`svg_surface_resources.go:8`, `svg_visibility.go`, and `markdown_cards.go`.
`scripts/isometric-coverage-scoreboard.py` observes 20 original functions
across eight test files, keeping every original assertion active. All
1,053 observations already match; no production algorithm change.

The new `svg::coverage_api_tests` bridge, module inclusion and direct raster
validator adapter compile only under `cfg(test)`. Replay receives only inputs
and compares complete ordered meshes, material/group/owner sharing, receiver
values (including original NaN/Inf admission), texture pixel hashes, cover
grouping, visibility fragments, projected coordinates, exact errors and full
SVG bytes. Retained vector document trees are inputs to the native SVG writer;
the shared scene serializer retains their complete paths and assets. The
1,002 resource-definition calls preserve original writer/surface sharing.

Go slice-allocation/nil-slice and receiver-pointer identities are outside this
projection: native receiver planes are owned scalar values and meshes are
owned Vecs. Caller raster assertions remain active in Go; the shadow observer
compares SVG bytes before the caller's filter removal/rasterization. Those
uncaptured assertions and unused internal caches are not claimed as native
API parity. Fixture: `tests/fixtures/coverage-api.json`; source observers:
`tools/isometric-coverage-scoreboard/{hook_test.go,meshjson.go}`, reusing
`tools/isometric-content-scoreboard/scenejson.go`. Capture/replay evidence:
`.reference/codex-runs/iso38/`. No new whole-board golden gain.

## Original ink, Markdown-card and opacity contracts (Lane I39)

.reference/codex-runs/iso39 records the original ink/card/composition batch
(master 6a7b48378): native_edges.go:102,429, native_ink_joins.go:12,
markdown_cards.go:12,26,33, native_shapes.go:403,
hierarchy_boards.go:77 and hierarchy_headers.go:25, plus raster.go.
Twenty original Test functions retain all Go assertions; 146 observed calls
match native Rust. No production algorithm changes were needed.

`img/native_edges/composition_api_tests` receives input-only original calls.
Its inclusion and the shared hierarchy/raster replay helper visibility compile
only under cfg(test). Ink meshes include ordered vertices, normals, UVs,
materials, support facets and group/owner sharing. Markdown calls additionally
compare source nodes, texture hashes, face/rich budgets, animation ranges,
links and header placement. Absent source-node slices retain their original
nil marker in observer echoes; native placement still consumes an empty slice.
Ink replay calls the public classic_ink_edges wrapper, preserving its source
caption-tail insertion behavior. The initial bridge bypassed that wrapper;
its corrected result is not a native runtime fix.

Opacity render/frame calls replay original Go-built meshes through native
Rust rasterization. Complete nonempty raw RGBA inventories and hashes are
checked with the existing raster gate. Go cached-buffer layout, pointer
allocation and caller assertions outside observed operations are unqualified;
these named observations do not establish every original assertion natively.
The combined observers live in tools/isometric-composition-scoreboard/ and
reuse the solid, shape and hierarchy observers. Capture/replay:
scripts/isometric-composition-scoreboard.py; evidence:
.reference/codex-runs/iso39/final-source and final.json. Full original
inputs/expected outputs remain retained; no golden gain is claimed.

## Remaining route, opacity and background contracts (Lane I40–I41)

The interrupted I40 batch is completed by I41. Pinned master `6a7b48378`:
`geometry.go:431–480`, `geometry_routes.go:436`, `route_caption_mask.go:39,59`,
`opacity_groups.go`, `raster.go`, `solid_contacts.go` and `ground_receivers.go`.
Twenty original test functions retain their Go assertions. The residual
scoreboard observes 73 calls: ordered meshes and shared materials/owners,
packet routes, native scene/camera/background values, opacity validation and
snapshot ownership, shadow depth/opacity bytes and complete raster pixels.

Native construction now honors the supplied density and scene options. A
vector context is retained only for vector output or an explicitly requested
vector-surface context, matching Go's painter context rather than attaching
documents to ordinary raster labels. ARM64 assembly establishes the separate
contraction sites in caption masking: the projection fuses `dx*cos`, and
`geometry_routes.go:436` fuses the compiled-width product plus the gap. The
two solid-contact witnesses exposed the latter difference before clipping;
the clipper itself matches when supplied identical geometry and surface.
`contact-caption-api.json` preserves five complete original Go calls as an
ordinary native regression. No tolerance or output normalization is added.

The residual replay module and raster adapters compile only under `cfg(test)`;
requests contain inputs only. Original Go cached-buffer allocation, pointer
addresses and caller assertions outside observed calls remain unqualified.
Text-count and vector-context fields missing from I40's initial observer were
transport corrections, separately recorded from native runtime fixes.
Observers: `tools/isometric-residual-scoreboard/`; capture/replay and gates:
`scripts/{isometric-residual-scoreboard,test_isometric_residual_scoreboard}.py`.
Evidence and source mapping: `.reference/codex-runs/iso41/`, reusing I40's
complete `final-source2` input/expected-output capture. No new golden gain is
inferred from these API observations.
