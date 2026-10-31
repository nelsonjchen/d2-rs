# Corresponding source

## Shared font metrics and lazy outlines (run 105)

At master `6a7b48378`, `lib/textmeasure/textmeasure.go:134–153` loads each
face once into the ruler's `ttfs` map; `fontface.go:30–58` parses tables
without eagerly outlining every glyph. Native `measure.rs` shares immutable
metrics and synchronized per-glyph outline cells across bundled-face callers.
Bounds still use the same ttf-parser calculation and horizontal-ink filter;
fixed-point arithmetic and iteration order are unchanged. Custom inputs own
their font bytes, retain the existing table-validation/fallback errors, and
do not enter the bundled cache. SVG and scene Markdown rulers use the shared
bundled registry, including HandDrawn style fallbacks. Exhaustive bundled
glyph comparisons and concurrent/released-input contracts guard the change.

## Master matcher allocation assertion (Lane L15)

`markdown/substitute/sparse_tests.rs` captures the original
`lib/textmeasure/substitutions_test.go:144–171`
`TestMarkdownMatcherNoMatchStorageIsSparse` at master 6a7b48378. It exercises
the production matcher from `substitutions.go:143–342`, including terminal
transition storage and the original five-run no-match allocation ceiling.
A test-only System allocator counts allocations/reallocations on the calling
thread after the original warm-up; a positive allocation control validates
the counter. Rust uses one allocation for the owned unchanged result; Go
uses zero for its borrowed string. Both must satisfy the original <=1 bound;
raw counts remain observations, with complete text and work/error results
compared exactly. Empty BTreeMaps allocate no transition storage. Production
substitution logic is unchanged.

Lane L14 (2026-10-25), master `6a7b48378`: the public
`markdown::link_safety::safe_markdown_link` maps
`lib/textmeasure/markdown_svg.go:317–326`. It trims Unicode whitespace and
uses the existing bounded danger predicate, returning the trimmed original
spelling for safe metadata or an empty string. All 34 original predicate and
safe-link results in `TestDangerousLinkPolicy` are captured with assertions
active; surrounding whitespace and original encoded bytes have native tests.

`markdown/link_safety.rs` maps master `6a7b48378`
`lib/textmeasure/markdown_svg.go:328–418`: eight bounded HTML/percent decoding
passes, scheme/control/whitespace classification and Goldmark's dangerous URL
policy (`renderer/html/html.go:933–963`). It reuses the existing Go HTML entity
decoder and preserves original safe-link bytes in compiler metadata. The
original 17 `TestDangerousLinkPolicy` inputs are retained as a native unit
contract; this does not qualify the separate Markdown SVG/link emission API.

`markdown/substitute.rs`'s complete-pattern Aho-Corasick matcher and bounded
replacement API map pinned D2 master `6a7b48378`
`lib/textmeasure/substitutions.go:23–332`. It charges one unit per candidate
match and inserted UTF-8 byte, chooses the longest original match, and skips
Markdown code spans/blocks. Compiler-side pattern preparation shares the
compilation budget (`d2ir/compile.go:799–848`). The signed-limit
`replace_substitutions_bounded_with_context` API follows context checks at
entry, matcher construction (`substitutions.go:139–213`), Markdown enter/exit
visits (`:56–98`), byte/candidate scans, selection and replacement assembly
(`:214–332`). Negative limits precede context checks; typed context causes
propagate without publishing partial text/work. The background API shares
that implementation. The compiler forwards its active context and latches
the cause (`d2ir/compile.go:605–622`).

Thirteen complete calls from seven original master
`lib/textmeasure/substitutions_test.go` functions are captured by
`scripts/language-markdown-master-scoreboard.py` and retained as
`tests/fixtures/master-markdown-substitutions.json`. The original Go assertions
remain active, including the 10,000-pattern timeout subprocess; native output,
work count, diagnostic text and typed cause compare exactly. The internal
matcher allocation-count assertion remains unqualified; these calls do not
establish allocation or general complexity parity.

This is an unpublished development checkout of d2-rs. There is not yet a public
release source URL. Keep this complete checkout, its Git revision and all local
modifications alongside any locally shared binary or WASM artifact.

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

The TrueType subsetter in `crates/d2-rs-text/src/font_subset.rs` is a Rust
translation of D2 v0.9.0's `lib/font/subsetFont.go`, which is derived from
`github.com/jung-kurt/gofpdf`. Its third-party copyright and permission notice
is retained in `THIRD_PARTY_NOTICES.md`. The exact-output tests use D2's pinned
`lib/font/subset_fingerprints_test.go` fixtures.

`crates/d2-rs-text/src/subset_font.rs` is a literal port of the same Go file
(`UTF8CutFont`), `crates/d2-rs-text/src/woff.rs` ports D2's
`lib/font/font.go` (`Sfnt2Woff`, a Go port of the MIT-licensed JavaScript
`fontello/ttf2woff`) and `lib/font/woff_compression.go`, and
`crates/d2-rs-text/src/d2fonts.rs` ports the bundled-face registry and
`Font.GetEncodedSubset` of `d2renderers/d2fonts`. WOFF table compression uses
the `d2-rs-goflate` port of Go's `compress/zlib`. Their tests
(`tests/font_subset_oracle.rs`) use digests produced by
`tools/font-subset-oracle` against the pinned D2 source and every embedded
font in D2's `e2etests/testdata` goldens.

Before publication, supply a verified accessible source URL and exact source
revision for every artifact. Include LICENSE, THIRD_PARTY_NOTICES.md,
the repository root's DEPENDENCY_NOTICES.md and this file with every binary/WASM distribution and
crate. Source recipients must receive the MPL-covered modifications needed to
build their version. Third-party material retains its applicable terms.

Code token/rendering translation: original Go D2 v0.9.0 `d2scenebuild/code_shape.go` and `d2svg.go` language branch; pinned Chroma v2.27.0 `lexers/go.go`, `lexers/embedded/go_template.xml`, `lexers/embedded/{python,yaml,javascript,json}.xml`, `lexers/markdown.go`, `regexp.go`, `mutators.go`, `iterator.go`, `style.go`, and `styles/{github,catppuccin-mocha}.xml`. Native Go/template, Python, Markdown/YAML and JavaScript/JSON rules and two style tables are implemented; full lexer registry, resources/validation/cancellation, and scene API qualification remain open.

Markdown/ordinary ruler continuation487 translates original textmeasure.go scaleUnicodeLegacy, scaleUnicode/controlRune, atlas.DrawRune and exact width bounds. Go Unicode17 default-ignorable classification is exhaustively generated into markdown_ignorable_ranges.rs. All360raw/legacy and360ordinary f64 width controls match; ordinary tab remainder/atlas-origin arithmetic is retained from Source. Evidence: .reference/positioned-tooltip-20261006/paint-source-comparison.md.
Checkpoint 491 corrects tight-list HTML projection and trailing blank-line
handling from pinned Goldmark v1.8.5 `renderer/html/html.go:renderTextBlock`
and `parser/list_item.go:Continue`. TextBlock content has no paragraph wrapper;
continuing across a blank does not rewrite the current item's preceding-blank
flag. All 72 original-Go paragraph/list HTML projections are checked alongside
their complete paint primitives in the renderer's Source regression fixture.
Evidence: `.reference/markdown-flow-20261007/source-comparison.md`.

Checkpoint493 adds a lexical translation of pinned golang.org/x/net v0.57.0 html/token.go TagName/TagAttr ASCII name folding in markdown/html_names.rs. Text and attribute values retain their bytes; raw Goldmark projection remains unchanged. This is not full HTML tree/tokenizer compatibility. BSD3 notice/license are retained in THIRD_PARTY_NOTICES.md and the root notice. The96 Source public-layout controls cover casing, preserved link values and actual unsupported-element errors.

Checkpoint494 translates pinned Go1.27 encoding/xml/xml.go rawToken/name/nsname/text/attrval/popElement/procInst admission used by original compileLabel/tooltip. xml.rs preserves SyntaxError.Msg, error priority, raw-prefix matching and interface-value first-root stopping, backed by121 complete original-Go controls. go_xml_names.rs and go_quote.rs retain the repository Source XML-name ranges and strconv.Quote/Unicode17 translation. html.rs corrects Goldmark v1.8.5 NUL escaping and table extension tbody/newline/XHTML alignment projection; all Markdown XML controls compare complete source HTML. This is not a general XML/namespace/charset DOM or full Markdown compatibility claim. Evidence: .reference/markdown-compiler-xml-20261007/source-comparison.md.

Checkpoint546 (2026-10-08): Original Goldmark v1.8.5 renderer/html/html.go byte writers and util.URLEscape/reference stages map to native byte HTML output. Go encoding/xml complete-token UTF-8/error priority is retained; all128 actual byte/HTML/XML contracts are permanent. Display strings remain separate from compiler validation. Full Markdown/sanitization/XML/API semantics remain open. Evidence: .reference/markdown-bytes-native-20261008/source-comparison.md and api-qualification.json.

Checkpoint548 native uniseg.rs maps original uniseg v0.4.7 line.go/linerules.go/graphemerules.go/grapheme.go/width.go/properties.go. Unicode15 ordered properties/transition payloads and Go1.27 Unicode17 mark/symbol categories are generated by scripts/generate-uniseg-boundaries.py from pinned references. Valid-UTF8 byte boundary/width controls remain distinct from malformed-byte APIs. measure_markdown_css_advance maps original textmeasure.inlineTextAdvance/scaleUnicodeCSS, keeping supported-font advances separate from glyph fallback. Complete source controls and limitations: .reference/markdown-wrap-native-20261008. MIT, Go BSD3 and Unicode notices are retained in licenses/.

Checkpoint548: valid-UTF8 Unicode15 line/grapheme boundaries and StringWidth are native in uniseg.rs/uniseg_data.rs, generated from pinned rivo/uniseg v0.4.7. Go1.27 symbol/mark categories support original CSS fallback advance. All16652 complete source controls match; source requests and complete segmented outputs remain in .reference/markdown-wrap-native-20261008. This does not qualify malformed UTF8 or full Unicode shaping.

Native code work controls in `src/code/control.rs` translate original D2 v0.9.0 `d2renderers/d2scenebuild/code_shape.go` collector/piece-budget/cancellation gates under MPL-2.0. Original Chroma MIT rule/style notices remain independent.

Registry lookup mirrors Chroma2.27 `registry.go` and `lexer.go`, using captured original global Config definitions in `assets/chroma/registry-config.json`. Names/aliases, Unix filename glob syntax, suffixes and priority sorting are native; this data is registry configuration, not fixture outputs. Windows filename boundaries remain unqualified.

Bash (`sh`, `bash`, `zsh`, `ksh`, `shell`) executes pinned Chroma2.27 `lexers/embedded/bash.xml` (byte-identical copy in `assets/chroma/bash.xml`) through `src/code/xml.rs`, which now also translates `emitters.go` ByGroups and regexp2 `\w`/`\W`/`\d`/`\b` and literal-in-class characters escape-aware. `src/code/chroma_style.rs` maps `types.go` TokenType numbering/Category/SubCategory, `style.go` ParseStyleEntry/Inherit/Style.Get/synthesise and `formatters/svg` StyleEntryToSVG for d2svg `styleToSVG`. Evidence: `tests/fixtures/chroma-bash-styles-v227.json` from `tools/chroma-code-oracle` (34 Bash streams, registry names, both complete style maps).

Run 29: `d2fonts::CustomFonts` follows pinned `d2fonts_common.go:38–61`
(`GetEncodedSubset`) for supplied face bytes, and `AddFontFamily:129–209`
for omitted-style SourceSansPro fallbacks in both custom families. Custom
families are per-render values; they do not mutate the bundled registry.
The existing native UTF8CutFont/Sfnt2Woff pipeline supplies the actual bytes.

Run 36: `subset_font::utf8_cut_font_fpdf` selects the format-4 cmap length
formula of pinned `codeberg.org/go-pdf/fpdf v0.12.0/utf8fontfile.go:608` for
PDF embedding. D2's lib/font helper corrected that formula for SVG; its
existing default remains unchanged. The shared glyph/table construction is
otherwise identical. Complete PDF font fixtures live in d2-rs-export.

Run 73 (master 6a7b48378): `d2fonts::{add_font_family,is_registered_family,
registered_encoded_subset}` follows `d2fonts_common.go:38–61,116–209` and
`d2target/render_validation.go:264–277`. Named registrations retain all four
faces; omitted styles use SourceSansPro, and supplied styles use the existing
native UTF8CutFont/WOFF encoder. Raw SVG validation and embedding share this
registry. Original witness: d2svg `TestRenderAcceptsRegisteredCustomFontFamily`;
complete call capture and exact document fixture are retained in
`svg-font-api.json` / `scripts/svg-font-scoreboard.py`. Built-in enum faces
and per-render `CustomFonts` remain their existing separate APIs.

Lane L36: `registered_font_face` exposes an owned snapshot of
`d2fonts.FontFaces.Lookup` (`d2fonts_common.go:116–209` registrations), with
the bundled registry as its fallback. Isometric plain text and scenebuild
font assets/Markdown metrics now consume the same explicit named bytes as
SVG. The original configured-font and explicit-fallback tests qualify six
complete observations in `scripts/isometric-font-scoreboard.py`.
