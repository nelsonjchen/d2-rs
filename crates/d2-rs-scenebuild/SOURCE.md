# d2-rs-scenebuild source mapping

Port of D2 master 6a7b48378 `d2renderers/d2scenebuild`: `build(&Diagram,
&Options) -> Result<Document, Error>` mirrors `d2scenebuild.Build`. Input is
the d2-rs-target `d2target.Diagram` mirror; output is the d2-rs-scene
`d2scene.Document`. Error values carry Go's exact error text.

| Go (d2renderers/d2scenebuild) | Rust (src/) |
|---|---|
| build.go Build, Options, LinkBudget, SketchBudget, FontFallbackOptions, AssetOptions/SVGImportBudget, admitSceneWork, preflight/preflightShape/preflightConnection, buildDocument, diagramObject order, buildShape, buildShapeText, targetGeometry, scenePath, font, stroke, validate*, dimensions, checkedAdd/Sub, expandDimensions | `lib.rs` (error/integer helpers, boxClip/roundedBoxClip/imageShapeClip in `util.rs`) |
| connection.go buildConnection, animated tracks/timing, splitAnimatedConnectionPath, arrowheadAdjustment(s), connectionPath, arrowhead | `connection.rs` |
| labels.go buildConnectionMask, borderLabelMaskBox, shapeLabelPlacement, buildConnectionLabels, centeredTextRuns | `labels.rs` |
| special_shapes.go class and SQL table shapes | `special_shapes.rs` |
| shape_effects.go shadow/blend nesting, animated shapes, finishShape, 3D, multiple, double border, ordinary outlines, polygonPath | `shape_effects.rs` |
| patterns.go built-in patterns, overlay/interleave | `patterns.rs` |
| pattern_sources.go paper/grain/streak sources (assets shared from `crates/d2-rs-render/assets`, digests checked) | `pattern_sources.rs` |
| d2svgimport/path.go ParsePath (used by the pattern sources) | `svgimport_path.rs` |
| color.go paint; internal/scenepaint/gradient.go; lib/color ParseGradient (master); csscolorparser v0.1.8 Parse + RGBA255 (d2-rs-math `svg_color`) | `color.rs` |
| markdown.go Markdown labels, primitives, link regions; lib/textmeasure SafeMarkdownLink/IsDangerousLink (master) | `markdown.rs` |
| metadata.go link regions, budgets, linkDestination (d2-rs-language parse_key), visualBoundingBox | `metadata.rs` |
| tooltip.go positioned tooltips | `tooltip.rs` |
| legend.go legend preflight/measure/build; d2graph.GetTextDimensions (Ruler.Measure) | `legend.rs` |
| appendix.go appendix items, badges, rows | `appendix.rs` |
| code_shape.go code shapes and labels over the d2-rs-text Chroma translation; codeFontMetrics.advance | `code_shape.rs`, `shaping.rs` |
| font_fallback.go resolveFontFallbacks, shapeTextRuns, fontFace/fontCoverage, ensureMissingGlyphFont, sceneTextReferences; markdown.go markdownFontAdvance | `shaping.rs` |
| d2fonts/bundled_fallback.go normalized, resolve (scene handoff), admitBundledBytes; fallback.go uniqueFallbackRunes; noto_color_emoji_native.go bounded authenticated loading; internal/fontfallback/fontfallback.go AssetID/hashFont | `fallback.rs`; `shaping.rs` font asset/result admission, style-bucket reuse and per-reference selection |
| assets.go preflightAssets/shapeAssetIsEmitted, resolveImageAsset (raster and unavailable cases), resolveUnavailableImageAsset/newUnavailableImageAsset, hashImageAsset, image/icon nodes | `assets.rs`; `lib.rs` source/digest caches and AssetOptions.resolver_impl |
| latex.go input/import admission, currentColor, MathJax render, cache, vector asset and intrinsic placement | `latex.rs`, `svgimport.rs` |
| sketch.go, sketch_connection.go, sketch_shapes.go | not ported: `Options.sketch` paints are reported as unsupported |
| d2fonts.FontFaces.Lookup / registerBundledFontFaces | `fonts.rs`, `shaping.rs` |
| d2target BoundingBox, CalculateTooltipPosition, calculateTooltipBounds, connection label/icon/arrowhead positions, Arrowhead.Dimensions, GetIconSize, BaseConnection, VisibilityToken, ConstraintAbbr | `target_ext.rs` (adapted from d2-rs-render d2svg/target_methods.rs and tooltip.rs) |
| d2scene Matrix.Mul / Matrix.Point as compiled | shared `d2-rs-scene` `Matrix::{mul,point}` |

`src/golib/` holds unmodified copies of d2-rs-render's crate-private ports
of lib/geo (`geo.rs`, re-exporting `d2_rs_render::go_float`), lib/label,
lib/shape, lib/svg path.go, lib/color (`color.rs`, of which only
IsGradient and Darken are used) and lib/textmeasure LayoutMarkdown
(`markdown/`); `markdown/ruler.rs` adds `Ruler::measure` and
`Ruler::scale_unicode` (textmeasure.go Measure/scaleUnicode) for
GetTextDimensions. The lib/geo and lib/shape oracle/golden suites run in
d2-rs-render against the original files.

## Floating point

ARM64 contraction sites come from
`.reference/isometric-port/fma-sites-foundations.txt` and, where inlining
matters, the pinned listing (`go build -gcflags=github.com/d2lang/d2/d2renderers/d2scenebuild=-S ./d2renderers/d2scenebuild`
in ../d2 with the pinned toolchain and caches). Products by a power of two
are exact and written plainly. Findings:

- Every inlined `d2scene.Matrix.Mul`/`Point` in d2scenebuild (and the
  standalone `d2scene.Matrix.Mul`) rounds the SECOND product with FMULD and
  fuses the FIRST (`fma(m.A, r.A, round(m.C*r.B))`). The shared `d2-rs-scene` methods now follow
  that (run 45); the local workaround was removed. Fusing the second product
  broke 14 boards' arrowhead transforms in the last bit.
- SSA products held in variables still fuse at their later use:
  code_shape.go:182 `lineTop + lineAdvance` is `fma(FontSize, 1.3, lineTop)`.
- Go folds untyped constant expressions exactly before rounding:
  `-12*math.Pi/180`, `43.6/87.3` and the animated-shape filter expansion use
  their correctly rounded literals.

## Verification

`scripts/scene-scoreboard.py --tier scenebuild` runs the Go probe
(`tools/scene-probe`, prebuilt `.reference/bin/d2-scene-probe-6a7b4837`) and
`examples/scenebuild-probe.rs` (build:
`cargo build --profile diagnostic -p d2-rs-scenebuild --example scenebuild-probe`)
over every e2e golden board and compares documents value by value (font
bytes by content-addressed file reference) or error text byte for byte.
At this port: 1008/1008 match (897 documents, 111 identical errors).
`cargo test --profile diagnostic -p d2-rs-scenebuild` pins the individual
contracts above.

`scripts/scenebuild-fallback-scoreboard.py` retains the original assertions of
three master bundled-emoji tests and compares their four Build calls: full
scene JSON, every glyph and font-byte SHA-256, or complete errors. Constructor
limits are captured from the actual resolver instances. Build the native
adapter with `cargo build --profile diagnostic -p d2-rs-scenebuild --example
fallback-probe`. Run 48 improves this tier from 0/4 to 4/4, preserving 1008/1008
ordinary scene builds and 933 e2e SVGs. The fixture in
`tests/fixtures/bundled-fallback-master.json` preserves those source requests
and outcomes; the integration test also requires one resolver call per build,
including regular/italic bucket reuse. This is Build coverage; native raster
output from these new documents is not separately qualified here.

`FontFallbackOptions.resolver_impl` accepts an `Arc<dyn FallbackResolver>`.
`BundledFallbackResolver::new` provides bounded bundled-first resolution and
optional downstream delegation. Its immutable results use the source's
internal scene-cache byte accounting; this is not a translation of the
mutable public d2fonts ResolveFallbacks API. The scene model still stores its
own Vec of font bytes per Document, rather than Go's cross-document shared
slice; only the resolver's resource bytes and parsed font authentication are
shared. No new floating-point sites were added.

`AssetOptions.resolver_impl` accepts an `Arc<dyn ImageResolver>`. Its
immutable `ImageResource` models an already-probed imageasset.Resource;
fetching, format/metadata validation, first-frame semantics and cumulative
resolver limits remain the resolver's responsibility. Build mirrors
`assets.go:103–179,182–256,301–323`: source-cache lookup precedes resolver I/O,
kind/MIME/bytes determine the SHA-256 content ID, raster bytes become an
owned document copy, and only ErrUnavailable selects the shared vector
placeholder. Fatal resolver failures and nil results remain errors. There
is no new network/file loader in this API. Run 50 connects SVG results to the
bounded native importer; older marker-only callers still cannot resolve
resources. No new floating-point arithmetic sites in this handoff.

`scripts/scenebuild-asset-scoreboard.py` inventories all 14 functions in
master's `assets_test.go`. Twelve Build test functions yield 28 calls;
the two internal-only merge/cancellation functions are explicitly uncovered.
All original selected assertions, including I/O counts and source raster
assertions, execute unchanged. A trace-only observation at Resolver.Resolve
captures the actual resolved bytes/metadata/errors, which the native adapter
replays at the same scene boundary. Internal-package file mode avoids a
second production build without the test-only hook; all 12 selected functions
and their subtests must pass. Full scenes (font bytes by SHA-256), full raster
asset bytes and complete errors compare with the existing typed comparator.
Run 49: **8/28 → 18/28**, then run 50: **18/28 → 28/28**, zero
regressions. `tests/fixtures/image-assets-master.json` retains every call;
`cargo test --profile diagnostic -p d2-rs-scenebuild --example asset-probe`
tests all 28 image/icon calls, all eight LaTeX calls, source caching,
deduplication, nil/kind errors and independent document ownership. This tier does not qualify
native fetching/probing or native raster pixels for the new documents.

## Not ported

- Sketch scene geometry (sketch*.go, d2sketch scene compilation): no probe
  board builds with `Sketch`.
- Rooted local-file policy, broader HTTP error/ownership contracts,
  APNG/animated-WebP configuration and shared resolver caches. The ordinary board probe configures no
  resolver or import limits, so its 84 image/icon and 27 LaTeX errors remain
  identical errors, not rendered boards. Configured raster/SVG resources,
  unavailable-image placeholders and bounded LaTeX imports are supported.
  General SVG/API coverage beyond the retained cases remains unqualified.
- Host font discovery, cancellation-aware resolution, the mutable public
  d2fonts resolver API and cross-Document shared font slices. The native
  scene resolver boundary and bundled Noto path are implemented; arbitrary
  caller resolver implementations remain unqualified.
- Context cancellation (no Rust context is threaded through).
- Grapheme clusters for fallback coverage use d2-rs-text's uniseg UAX #29
  translation rather than the go-text segmenter.

## Run 50: shared SVG importer and LaTeX scene bridge (master 6a7b48378)

`assets.go:143–173,257–299,325–425` maps to `assets.rs` and
`svgimport.rs`: per-document remaining limits, max(parsed, emitted) charges,
atomic sorted raster merge, owned scene conversion, intrinsic viewport and
clip wrappers. The parser/compiler are native `d2-rs-math::svg_import`, not
an SVG rasterizer or external runtime. `latex.go:30–173` maps to `latex.rs`:
input/options/currentColor admission precedes formula+color cache lookup;
MathJax bytes and color define the asset ID; shape/connection labels use the
intrinsic imported dimensions. Document budgets are shared with images.
The arena bridge copies primitive values without new floating-point arithmetic.

The importer viewport helper is retargeted to master's `d2scene/aspect.go:28–54`:
explicitly rounded products replace the old FMA residuals. `viewport.go` is
unchanged since v0.9.0. The `mathtext-img-svg-viewport` recipe
(`tools/fixtures/mathtext-img.toml`) regenerates the 2,280 viewport/matrix
rows of `crates/d2-rs-math/tests/fixtures/svg-viewport-master.json` at the
pin; the superseded v0.9.0 rows remain at base commit 9b706ac1.

Original master asset calls: **18 → 28/28**; original LaTeX calls:
**2 → 8/8**, with complete scene JSON, owned asset bytes/font hashes and
exact errors. `scripts/scenebuild-latex-scoreboard.py` reuses the asset gate,
inventories all six original LaTeX test functions and keeps their Go assertions.
Fixtures: `image-assets-master.json`, `latex-master.json`. These scene-call
contracts do not establish arbitrary SVG/API or newly rendered pixel parity.

## Run 51: bounded local/data-URI raster resolver (master 6a7b48378)

`image_resolver.rs` supplies `RasterImageResolver` at the existing
`AssetOptions.resolver_impl` boundary. Mapping: `imageasset.go:70–112`
(inclusive caller limits), `:168–218` (owned chunked data URIs),
`resolve.go:72–132,183–254` (canonical same-session source memoization),
`:368–413` (explicit trusted local-file reader), `:507–576` (bounded
percent/base64 data URIs), `:687–720` (atomic cumulative reservation),
and `probe.go:230–359` (retained raster bytes, dimensions and decoded storage).
Configuration reuses the native math crate's PNG/JPEG/GIF/WebP metadata
admission. No entropy is decoded during resolution and no FMA sites are added.
The owned scene bridge keeps copying resolver bytes as before.

`scripts/imageasset-scoreboard.py` captures **17 actual Resolve calls** from
four original test functions, with Go assertions intact, and inventories the
remaining functions in all six imageasset test files explicitly. It compares
complete retained bytes, dimensions, MIME/kind, data URI, accounting and errors;
the new native path changes **0 → 17/17**. The permanent original fixture is
`tests/fixtures/imageasset-local-data-master.jsonl`. The native asset example
also builds the complete original Go image/icon scene directly from its data
URI, without a recorded-resource resolver. Local retry, canonical cache hits,
owned output, positive limits, cancellation precedence and bounded base64
growth have separate native contracts.

Scope is explicit: HTTP, SVG acquisition, rooted file policy, external caches,
parallel per-source loading and full context concurrency semantics remain open.
PNG and WebP use static admission; animated GIF configuration is supported.
The ordinary no-resolver corpus still has 84 asset errors, and this finite
call tier does not qualify full imageasset, new raster pixels, or whole
isometric boards. General scene construction still has no context parameter.

## Run 53: native local/data-URI SVG acquisition (master 6a7b48378)

`image_resolver.rs` now recognizes declared MIME aliases and local .svg/.svgz
extensions (`resolve.go:399–413,507–576`, `probe.go:355–380`), gives raster
signatures priority, and calls shared `d2-rs-math::image_svg` for bounded SVG
admission/expansion (`probe.go:39–227,418–590`). KindSVG resources retain raw
source bytes or final decompressed XML, zero pixel dimensions, final encoded
and decoded byte charges, and source fetched accounting. Reservation remains
atomic, and all errors remain fatal unless the I/O boundary says unavailable.
The same session memoization/data-URI ownership and existing scene importer
apply. No nested resource is fetched by acquisition.

Original fixtures `imageasset-svg-master.jsonl` and
`imageasset-svg-codecs-master.jsonl` contain eight original raw/ISO/DTD calls
and seven original compression/limit calls. The source test functions keep
their assertions. Ten original SVG scene/import-budget outcomes also run with
the native acquisition resolver instead of a recorded-resource implementation.
HTTP, rooted file policy, cross-session caches and concurrent-loader context
semantics remain unqualified; named remote-image boards still require HTTP
integration. This does not claim a whole-isometric-board or new pixel match.

## Run 54: HTTP imageasset acquisition (master 6a7b48378)

`image_resolver.rs` and `image_http.rs` translate `lib/imageasset/resolve.go`
classifySource/displaySource (`216–294`), loadHTTP (`418–495`),
selectContentType/parseContentEncodings (`790–870`). HTTP canonical requests
retain credentials/query privately and drop fragments; diagnostic labels drop
credentials/query/fragment. Response status and declared length precede bounded
MIME/encoding header validation and streaming body admission; decoded image
bytes then enter the existing probe and atomic session reservation. Transport
unavailability and custom redirect-policy errors remain distinct.
`d2-rs-math::image_svg::decode_http_content` handles reverse encoding layers.

`ImageHttpClient` is an explicit injected-client boundary. The native default
`NativeImageHttpClient` translates `lib/netpolicy`'s public-only hostname,
address-set, dial and redirect policy (the same source mapping as the CLI's
existing `http_asset.rs` adapter). Private networks require an explicit opt-in.
The native transport uses reqwest/Tokio for I/O, sends the upstream headers,
disables proxies under public policy and applies a one-minute request ceiling.
Cancellation drops request/body futures and keeps the caller's original cause.
Native transport compilation is excluded on wasm32; browser hosts must inject
their own boundary. The ordinary CLI still uses its separate existing adapter.

The `http` and `http-headers` imageasset scoreboard suites capture **18 original
Resolve calls**, including six codecs, stacked encodings, custom redirect
rejection, decompression/encoded/fetched limits, fragment memoization and header
bounds. All original source assertions execute. The test hook observes the
actual injected client's request/final raw response, restores the response body
and records exact resource/error/accounting expectations; it never supplies
expected resource bytes to the native probe. Fixtures preserve all calls.
Native socket tests additionally exercise redirects, request headers, sniffing,
public-policy rejection and stalled-body cancellation. A scene integration test
uses a fresh loopback locator with the original remote-origin fixture's bytes
and compares its full expected scene before/after server shutdown.

These finite calls do not qualify all URL syntax, malformed transport/codec
errors, arbitrary injected clients, transport ownership, shared-cache policy
boundaries or concurrent loader/context semantics. The header function's direct
canceled parse call/default transport header-cap assertion remain explicitly
uncaptured natively. Rooted local policy and end-to-end isometric CLI wiring
remain open; no new whole isometric-board/pixel match is claimed.

## Run 55: caller-owned bounded shared image cache (master 6a7b48378)

`image_cache.rs` translates the explicit `MemoryCache` and its deterministic
entry/encoded-byte LRU bounds (`lib/imageasset/imageasset.go:215–302`). A keyed
doubly linked list preserves constant-time recency updates without a new
dependency. Only resolver-created immutable snapshots enter the cache; callers
receive owned metadata and shared immutable bytes. Oversized replacement entries
remove a previous binding and are not retained. Cache limits must be positive.

`RasterImageResolver::with_shared_cache` requires a namespace and hashes it
with the caller's explicit public/private transport-policy tag (`imageasset.go:
348–354,389–396`). As with upstream Options, the namespace must identify the
installed transport's tenant/credentials and its policy tag must be accurate.
`ImageFileReader::cache_key` supplies policy-scoped local keys before lookup;
its default skips shared file caching, and Unrestricted explicitly opts in
with absolute canonical paths (`lib/localfile/localfile.go:111–132`). Future
rooted readers must supply retained-root identity and lexical admission here.

Resolution follows `resolve.go:107–159,633–682`: same-session hits charge once;
data URIs never enter the shared cache; shared hits revalidate fetched,
encoded, decompressed, SVG, dimension, pixel and decoded-byte limits, then
atomically reserve each resolver's own aggregate budget. Rejected/canceled
operations do not publish a new cache entry or change session accounting.
Cache-hit validation uses private construction invariants instead of accepting
arbitrary external Resource metadata. Custom Cache implementations, rooted file
policy, parallel per-source loading and full context semantics remain open.

Three permanent cache fixtures preserve 25 original Go Resolve calls from
seven test functions. All original assertions execute, including identity,
request counts, namespaces and eviction. Replay uses the captured opaque cache
prefix as a namespace token, preserving equivalence classes without rewriting
inputs or expected values. It compares full resources, errors, accounting and
cache occupancy. The direct empty-namespace New call is explicitly uncaptured;
its exact native error is independently unit tested. Other native tests cover
snapshot ownership, no-I/O hits after file removal, policy/base isolation,
cancellation before hits and atomic aggregate failure/retry. These are foundation
contracts for the named image-bearing boards, not new whole-board SVG/pixels.

## Malformed raster diagnostics (master 6a7b48378, run59)

`image_resolver.rs` retains normalized declared MIME types on failed raster
sniffing, or reports net/http's detected type when no declaration exists,
following `lib/imageasset/probe.go:112–118,355–377`. The shared detector is the
existing Rust Go net/http sniff translation in d2-rs-render. This fixes the
second-board bad PNG icon errors in isometric PDF/PPTX failure contracts;
native resolver tests also cover aliases, absent hints and unchanged charges.

## Lane L21: local image policies (master 6a7b48378)

`image_local.rs` maps lib/localfile/localfile.go:53–256 and open_unix.go.
The default denies access. Unix rooted policies keep a shared directory
handle and per-instance cache scope, reject lexical escapes, resolve relative
symlinks with an O_NOFOLLOW/openat descriptor walk, and share close across
copies. Opens are nonblocking and verified as regular files before bounded
reads. The configured path cannot retarget a retained root after rename.
The existing explicit UnrestrictedImageFiles adapter remains unchanged.
The rooted implementation is Unix-only; other platform rooted APIs and exact
non-witness OS error text are not qualified by the local imgbundler corpus.

## Lane L22: standalone localfile API (master 6a7b48378)

`ImageLocalFiles::{open,policy_cache_key,try_close}` maps lib/localfile/
localfile.go:101–201. Open returns a caller-owned regular-file descriptor;
bounded image reads reuse that acquisition boundary. `LocalFileError.denied`
preserves the ErrDenied identity for lexical denials; symlink/OS/closed-root
failures remain separate. Copies share the actual once-only descriptor close
and its result. Open files survive closing their root, as os.Root requires.
The original eight localfile functions replay 104 filesystem/policy/read/close
operations, including real FIFO rejection, renamed roots and concurrent
closes. Raw random cache keys are retained on both sides; the source tests'
key equality contract compares the complete equality matrix. Independent
random scope bytes are not claimed identical. Unix is the qualified platform;
other platform rooted APIs and non-witness OS/close-error spellings remain open.

## Lane L23: shared netpolicy address and validated-dial boundary

At master `6a7b48378`, `lib/netpolicy/netpolicy.go:129–198`
(`validatedDialContext`, `resolvePublicAddresses`) maps to public native
`netpolicy::{validated_dial,resolve_public_addresses,validate_public_addresses}`.
The native caller supplies a split host/port and resolver/dial closures; this
batch does not qualify Go's malformed SplitHostPort diagnostics. Address/name
predicates and reserved prefixes (`netpolicy.go:200–244`) share this module.
The production reqwest resolver uses the same complete-answer validation,
IPv4 unmapping and ordered deduplication. `public_client_builder` disables
proxies and installs this resolver. Both NativeImageHttpClient and the CLI
http_asset path consume it, removing their duplicate address predicates.

`scripts/netpolicy-scoreboard.py` observes all original assertions in
TestNonPublicAddressRanges and the three TestValidatedDial functions: **4
functions / 29 operations**. It compares complete errors and lookup/numeric
dial histories; requests include only actual injected resolver answers and
dial failures. Original repeat-connection operations replay in one process.
The baseline preserves the exact old CLI predicate; its three missing native
dial APIs remain unsupported. This is API coverage, not a new golden-board
fix. Four client/transport functions (redirect checks, caller hook isolation,
bypass-hook clearing and private opt-in custom transport ownership) remain
uncovered; Go transport-field identity has no qualified Rust adapter yet.

### Resolver HTTP transport lifetime (Lane I21, master 6a7b48378)

`image_resolver.rs` records HTTP-client ownership and implements explicit
`close_idle_connections`, following `lib/imageasset/imageasset.go:327–419`.
The default native client owns its request-local transport and retains no idle
pool. `with_http_client` borrows a caller-owned client; the new
`with_owned_http_client` transfers a boxed, independently configured client.
Policy adapters remain responsible for constructing isolated public-policy
clients before transfer. The `ImageHttpClient::close_idle_connections` default
is a no-op for clients without a retained pool; injected pool owners can
implement it. Closing never invokes a borrowed client's cleanup callback.

Original `TestResourceDataURIContext` and
`TestResolverClosesOnlyOwnedHTTPTransport` assertions are observed by
`tools/imageasset-lifecycle-scoreboard/hook_test.go` and replayed by the native
example of the same name. All five returned data-URI byte slices (including
100,000-byte chunked input, cancellation and nil) and ten constructor/close
operations compare. Rust absence uses `Option`, rather than nil receivers.
The empty Go standard-transport witness maps to an independently constructed
native default client; arbitrary Go HTTP-client configuration cloning and
active-connection behavior are not qualified by these finite tests.

`netpolicy_client.rs` translates the client ownership boundary in pinned master
`lib/netpolicy/netpolicy.go:48–125`: safe scalar options are cloned, public-only
clients discard caller transport callbacks and custom transports are admitted
only with private-network opt-in. Standard transport callbacks are Rust
`ClientBuilder` configuration, rather than Go transport-field emulation.
`ClientIo` provides the unexported Go resolver/dial test seam using validated
numeric socket routing. This finite adapter does not qualify arbitrary Go
HTTP configuration cloning, connection-pool identity or protocol-field identity.
Original remaining client tests are observed by
`tools/netpolicy-client-scoreboard/hook_test.go`; the native replay compares
bodies, endpoint inventories, DNS/numeric destinations, cancellation/error kinds,
callback isolation and shared custom-transport ownership. Go's original field
assertions remain active. Three native regressions additionally exercise mixed
DNS answers, actual custom transport calls and cancellation during a request.

### Lane L36: named font registrations (master 6a7b48378)

`fonts.rs`/`Builder::font_for` now read owned snapshots from the same explicit
font registry as SVG, following `d2scenebuild/build.go:1066–1131` and
`markdown.go:345–386`. The Markdown ruler keeps per-ruler parsed custom
families, following `lib/textmeasure/textmeasure.go:121` and its named font
lookup. Custom identity remains distinct from bundled identity even when
bytes coincide (including the family-sensitive heading rules). Glyph,
kerning, CSS metrics and layout arithmetic are unchanged. `legend.rs` uses
the same named ruler lookup.

The original isometric configured-font test supplies different registered
primary/mono families and explicit HandDrawn rich text; the complete source
documents and font bytes compare in `scripts/isometric-font-scoreboard.py`.
Its resolver companion compares complete supplied-fallback texture output
and requests. This extends the named registration path; no host font
search is introduced.
