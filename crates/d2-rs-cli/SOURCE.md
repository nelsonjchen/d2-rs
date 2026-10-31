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

The watch-mode browser client and favicons in `crates/d2-rs-cli/src/watch_static/`
are copied from `d2cli/static/` in the pinned D2 v0.9.0 source. Rust's watch
server and file-dependency tracking are separate MPL-2.0 implementations.
The preview listener follows `d2cli/watch.go:465–472` and Go 1.27.0
`net/dial.go:889` (prefer a resolved IPv4 address, otherwise the first address).
Dependency-change and broadcast notices follow `d2cli/watch.go:296–309,619–637`;
the original CLI watch tests exercise these notices alongside live navigation
and imported-file updates.

Single-board SVG target hashing follows `d2cli/main.go:368–377,542–545`:
an explicit empty or named target clears descendant diagrams before HashID.
`d2svg_subtree_hash_input` retains children only for the default/wildcard
selection; recursive document rendering explicitly requests those subtrees.
The regression `explicit_board_targets_hash_only_the_selected_board` checks
root/named hashes and wildcard hashes against pinned Go, with both flag forms.

Static SVG board-link rewriting follows master `d2cli/main.go:550–558,704–714,
778–863`: clear the selected board's descendants before building the full-root
link map, retaining siblings and ancestors. `scripts/target-link-scoreboard.py`
compares complete SVG bytes and output inventories for root/named/wildcard
selection, node/connection links and file/stdout output on all built-in engines.
Internal hash traversal retains child subtrees rather than pruning each child
as a new CLI selection. Stdout link paths keep Go's empty filename extension
(`board_output.go:109–116`), and multiboard stdout rejects with master's exact
`main.go:902` diagnostic. Six named-subtree stdout source cases remain open.

The watch HTTP/WebSocket request handler follows `d2cli/watch.go:475–486`'s
`net/http` server: wait for request headers before parsing the handshake.
Accepted sockets explicitly clear the nonblocking flag that macOS can inherit
from the listener; WebSockets enable it again after the handshake. A regression
holds back request bytes on a nonblocking accepted socket and checks the full
favicon response. The prior handler fails that test immediately with no data.

Before publication, supply a verified accessible source URL and exact source
revision for every artifact. Include LICENSE, THIRD_PARTY_NOTICES.md,
the repository root's DEPENDENCY_NOTICES.md and this file with every binary/WASM distribution and
crate. Source recipients must receive the MPL-covered modifications needed to
build their version. Third-party material retains its applicable terms.

Markdown measurement continuation487 delegates to d2-rs-render::markdown_measure::MarkdownMeasurer with existing custom face references; source geometry algorithms remain native Rust. Evidence: .reference/positioned-tooltip-20261006/paint-source-comparison.md.

Checkpoint517 follows original `d2graph.Object.GetLabelSize`'s TeX branch:
math dimensions are intrinsic and independent of surrounding font size.
Successful native math measurement no longer multiplies by font/16; ordinary
text/error fallbacks retain their existing font behavior. Actual bundled and
fallback CLI measurers are tested at16/20/30; WASM receives the same correction
and native-host compilation check only. General Native TeX parsing/layout,
whole-formula witness removal and full WASM runtime parity remain open.
Evidence: .reference/grid-math-raster-20261007/ and docs/STATUS.md checkpoint517.

Checkpoint 879: source d2scenebuild/assets.go::preflightAssets/resolveImageAsset and lib/imageasset/resolve.go::Resolve/classifySource retain non-nil empty connection icons and reject the empty locator with the classify-source error. Use authored Go edge identity in scene acquisition errors. Source/node error probes, accepted target fixtures and raw original-Go CLI stderr live in .reference/native-target-icons-c4-20261016; raster_asset::tests covers exact scene errors.

SVG scale presence continuation (2026-10-18): original d2cli main keeps a scale
pointer only for a positive command/environment value. Finite nonpositive CLI
values therefore retain fit-to-screen SVG behavior; the public API separately
preserves an explicit zero/negative pointer. Omitted/default scale passes None
through rendering, while explicit one disables fitting. PNG/native raster paths
retain their intrinsic default-one canvas. Fresh source/before/final SVG captures
cover34 cases, both browser viewport sizes and all three in-process engines.
Non-finite CLI scale inputs remain outside this checkpoint's qualified domain.

Run 26: d2cli/raster.go:633–666 uses framesPerBoard for static boards too;
render_source_animated_gif and preview GIF now use the interval-aware exporter
for every board, including one-layer-gif. Loop count and quantization are
owned by d2-rs-export (see its SOURCE.md).

Run 27: directory inputs follow `d2cli/main.go:231–244`: choose the default
output from the original argument, then resolve directories to `index.d2`.
The resolved source path controls imports and diagnostics; explicit outputs
retain their path. `multiboard/life_index_d2` is the upstream five-board witness.

Run 29: `load_render_fonts` carries the raw custom face bytes selected by
`d2cli/main.go:1117–1200` into per-render d2svg options. Ordinary and animated
SVG embedding uses those bytes instead of losing them after measurement.
The upstream witness is `TestCLI_E2E/with-font`; reduced controls additionally
cover regular, mono and animated output. See d2-rs-text's custom-family map.

Run 30: `pptx_properties.rs` and the document rendering boundary follow
`d2cli/main.go:630–644` (presentation title/subject from the output basename,
input basename for stdout, `stdin` for stdin-to-stdout; creator from the current
OS account). Both single-board and multiboard paths pass explicit presentation
properties. Multiboard stdout returns the package bytes instead of creating a
literal `-` file, following main.go:656–668. Unix account lookup uses
`getpwuid_r(getuid())`; Windows requests NameSamCompatible with GetUserNameExW.
macOS is qualified here; the Windows branch has not been runtime-qualified.

The animation destination check follows `d2cli/main.go:252–258`: reject a
non-SVG/GIF file before source I/O, preserving the stdout exception.
`fmt --check` counts every differing file without rewriting, as
`d2cli/fmt.go:26–66` does; singular/plural failures retain its exit diagnostic.
Scoreboard cases: stdout_pptx, incompatible-animation, fmt-check-unformatted.

Run 38: named GIF `board.*` targets follow `d2cli/main.go:368–385,539–549`
(GetBoard then retain children), using the compiler's canonical board resolution
before filtering descendants. The selected board is the GIF traversal root;
`d2cli/raster.go:580–629,929–975` supplies preorder renderable boards, skipping
folder-only parents. `main.go:582–608` writes the composed GIF via the normal
file/stdout boundary, including stdin sources. Native GIF packaging returns
stdout bytes rather than rejecting the destination or writing a literal `-`.
Nine complete original-Go CLI witnesses cover nested, shorthand and quoted
mixed-collection subtrees, folder-only/leaf selection and file/stdin/stdout.
Fixture: tests/fixtures/gif-subtree-source-v090.json; process regression:
tests/gif_subtree.rs; generator: .reference/codex-runs/witness-inputs/run38-gif-subtree.py.

Run 39: PDF/PPTX selections follow `d2cli/main.go:368–385,539–549,610–667`:
GetBoard precedes packaging; a target without `.*` drops children. Paged
traversal starts at that selected board with ID `root`, then appends relative
collection/name components (`d2cli/paged.go:408–553,656–683`). Navigation uses
the selected root label; PDF descendant titles use labels and PPTX uses names.
The native plan keeps compiler-resolved absolute targets for rendering and
rebases only document page IDs/title paths, omitting external ancestors and
folder-only pages. `scripts/check-paged-subtrees.py` checks 24 original-Go CLI
fixtures: all page dimensions/streams/used image bytes/annotations for PDF,
and every ZIP member for PPTX. PDF clock bytes remain outside this supplementary
contract; the upstream strict comparator is unchanged.
The selected root's default label follows `d2exporter/export.go:25–28`: use
the board name only when no label was declared; an explicit empty label stays
empty. This controls navigation for both renderable and folder-only roots.
Two additional error fixtures require `paged.go:66–76`'s no-renderable-board
error when a non-wildcard selection removes a folder's children; they also
require a nonzero process exit and a complete empty output inventory.

Run 40: standalone PNG output adds D2's eXIf chunk after IHDR, following
`d2cli/raster_png_encoder.go:305–318` and `lib/png/png.go:199–229`. The native
format boundary covers file, stdin/stdout and every recursive board output;
`--omit-version` does not suppress PNG metadata. The model uses the CLI's
release version `v0.9.0`, consistent with its version command. The source-only
oracle's `v0.8.1-HEAD` remains visible; no comparator normalization or source
version change was made. All 12 upstream PNG output files match complete
release-stamped Go CLI bytes. `scripts/check-png-metadata.py` replays seven
additional Go process contracts (ten complete PNGs), backed by
`tests/fixtures/png-metadata-source-v090.json`.

### User-facing CLI boundary (2026-10-24)

`dispatch` now parses the complete pinned flag set before every user-facing
command, as d2cli.Run does (main.go:162–196), via
`command_line::version_boundary`; the d2-rs internal protocol/source
commands keep their own parsers. Ports: help.go layoutCmd, shortLayoutHelp,
longLayoutHelp, layoutNotFound and the d2plugin LongHelp templates with
xmain.Opts.Defaults() over each plugin's hidden flags; fmt.go fmtCmd and
validate.go validateCmd (explicit path, ms.AbsPath/ReadPath errors, validate
via d2lib.Parse without a path); export.go getOutputFormat; main.go's
too-many-arguments, ppt, animate-interval, theme and dark-theme checks, the
non-SVG dark-theme warning, and Success.Printf after every export.
`tools/cli-scoreboard/hook_test.go` models xmain.mainFatal: only `err:`
lines are the error and a UsageError's `Run with --help` line is dropped.
Differential evidence: `scripts/cli-diff-scoreboard.py`.

### D2 master 6a7b48378 CLI (2026-10-24)

- `d2cli/board_output.go` + `board_output_publish.go` → `src/board_output.rs`:
  boardOutputComponent (`_d2_<sha256>` for unsafe/reserved names),
  validateBoardOutputPaths collisions, the staging workspace and publication
  (rename when absent; otherwise install, retire unchanged stale files from
  the previous manifest, remove emptied stale directories, write the
  manifest). The shared collision key applies full Unicode case folding and
  NFC; stale aliases additionally require filesystem identity (run 74).
  `main.rs` run_multiboard_document stages whenever the rendered (sub)tree
  has child boards, watch subtrees included; an empty document with child
  boards publishes only its manifest.
- `main.go` relink/boardOutputLink: connection links relink too; components
  are url.PathEscape'd (`relink_board_link`, `url_path_escape`).
- `watch.go` access token (`watch.rs` WatchAccess): 32 random bytes, base64
  RawURL; `?token=` → 303 + `d2-watch-<port>` HttpOnly/SameSite=Strict cookie;
  otherwise 403 `watch access denied`.
- `help.go` built-in layout wording; `layout_elk.go` `--elk-edgeEdgeBetweenLayers`.
- Image assets (master `lib/netpolicy`, `lib/imageasset`, `lib/localfile`,
  `lib/imgbundler`, `d2cli/main.go` privateNetworkEnvDefault/netpolicy.WithPolicy,
  `d2cli/raster.go` svgBundleResolver/newImageAssetResolver):
  - `netpolicy.Policy` → `http_asset.rs` NETWORK_POLICY
    (`set_allow_private_network`, set once by `main.rs` apply_network_policy
    from `--allow-private-network`/`$D2_ALLOW_PRIVATE_NETWORK`).
    isLocalHostname, isPublicAddress + nonPublicPrefixes (after Unmap),
    resolvePublicAddresses and PublicResolver now share
    `d2_rs_scenebuild::netpolicy` with the native scene image client.
    `PublicResolver` (reqwest dns_resolver: every connection re-resolves and
    rejects the whole answer set if any answer is non-public) plus
    `host_is_blocked` for IP literals, which hyper dials without the resolver,
    checked on the request URL and in the redirect policy. newHTTPClient's
    `Proxy = nil` → `.no_proxy()`; the policy dialer's 30 s → connect_timeout.
    A blocked destination is a transport failure without a response
    (unavailable "HTTP request failed"), like Go's dial error.
  - `privateNetworkEnvDefault` → flag-table `default_env` handled in
    `command_line.rs` defaults(): ""/0/false, 1/true, otherwise the usage
    error, evaluated in flag order before any subcommand like Go.
  - `imageasset.Resolver.Resolve` (classifySource/displaySource/uriScheme,
    loadLocal, loadHTTP, SourceError, LimitError text) → `raster_asset.rs`
    `resolve`/`resolver_base`; `localfile.Unrestricted().Open` (O_NONBLOCK
    open, requireRegular) → `open_regular`. Raster preflight (`acquire`) and
    SVG bundling share it; the HTTP cache namespace includes the policy.
  - `imgbundler.BundleWithResolver`/runResolverWorkersWithLimit/
    classifyReference/boundedLabel → `main.rs` load_asset_hrefs,
    collect_asset_results_concurrent (one pass in rendered order, 16 workers,
    no dispatch after the first failure, at most 8 `failed to bundle image:`
    lines of ≤1024 bytes), report_asset_failures (`failed to bundle images:
    <first failure>`); data URIs keep the resource MIME type.
  Not reproduced: Go cancels in-flight resolutions after the first failure
  (their `imageasset: resolve ...: context canceled` lines race); d2-rs lets
  them finish. Probe-stage messages keep the existing Rust probe text under
  the `imageasset: probe "<src>":` prefix. Evidence: cli-diff-scoreboard.py
  image-asset cases (loopback server, no external network).

Master isometric SVG/PNG command path (run 57, `6a7b48378`):
`src/isometric_cli.rs` translates `d2cli/isometric.go:10–45`,
`isometric_image.go:21–243,245–327` and the document wrapper in
`d2renderers/d2svg/isometric.go:94–216`. `main.rs` connects the flag,
environment/source precedence and pre-/post-compilation admission points in
`d2cli/main.go:126,189,279–307,519–547`. The native layout and target projection
are shared with ordinary exports; `d2-rs-isometric` supplies vector/PNG output.
The helper retains Export's allocated empty slices (`export.go:47–58`) in
untouched descendant targets. The current SVG board alone follows
`relinkIsometricSVGBoard`'s `append(nil, slice...)` copies, preserving the
nil/empty distinction in `Diagram.Bytes`. Only the document root has Config.
The render target decoder gains Config.Isometric at its source declaration
position (`d2target.go:46–58`); omission leaves ordinary hashes unchanged.

`tests/fixtures/help-master-6a7b4837.txt` is the pinned release-stamped oracle's
complete `--help` with only argv[0] represented by `{program}`. The two
`isometric-*-hash-master.bytes` fixtures are exact `Diagram.Bytes` from actual
CLI root/board renders of the source in their tests. A temporary env-gated
hash hook in the trace checkout captured them; patches and commands are kept
in `.reference/codex-runs/run57-go-hash{,-tree}.patch` and report-run57.md.
No hook or Go execution is part of native rendering. The subprocess corpus
in `scripts/isometric-cli-scoreboard.py` follows source inputs/contracts in
`d2cli/isometric{,_image,_svg}_test.go`; it does not claim to execute every
upstream in-process assertion. GIF/timeline remains unsupported by this command
adapter. PDF/PPTX and watch previews follow below.

### Isometric paged command exports (master 6a7b48378)

`src/isometric_paged.rs` translates `d2cli/isometric_paged.go` and the
shared traversal/link adapters in `d2cli/paged.go:428–705`. It feeds native
`d2isometricimg.RenderPage` PNGs and source-pixel links to the existing
PDF/PPTX packagers. Board IDs include folder ancestors while title paths omit
them; selected subtree links rebase to the document's root. PDF annotations
round outward after division by png.SCALE; PPTX rounds in source pixels.
Presentation creator is empty as in isometric_paged.go, independently of the
ordinary command's user metadata. Cumulative pixel/PNG/link/string admission
and complete-document encoding precede atomic publication.

`scripts/isometric-cli-scoreboard.py --suite paged` retains 24 finite command
witnesses from `d2cli/isometric_paged_test.go`, including complete bytes and
failure destinations/modes. It does not execute every in-process assertion.
PDF full bytes require the explicit shared UTC clock and upstream FPDF catalog
sort control; PPTX full bytes use the release-stamped oracle. Run59's four
Architecture file/stdout failures came from shared compiler measurement
restricting paired link/tooltip padding to HTTP(S), unlike
`d2graph/d2graph.go:1644–1653`. Lane L's compiler change closes them; run60
qualifies **24/24** complete paged outputs/errors. No output correction is
applied in this adapter.

### Isometric watch previews (master 6a7b48378)

`src/isometric_watch.rs` translates `d2cli/isometric_watch.go:17–99`:
first drawable board selection, detached descendant slices for the preview
hash, absolute escaped full-tree navigation, a fixed 1600x1000 quality canvas,
and the file export's selected theme. `isometric_cli.rs` follows
`main.go:578–600`: prepare the preview before publishing the selected subtree's
SVG/PNG/PDF/PPTX file, then return the preview to the watch socket. Source
reloads continue through the ordinary config/flag selection on each compile.

`scripts/isometric-watch-scoreboard.py` drives real native and pinned-Go watch
processes with authenticated HTTP/WebSocket requests. Its 20 finite stages
follow `isometric_watch_test.go`'s source-mode reload, first-descendant and
selected/sibling/root navigation contracts. Complete preview messages and
file bytes/inventories compare; all PNGs also pass the existing raster gate.
This is a subprocess source comparison, not execution of every in-process
upstream watch assertion or GIF support.

`src/imgbundler.rs` translates D2 master **6a7b48378**
`lib/imgbundler/imgbundler.go:195–241,251–291,368–436`:
`bundleBudget.reserveBundled`, reference classification/scanning,
`resolveReplacement` serialization and `applyReplacements`. The command adapter
`src/imgbundler_cli.rs` composes these with the native imageasset resolver and
existing cancellation-aware worker collector. Limits count eligible occurrences
before href deduplication; replacement bytes accumulate once per raw href in
one document; output admission precedes writing. `main.rs` delegates generated
SVG bundling to this shared path. No Go/JS algorithm is used at runtime.

`scripts/imgbundler-scoreboard.py` and `tools/imgbundler-scoreboard/hook_test.go`
capture six original resource-limit test functions (12 calls), retaining their
assertions. The native adapter uses explicit pre-resolved SVG resource fixtures;
resolver/network/localfile policy and transport scheduling are not qualified by
this tier. The replacement/budget APIs and CLI bridge have permanent Rust tests.

The caller-owned resolver API in `imgbundler.rs` additionally maps
`BundleWithResolver`, `runResolverWorkersWithLimit` and `resolveReplacement`
(**imgbundler.go:57–82,296–389**). It composes the native scenebuild
`RasterImageResolver`, retaining canonical MIME/data bytes, a sticky shared
replacement budget, the first failure cause and completed replacements.
`scripts/imgbundler-resolver-scoreboard.py` observes seven more original test
functions through their raw mock HTTP replies and local file bytes. Twelve
calls compare complete output/error bytes; the scheduling test uses its
original predicates because the first failing URL depends on Go's worker
scheduler. Its complete raw Go and Rust errors remain visible, without string
normalization. Consecutive document-cache calls run in one native process.
This tier does not qualify network policy, rooted file policies, or transport
ownership. The CLI retains its existing native resolver/worker bridge.

The new caller-owned API accepts externally signaled cancellation; upstream's
automatic five-minute timer and logger/ticker adapter remain unported at this
API boundary. They are not qualified by the finite resolver corpus.

## Lane L21: explicit local bundling policies (master 6a7b48378)

`imgbundler::bundle_local_with_policy` follows lib/imgbundler/imgbundler.go
84–118,151–180: base directory, caller local policy, legacy limits, fresh
per-document resolver/cache, complete output and nested error propagation.
The default ImageLocalFiles policy denies reads; rooted and unrestricted
access require an explicit choice. This API reuses the native imageasset
resolver and worker implementation; CLI main.rs is unchanged.

## Lane L22: remote bundling policy (master 6a7b48378)

`imgbundler::bundle_remote_with_policy` maps lib/imgbundler/imgbundler.go:
121–180. Each invocation owns its resolver and optional cache; private-network
permission is explicit and local to that invocation. The existing native
imageasset HTTP client enforces literal, DNS and redirect policy. Local and
remote legacy wrappers share the source's limits. No process-global policy
or asset cache is added. `asset-policy-scoreboard` replays all four original
network-policy calls against a real loopback server in one native process,
checking full output/error bytes and endpoint request counts.

## Run 65: isometric GIF CLI selection (master 6a7b48378)

`main.rs` passes the authored signed interval into `isometric_cli.rs`.
The GIF branch follows `d2cli/isometric_image.go:120–145`: collect drawable
boards in static traversal order; use RenderTimeline for subboards or an
explicit interval (default hold 1000 ms), and Render for a lone leaf.
The native GIF/timeline APIs own the palette, common camera and frame timing.
GIF always publishes one file or stdout, with the existing atomic writer and
caller cancellation. Folder-only boards contribute no blank image.
`scripts/isometric-cli-scoreboard.py --suite gif` compares 16 finite CLI
witnesses from `isometric_test.go:126–149` and `isometric_image_test.go:89–165,
216–244`, using the established complete composited-frame/delay/canvas/loop
gate. This subprocess corpus does not execute all in-process Go assertions;
encoded GIF byte identity is not claimed.

## Run 66: formatter logging and typed exit boundary (master 6a7b48378)

`fmt --check` follows `d2cli/fmt.go:18–69`, `lib/log/prettyhandler.go:30–51`
and util-go v0.2.0 `xmain/xmain.go:58–82,133–151`. The formatter preserves
its wrapped API ExitError; main prints its embedded message and returns its
exit code. Other errors keep their existing contexts. `cli_log.rs` writes
warning paths to process stdout with Go's ANSI colors, level padding and
local RFC3339 timestamp. Valid `SOURCE_DATE_EPOCH` explicitly selects UTC
for reproducible warnings, as it does for document dates.

The optional `D2_RS_CLI_ERROR_JSON` transport writes the native API error,
terminal message, kind and code to a fresh file. `D2_RS_CLI_PROCESS_LOG`
lets an embedder supply a separate process-log sink. The upstream subprocess
adapter uses these to preserve the distinction between xmain.State output
and process stdout, and between returned errors and mainFatal's projection.
It validates typed exit records against actual process status/message;
original upstream assertions and artifact comparisons remain unchanged.

`scripts/build-cli-clock-oracle.py` temporarily instruments only the pinned
trace checkout with `tools/cli-scoreboard/log_clock.go` and the existing PDF
clock helper, builds with the release version stamp, then restores all four
touched files byte-for-byte. `cli-diff-scoreboard.py --go <that-oracle>
--epoch <Unix-seconds>` injects the same UTC clock into both processes and
compares complete bytes. Ordinary runs explicitly remove inherited clocks.
No timestamp normalization is used; ordinary wall-clock byte identity is not
claimed for records emitted at different instants.

### Run 74: Unicode board output collision keys (master 6a7b48378)

- `d2cli/board_output.go:281–283` → `board_output.rs::collision_key`:
  lexical path cleanup, full Unicode case folding and NFC replace lowercasing.
  unicode-normalization is pinned at 0.1.25 (Unicode 17.0.0, matching x/text).
  The source's stream-safe policy counts backward-combining starters too;
  generated lead/trail counts preserve its 30-nonstarter boundaries.
- `board_output_publish.go:474–482,815–830` → `same_output_path` and stale
  file/directory selection: preserve equivalent spellings only when they
  actually name the same file. Go `%q` collision errors use the shared Go
  quoting implementation, including literal combining characters.
- `tools/board-output-scoreboard/generate-fold.go` generates the immutable
  fold/nonstarter tables and the 23 direct-key fixtures using pinned
  `golang.org/x/text v0.41.0`. Run it from the manifest trace checkout with
  the absolute shared Go caches; pass `--fixtures` for the JSON fixture.
  Go runs only during generation/testing; every production operation is Rust.
- `scripts/board-output{,-collision}-scoreboard.py`: five original upstream
  publication functions (all assertions active, ten complete output snapshots)
  and four SVG/PNG collision inputs. Complete inventories, bytes, ownership
  hashes and errors compare. The observer reads each manifest's declared path
  through the filesystem, retaining both disk spelling and declared spelling;
  it does not normalize either or rewrite digests.

### Run 75: board manifest ownership admission (master 6a7b48378)

- `d2cli/board_output_publish.go:272–331` → `Manifest::read` and
  `validate_manifest_path`: accept nil/empty lists, enforce the 100,000-path
  limit, reject duplicate portable keys, manifest self-ownership and every
  file-as-ancestor claim after reading the complete inventory. Diagnostics
  use Go `%q`. Stale-file hash comparisons keep the existing lowercase-hex
  behavior (`staleFileChanges`, lines 499–508).
- `tools/board-manifest-scoreboard/hook_test.go` observes the three calls in
  `TestValidateBoardOutputManifestRejectsUnsafeOwnershipClaims` without
  replacing its assertions, plus thirteen finite validation cases. Native
  input-only replay and JSON transport compile only in tests; complete errors
  compare without normalization. `scripts/board-manifest-cli-scoreboard.py`
  checks six SVG/PNG rejection witnesses and one successful stale-file SVG
  witness, comparing complete inventories, bytes and errors at shared paths.
  This finite admission corpus does not qualify transaction rollback,
  concurrent ancestor replacement or every manifest decoder diagnostic.

### Run 76: preflight the complete board output tree (master 6a7b48378)

- `d2cli/board_output.go:433–482` (`preflightMerge`) → `collect_tree`:
  inspect destination types in lexical recursive walk order before installing
  any file or creating any destination directory. Reject existing symlinks
  and incompatible types with the complete Go diagnostic and `%q` paths.
- `scripts/board-preflight-scoreboard.py` and its Go test-only hook execute
  `TestMultiboardOutputRejectsPreexistingSymlink` and
  `TestMultiboardOutputRejectsPreexistingSymlinkAncestor`, preserving every
  original assertion. Two additional SVG/PNG boards put the conflicting file
  after `index` to expose partial publication. Complete returned errors and
  before/after node inventories, bytes, permissions and symlink targets compare
  at identical paths. PNG witnesses qualify rejection, not rendered pixels.
  This preflight change does not qualify concurrent ancestor replacement,
  descriptor-relative publication or transaction rollback after mutations.

### Run 77: publication journal and mutation-time checks (master 6a7b48378)

- `d2cli/board_output_publish.go:414–661,831–992` →
  `board_output/transaction.rs`: prepare the complete replacement/stale/manifest
  backup set before mutation; install files atomically with original modes,
  recheck stale-file identity and digest immediately before removal, install
  the manifest last, and undo applied files/directories in reverse on failure.
  Rollback refuses to overwrite a different installed identity; successful
  rollback clears the actual visible-mutation flag.
- `board_output_publish.go:117–165,664–809` → `board_output/root.rs`:
  Unix operations use an opened root and descriptor-relative parents,
  no-follow opens, exclusive temporary files and rename/link publication.
  Relative parent symlinks resolve beneath that root; absolute targets and
  `..` escapes fail. The eight-symlink limit follows pinned Go 1.27
  `os/root.go:76`. File identities come from opened handles (dev/inode on
  Unix; volume/file ID on Windows), never size/timestamp guesses. Windows
  retains a path-based parent fallback and is not qualified by this Mac run.
- `scripts/board-transaction-scoreboard.py` and its Go hook run all original
  assertions in the rollback, ancestor-swap and stale-edit functions.
  Rust's cfg(test)-only transport invokes the original Go callback
  synchronously at each production mutation boundary; it supplies no expected
  outputs to Rust. Complete errors, callback order, touched state and final
  inventories/modes/links/bytes compare at identical paths: **0/3 → 3/3**.
  Four ordinary native tests also cover restoration and a real source-read
  failure. No runtime Go/JS dependency or production JSON bridge is added.
  This finite corpus does not qualify arbitrary concurrent replacements,
  rollback failures, stage-root replacement, every filesystem diagnostic,
  or non-Mac platform behavior.

## Run 78: original isometric publication contracts (2026-10-25)

Test-only `isometric_cli/publication_api_tests.rs` invokes the unchanged
`atomic_image` through cancellation tracking. The input-only Go hook in
`tools/isometric-publication-scoreboard/hook_test.go` retains the original
assertions from pinned master `6a7b48378`:
`isometric_image_test.go:166–205`, `isometric_integration_test.go:43–89`,
`isometric_svg_test.go:204–241` and `isometric_paged_test.go:168–205`.
Runtime mapping: `d2cli/isometric_image.go:294–339` (atomic private sibling,
mode, sync, cancellation, rename) and `board_output*.go` (staged multiboard
publication). No production algorithm change.

`scripts/isometric-publication-scoreboard.py` qualifies four original
functions / 20 calls. Original failure/success contracts, complete stdout,
ordered before/after inventories, file bytes, modes and links compare at
identical paths; atomic returned errors compare exactly. The release profile
explicitly stamps Go with v0.9.0 and compares complete SVG/manifest bytes.
`--source-version` exposes the known source-versus-release stamp difference
without normalization; it is not counted as a runtime fix. Evidence:
`.reference/codex-runs/run78/`. CLI error text beyond the original tests'
failure assertions and arbitrary filesystem races remain unqualified.

### Run 79: tracked import opens (2026-10-25)

`watch::open_tracked_file` and `filesystem_import_opener` follow master
`6a7b48378` `d2cli/watch.go:740–752` (`trackedFS.Open`) and
`lib/localfile/{localfile,open_unix}.go`: open through an explicit regular-file
policy, then record a successfully opened dependency. Reuses
`d2_rs_scenebuild::ImageLocalFiles`' descriptor validation and Unix nonblocking
open. Import readers retain the opened file rather than eagerly reading a
potential special file. The FIFO import witness changes from a timeout to
Go's complete regular-file error. `scripts/tracked-fs-scoreboard.py` preserves
both original trackedFS test bodies; its input-only test bridge compares
complete errors, denied identity and opened paths, including nil unopened
slices. Original close assertions remain active; arbitrary close failures and
other platform-specific special files are not qualified by this projection.

### Run 80: paged admission and document writer (2026-10-25)

`isometric_paged::{Budget,DocumentWriter}` follows master `6a7b48378`
`d2cli/isometric_paged.go:135–175,215–235`. The writer checks cancellation
before its cumulative byte allowance and rejects a complete oversized chunk
before touching the underlying output. The owned-document entry transfers the
native packager's existing allocation after the same admission check. It does
not stream or limit the packager's internal allocations. Rust's `Write` trait
cannot return a byte count alongside an error; generic partial-write/error
combinations are outside this qualification.

`scripts/paged-admission-scoreboard.py` observes all nine calls in original
`TestIsometricPagedAdmissionAndBoundedWriter` (`isometric_paged_test.go:243–290`)
with its assertions intact. Test-only native transport receives inputs only;
complete page budgets, errors, writer counts/remaining allowance and emitted
bytes compare. No expected outputs, fixture constants or runtime Go dependency
are introduced. Evidence: `.reference/codex-runs/run80/`.

### Run 81: formatter context propagation (master 6a7b48378)

`d2cli/fmt.go:42` calls `d2parser.ParseContext` before formatting and
`WritePath` (line 53). `main.rs::format_source` now forwards the active native
cancellation token to `d2_rs_language::parse_with_context`, preserving the
parser's context-error precedence and polling instead of parsing without a
context. The original `TestFmtHonorsCanceledContextBeforeWriting` exposed a
file rewrite after cancellation; native bytes and mode now remain unchanged.

`scripts/cli-admission-scoreboard.py` observes that original test and both
enabled-profile calls in `TestEmptyDiagramRejectsUnavailableLayouts` without
removing Go assertions. The test-only input transport compares complete CLI
terminal errors, stdout, file bytes and modes. Error rendering uses upstream
util-go v0.2.0 `xmain.mainFatal`/`cmdlog` and native `write_cli_error`; it does
not normalize diagnostic content. The empty-layout calls already matched.
Native ordinary tests also exercise deadline cancellation. Go error identity,
disabled-engine build profiles and cancellation during filesystem writes are
not qualified by this projection. Evidence: `.reference/codex-runs/run81/`.

### Original isometric mode contracts (run85, 2026-10-25)

Pinned `6a7b48378`, `d2cli/isometric_test.go:35–193`:
`scripts/cli-mode-scoreboard.py` executes the five original precedence,
default/source-parity, failure, selected-board, and SVG-scale tests with their
assertions unchanged. A trace-checkout helper observes all 29 actual `Run`
calls. Native subprocesses receive only those args, environment, source bytes
and initial files. Complete stdout SVG, terminal errors and the entire
input/output file inventory/bytes/modes compare exactly. Both sides use the
release `v0.9.0` stamp (`go test -ldflags=-X\ github.com/d2lang/d2/lib/version.Version=v0.9.0`);
no bytes are normalized. The ordinary source stamp's initial 18 differences
are retained as version-profile evidence, not runtime fixes. All 29 calls
match; no CLI implementation changes were needed. This does not qualify the
separate Go `LayoutResolver` mutable-flag/slice snapshot tests or GIF watch.

### Export selection and private atomic SVG/text writes (run86, 2026-10-25)

Master `6a7b48378`, `d2cli/export.go:43–66`: `getOutputFormat` treats an empty
stdout flag as unset, and `getExportExtension` accepts only exact supported
filename extensions. `main.rs::cli_export_extension` preserves that distinction
from the more permissive library format parser. Explicit stdout format names
remain case insensitive. Uppercase filename suffixes and `.text`/`.ascii`
therefore default to SVG.

`d2cli/main.go:1210–1219` delegates SVG/text files to util-go v0.2.0
`xmain/xmain.go:185–216` (`AtomicWritePath`): write a private sibling, check
close, then rename over the destination, falling back to the direct writer
on failure. `atomic_write_file` now supplies this path for ordinary SVG/text,
including multiboard SVGs. Existing output replacement also receives the
temporary file's mode. The separate raster and isometric writers retain their
own upstream contracts.

`scripts/cli-mode-scoreboard.py --suite export` observes both original export
CLI test functions with their assertions intact, then runs eleven explicitly
separate source-derived witnesses. Native requests contain inputs only;
complete terminal errors, stdout and file inventories/bytes/modes compare.
The initial result was 3/16 calls; the final result is 16/16, including both
original unknown-extension calls whose SVG bytes already matched but modes
differed. The existing 29 isometric mode calls remain exact. Both sides use
the explicit release stamp; no output bytes or permission bits are normalized.
Evidence: `.reference/codex-runs/run86/`.

### Watch cookie/query authorization (run87, 2026-10-25)

Master `6a7b48378`, `d2cli/watch.go:532–573`: `authorizeRequest` obtains
`URL.Query`, strips token-bearing URLs through a redirect, and consults
`Request.Cookie` for cookie-only authorization. `watch.rs` now selects the
first successfully parsed cookie of the requested name, including quoted
values, instead of accepting a later matching duplicate. Query decoding
rejects malformed percent escapes and semicolon-bearing pairs, preserving
other valid pairs and arbitrary decoded bytes. Clearing `RawPath` before
`URL.String` and `http.Redirect` means redirect paths are decoded, escaped
as whole paths and cleaned before publication.

Parsing/escaping maps to Go **1.27.0** `src/net/http/cookie.go:371–416,555–574`,
`src/net/url/url.go:106–181,979–1015`,
`src/net/url/gen_encoding_table.go:154–201`, and
`src/net/http/server.go:2419–2485`. Existing Go notices apply.
This focused port does not qualify arbitrary HTTP methods, repeated header
lines or Go runtime `GODEBUG` parser-limit overrides.

`scripts/cli-watch-scoreboard.py` preserves both original watch-security
functions' assertions and captures seven original calls. Twelve additional
parser witnesses are named separately; eight initially differ. Native replay
receives inputs only. Application status, complete headers/body and the
authorization boolean compare; socket length/close framing is checked
separately because Go's `ResponseRecorder` does not synthesize transport
headers. Random token calls compare errors, length, URL safety and independent
generation, rather than random token bytes. Evidence: `.reference/codex-runs/run87/`.

### Source-configured isometric admission (run88, 2026-10-25)

Pinned master `6a7b48378`, `d2cli/main.go:521–542`, converts a
source-selected isometric export extension before checking appendix and raw
scale. `main.rs` / `isometric_cli::validate_source_format` now preserve that
order. Unsupported text output therefore reports the format error even when
appendix or an invalid scale is also requested. The earlier flag-selected
validation follows `main.go:279–305`.

`isometric_cli::image_dimensions` maps `isometric_image.go:20–51`, retaining
the dimensions returned on failure. Its `dimensions` quality wrapper maps
lines 54–65: SVG display scale leaves the geometry canvas fixed. The shared
arithmetic and production limits are unchanged.

`cli-mode-scoreboard.py --suite network` observes both original private-network
environment/help tests. `--suite image` observes all 19 calls in
`TestIsometricImageOptionsAndFailures`, plus two separately named source-derived
priority witnesses. Direct option calls compare format, width, height and
complete error; other Options fields are not qualified. CLI calls compare
complete terminal errors, stdout and file inventories/bytes/modes. Original
Go assertions remain active, and native requests carry inputs only. Evidence:
`.reference/codex-runs/run88/`.

### Original board path admission observations (run91, 2026-10-25)

Pinned master `6a7b48378`, `d2cli/board_output_test.go:17–124`:
`board_output/path_api_tests.rs` replays the original byte-array names and
diagram name/kind/folder trees. The adapter calls production
`board_output_component`, `main::board_output_path` and
`validate_board_output_paths`, mapping `board_output.go:25–82,203–286`.
Both collision functions compare their complete error strings, including
case folding, escaped duplicates and a folder requiring a board file as its
parent directory. Input roots in this finite capture are unnamed SVG roots.

Go's invalid UTF-8 name is outside the native `String` input type. Its two
determinism calls remain explicitly unsupported; the original
`TestBoardOutputComponent` function is not counted as a full match. The
other 60 name calls compare exact output strings. No runtime path algorithm
changes are made. `scripts/board-path-scoreboard.py` preserves every original
Go assertion and gives native replay inputs only; evidence is in
`.reference/codex-runs/run91/`.

Run97 below supersedes the invalid-UTF-8 limitation; the run91 negative
baseline remains retained.

Run91 also completes the pending run90 isometric export change:
`d2cli/isometric_image.go:234–236` wraps render/write failures for file-based
multiboard output with the selected tree's local board key. `isometric_cli`
now keeps that context through SVG/PNG render and write errors, preserving
direct errors for single-board and stdout exports. The original transactional
publication test's complete error comparison improves from 2/3 to 3/3 calls;
the existing staged files and preservation behavior already matched. Unsafe
names remain 1/1 and SVG export/failure-preservation calls remain 12/12.
The original captures and failing baseline are retained under run90; qualified
replays and the unchanged e2e guard are under run91.

### Original paged CLI observations (run93, 2026-10-26)

Pinned master `6a7b48378`, `d2cli/isometric_paged_test.go:43–162,202–238`:
`cli-mode-scoreboard.py --suite paged-export` observes the three original
image/link/navigation, board-selection and selected-subtree functions.
Original Go assertions remain active. Native subprocesses receive only the
original source files, arguments and environment; complete PDF/PPTX bytes,
stdout and file inventories/modes compare under the existing explicit shared
UTC clock, FPDF catalog sorting and release version profile. Standalone PNG
boards additionally use the complete raster/manifest gate.

`isometric_paged_api_tests.rs` observes the two original direct
`rebaseIsometricPageLinks` calls (`isometric_paged.go:113–135`) through the
existing production `isometric_paged::links` adapter. Target/URL, complete
error and unchanged source links compare; box and tooltip are preserved
opaque input metadata for this observation. The adapter asserts one output
per original link and receives no expected values. Existing native runtime
code changes only by declaring this test module. Evidence and reproducible
commands: `.reference/codex-runs/run93/` and `report-run93.md`.

### Original local artwork and source-GIF observations (run94, 2026-10-26)

Pinned master `6a7b48378`, `d2cli/isometric_surface_test.go:13–82` and
`isometric_test.go:126–148`: `cli-mode-scoreboard.py --suite surface-icons`
and `--suite source-gif` observe all six original CLI calls, keeping the Go
assertions active. Inputs are the original files, arguments and environment
(including empty PATH), with no expected values in native requests.
Local assets follow `isometric_image.go:78–101` through the existing native
`isometric_cli::resources`; GIF mode selection follows `main.go` and
the existing native isometric GIF path.

SVG bytes are exact. PNG uses the shared complete-output gate and exact
RGBA samples. GIF compares every composited frame, canvas, delay and loop;
the two native entry points must also produce identical encoded GIF bytes
to each other. Go/Rust GIF encoding bytes are not equated. The CLI harness
now distinguishes unchanged local PNG/GIF inputs from emitted images,
checking their complete bytes/modes while still rejecting unchecked output
images. No native runtime changes. Evidence: `.reference/codex-runs/run94/`.

Run 95 observes the original master `d2cli/isometric_svg_test.go:131–148,
176–207` link-source-preservation and URL-provenance tests (five rebase calls,
four followed by image Render). `isometric_image.go:247–299` maps to the
unchanged production loop extracted as `isometric_cli::relink_svg_board`.
The input-only `isometric_svg_api_tests` adapter converts Go's selected-tree
path-map IDs to the Rust CLI's full-source IDs and calls this same loop.
Complete target JSON, preserved input, raw SVG bytes and complete errors
compare; all original Go assertions remain active. These inputs do not
exercise root-link mutation, allocated-empty-slice identity or error paths.
Harness: `scripts/cli-mode-scoreboard.py --suite svg-links`, observer
`tools/cli-mode-scoreboard/svg_links_hook_test.go`; no new golden or runtime
fix is claimed. Provenance rendering uses the production image API without
CLI SVG wrapping or byte normalization.

### Raw board-name bytes (run97, continuing unfinished run96, 2026-10-26)

Pinned master `6a7b48378`, `d2cli/board_output.go:29–35,52–54`:
`board_output_component` accepts bytes as well as UTF-8 names. Like Go's
`boardOutputComponent`, it preserves portable valid UTF-8 and SHA-256 hashes
the original bytes of every other name. The existing input-only path replay
now calls this production function directly for all original byte arrays,
including the two `0xff` calls in `TestBoardOutputComponent`. No replacement
character conversion, expected-output change or new golden claim. The
original 66-call fixture and Go capture are unchanged; historical unsupported
responses remain non-passing in `board-path-scoreboard.py`.

### Original node/connection board-link API (run98)

Pinned master `6a7b48378`, `d2cli/main.go:817–875` and
`board_output_test.go:742–771`: `TestRelinkIncludesConnections` keeps its
original assertions and supplies two captured calls. The existing production
node-style loop and graph edge-link loop are extracted together as
`relink_document_links`; `relative_board_output_link` and `board_output_link`
share the existing relative-path/PathEscape behavior. The ordinary CLI calls
these helpers. The input-only test adapter transports the original target
links into node styles and the graph edge-link side table, then restores only
those fields in the complete original target. Endpoint topology is inert
transport for this test, whose original connections have no endpoint IDs.
No expected values enter the native request. The direct parent-relative
`boardOutputLink` call uses the same production escaping helper.

Harness: `scripts/board-link-scoreboard.py`, observer
`tools/board-path-scoreboard/links_hook_test.go`, native entry
`board_output::link_api_tests::replay`. Full target JSON/errors and escaped
strings compare without normalization; missing calls/responses cannot pass.
This finite Unix path test does not qualify recursive boards, error paths,
raw byte paths or arbitrary Go filepath.Rel behavior. No golden gain or
runtime bug fix is claimed.

### Original append-path containment calls (run99)

Pinned master `6a7b48378`, `d2cli/board_output.go:79–119` and
`board_output_test.go:115–155`: all nine calls of
`TestAppendBoardOutputNameStaysContained` retain their original Go assertions.
The CLI planner now shares `contained_board_output_component` and the lexical
`ensure_path_within` check with the replay. The existing collision-key lexical
cleaning is extracted unchanged as `clean_path`; filesystem publication still
uses directory handles independently. Seven unsafe-name calls use the actual
production byte-name helper; the hidden `.svg` call uses the original literal
`index` component. The input-only SVG adapter uses `Path::with_extension("")`
for the existing output-root behavior and appends the original `.svg` extension.
Complete paths and errors compare without normalization. These original inputs
do not qualify other extensions, arbitrary byte paths, symlink resolution or
Windows filepath behavior. No runtime bug or new golden-board gain is claimed.

Harness: `scripts/board-path-scoreboard.py`, observer
`tools/board-path-scoreboard/hook_test.go`, native entry
`board_output::path_api_tests::replay`; the retained original fixture now has
75 calls (66 previous calls plus the nine containment calls).

### Original isometric watch preview/reload tests (run100)

Pinned master `6a7b48378`, `d2cli/isometric_watch.go:19–95` and all four
functions in `isometric_watch_test.go`: the existing static preview copy,
link rewrite and painter are shared as `render_target_preview`. Folder inputs
select their first drawable descendant; animation master IDs and dark themes
remain file-export options. The source-driven watch path calls the same
helper. The input-only replay expands the observed acyclic target identity
graph into owned values, supplies source-board navigation paths and calls
this production helper. Complete SVG bytes, errors and unchanged input/options
compare. Cyclic Go pointer targets remain explicitly unsupported, including
the cyclic half of `TestIsometricWatchPreviewRejectsInvalidTrees`; this is not
an error-parity claim for the original full function.

`scripts/watch-reload-scoreboard.py` and
`tools/watch-reload-scoreboard/hook_test.go` preserve every original Go
assertion. The original reload test also runs against the real native CLI,
with all four emitted SVG inventories/bytes/modes compared. The Go test build
uses the same explicit `v0.9.0` version stamp as the pinned release-profile
oracle and Rust CLI. No SVG normalization. The subprocess adapter sends
SIGINT on context cancellation, waits for termination, and records Rust's
intentional exit 1 (`main.rs`, `watch::was_interrupted`) before mapping it to
the canceled API call; an API/terminal error or other exit cannot use this
mapping. Direct raw-pointer admission and compiler board-planner behavior
remain separate contracts. No golden gain or runtime bug fix is claimed.

The same batch observes `d2cli/layout_test.go:91–125`,
`TestBuiltinLayoutFlagOrder`: six actual CLI invocations retain the original
before/after SVG equality assertions for Dagre, ELK and TALA. Complete
stdout/errors and input/output bytes/modes compare. This does not qualify
mutable or concurrent `LayoutResolver` instances, nor legacy plugin protocols.

Lane L82 adds only `examples/renderer-admission-scoreboard.rs`: an input-only
native public API runner for original master `d2svg` raw-isometric admission
and `d2latex.ValidateInput` calls. It delegates to the production isometric SVG
facade and math validator; it performs no compilation/layout and has no expected
outputs. Original Go assertions, flat graph identity/non-finite transport and
complete errors are captured by `scripts/renderer-admission-scoreboard.py`;
cyclic/reused Go pointers and invalid UTF-8 Rust strings remain unsupported.

Run 102 maps master `d2cli/layout.go:51–70`, `layout_dagre.go:32–49`,
`layout_elk.go:43–80` and `layout_tala.go:29–47` to `layout_resolver.rs`.
The production CLI takes an owned snapshot of typed engine options and seeds;
resolved handles retain immutable shared ownership independently of the
resolver. Original Dagre/ELK spacing, TALA seed override/snapshot and concurrent
resolver calls are observed together by `layout-resolver-scoreboard.py` and
its temporary Go hook. `layout_resolver_api_tests.rs` accepts only captured
inputs, dispatches the production resolver and native engines, and projects
their results back onto the original flat measured graph records. It preserves
all source attributes/references and complete geometry/errors. The projection
maps ordinary `Object.Text` default font size (`d2graph.go:659–670`,
`d2talalayout/adapter.go:226`) and default inside label positions
(`d2elklayout/layout.go:1167–1175`); unsupported structured/hierarchical input
fails explicitly. Concurrent calls use eight native threads per engine.
No algorithm or fixture-specific output is introduced.
# Remaining original renderer contracts (Lane L83)

`src/layout_features.rs` shares the production CLI's Dagre/ELK admission checks
with the named built-in engine `Check` entry point from pinned master
`internal/d2layoutfeatures/features.go:22–92`. `main.rs` delegates to it before
layout. The original eight source cases, six engine spellings and four unknown
names are observed without layout or diagnostic normalization.

`examples/renderer-residual-scoreboard.rs` and
`scripts/renderer-residual-scoreboard.py` replay original inputs for those
checks, `d2isometric/scene_test.go:200` icon identity/deep ownership, the eight
`internal/testutil/pdf_test.go:116` PDF fill/navigation documents and the two
actual `Render` calls in `e2etests/isometric_test.go:221`. The icon request
retains authored username/password fields explicitly, because Go URL JSON
omits their private fields. Both allocation checks and the full scene compare.
PDFs compare complete bytes with the existing explicit UTC epoch 946782245 /
FPDF catalog-sort profile; all original Go page-stream, image and navigation
assertions remain active. The PDF adapter rejects parameters outside this
test's unlinked zero-pad pages, including inconsistent page maps.

The SVG CLI reuses its validated source-board traversal for recursive hash
construction (`main.rs::render_source_partial_with_board_targets`). This follows
master `d2cli/main.go:510–557` and `d2target/d2target.go:168–217,410–423`: compile
the tree before selecting boards, then hash existing target data. Direct render
calls retain source planning; board order, validation and hash bytes are unchanged.
The own-board hash input is constructed only when a recursive hash needs it;
leaf targets use the renderer's existing own-board hash instead of building and
discarding a second target serialization.

The Go e2e/PNG snapshot-maintenance and tolerance helpers remain separately
recorded infrastructure. Their passing assertions do not count as native
passes or relax the native exact raster gate. No new golden matches or
renderer algorithm fixes are inferred from this observation batch.
