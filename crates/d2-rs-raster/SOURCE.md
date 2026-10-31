# d2-rs-raster source mapping

Native port of `d2renderers/d2raster` at D2 master `6a7b48378`. The public
scene input is `d2-rs-scene`; font parsing, shaping and color-plan validation
are shared with `d2-rs-fontface`. No Go or browser executes on native paths.

| Go source | Rust module |
| --- | --- |
| `raster.go`, `bands.go` | `raster.rs`: `FrameOptions`, preparation/resource limits, `render`, `render_bands`, primitive painting |
| `geometry.go` | `geometry.rs`: adaptive flattening, fills, stroke runs, cap/join geometry and bounds |
| `internal/scanline/rasterizer.go` | `scanline.rs`: coverage rasterizer and work accounting |
| `raster.go` frame buffer helpers | `pixels.rs`: NRGBA/RGBA storage, rectangles and conversion |
| `effects.go`, `mask_regions.go`, `stroke_regions.go`, `bands.go` | `effects.rs`, `regions.rs`: layers, masks, blend operations, resource planning and band reference bounds |
| `filter.go` and filter helpers | `filter.rs`: blur and shadows |
| `gradient.go`, `pattern.go`, `color_space.go` | `paint.rs`: gradient preparation/sampling, pattern tiles, sRGB/linear-light interpolation |
| `image.go` | `image.rs`: raster asset validation, PNG/JPEG/GIF/WebP decoding, placement and interpolation |
| `internal/rasterimage/firstframe.go`, `jpeg_orientation.go`; Go 1.27 `image/png/writer.go`, `bufio/bufio.go` | `first_frame.rs`, `png.rs`: shared authored-asset normalization and default PNG encoding; JPEG IDCT/EXIF reuse the existing native core port |
| `animation*.go` | `animation.rs`: renderer animation validation and interpolation |
| `vector_validation.go` | `vector_validation.rs`: retained vector validation and exact rational bounds checks |
| `text.go` | `text.rs`: lazy prepared fonts, explicit and raw shaped glyphs, anchors, decorations, COLRv0 layers, user/foreground/palette paints and color-text bounds |
| `colrv1.go` | `colrv1.rs`: authenticated color plans → prepared tree; transforms, glyph outlines, root clips, linear/radial gradients, SrcIn masks and isolated SoftLight groups |

Run 46 replaces the interim raster-only sfnt reader with the shared fontface
reader. Prepared bundled fonts share immutable outlines and create their
render-local shaping face only for raw text. Immutable COLRv1 plans (including
negative/error lookups) are cached per render, but every occurrence is charged
against node/path limits (`text.go:257–338`, `colrv1.go:27–65`). Palette alpha
and foreground/user-paint behavior follow `preparedTextPartPaint`; color layers
do not inherit the user's outline stroke (`text.go:421–458`).

`fma.rs` documents the pinned Go ARM64 matrix sites. The new gradient sites
were checked against Go 1.27 ARM64 assembly: `colrv1.go:272,276` fuse the first
product, `:277` uses fused subtraction, and `:278` fuses the second product.
Extracted instructions: `.reference/codex-runs/run46-colrv1-instructions.txt`.
Rebuild with the absolute shared Go caches and
`go -C ../d2-trace build -gcflags=github.com/d2lang/d2/d2renderers/d2raster=-S ./d2renderers/d2raster`.

## Evidence and limits

`scene-scoreboard.py --tier raster` scores the upstream e2e scenes against
Go pixels. `raster-api-scoreboard.py` additionally runs six original upstream
text/gradient/emoji tests, retaining ten actual Render requests and outcomes.
The original assertions must pass (including Go's emoji PNG hashes); native
outputs compare complete decoded RGBA bytes or exact error text. The API
gate retains alpha and RGB under zero alpha, rejects missing/corrupt/truncated
PNG files, and checks complete output inventories. Captures are bounded.

These are scoped contracts, not full raster API parity. `RenderSession`,
D2's BestSpeed PNG/GIF encoding APIs, and a caller-supplied cancellation
context are not exposed by this crate. Existing CLI export encoders remain separate.
Fontface's documented CFF/dfont/parser limitations apply. COLRv0 rendering is
source-translated but is not exercised by the current upstream capture.
The new pixel tier exercises bundled Noto COLRv1, not arbitrary untrusted
COLRv1 fonts (which fontface rejects according to the source contract).

Source remains MPL-2.0; the shared fontface sfnt translation retains its
original BSD notices. See root `THIRD_PARTY_NOTICES.md` and fontface's notices.

Run 47 adds `normalize_first_frame_png(data, format)` and
`encode_png_default(frame)` for Lane I's `vector_surface_svg.go:455–496`
adapter request. The former enforces that caller's 16-million-pixel and
32-MiB encoded-output limits. The latter is Go's default `png.Encode`, not
`d2raster.EncodePNG`'s BestSpeed profile; it handles NRGBA stride and nonzero
origins. Palette bit depth, transparency, Gray16/RGB16, filter tie order,
zlib level and Go's 32-KiB buffered IDAT write boundaries are preserved.
The first-frame port validates GIF/PNG/WebP containers, excludes APNG poster
images, materializes only the first animation frame, and places partial
frames on the logical canvas. GIF's logicalImage wrapper keeps its palette
ColorModel but lacks ColorIndexAt, selecting PNG16 (`firstframe.go:95–114`,
Go `writer.go:627–663`). JPEG uses `d2_rs_core::jpeg_first`, retaining exact
16-bit color conversion and EXIF orientation. Lossy WebP uses the native
VP8 planes and Go's integer YCbCr conversion, not the decoder's RGB shortcut.
No new floating-point contraction sites are introduced.

`scripts/raster-image-scoreboard.py` captures 23 DecodeFirst calls from eight
original upstream tests, with their assertions intact, then compares complete
PNG bytes at the isometric writer's actual normalization boundary. Seven
retained static-PNG model witnesses and two padded/origin-shifted NRGBA frame
encodings also match Go. This does not qualify arbitrary malformed assets or
cancellation; the renderer's existing private decoder is unchanged. Lane I
can replace its local adapter with this API; this run does not edit isometric.
The encoder/filter translations retain the Go BSD-3-Clause notices above.
