# d2-rs-scene source mapping

Port of D2 master 6a7b48378 `d2renderers/d2scene`.

| Go | Rust |
|---|---|
| geometry.go Point, Box, Bounds, Matrix | `src/geometry.rs` (math.Min/Max semantics in go_min/go_max) |
| path.go PathCommand, Path, bounds, EndpointArcToCenter | `src/path.rs` |
| paint.go Paint, gradients, PatternPaint, Stroke | `src/paint.rs` |
| scene.go, primitive.go, animation.go | `src/scene.rs` |
| aspect.go AspectRatioMatrix | `src/aspect.rs` |

ARM64 contraction sites: `.reference/isometric-port/fma-sites-foundations.txt`
(geometry.go:214–272, path.go:222–431). Go math (sin/cos/atan2/hypot/frexp/
ldexp/tan) comes from `d2_rs_render::go_float::gomath`; `tan` was added there
from the d2-rs-sketch translation.

`Matrix::{mul,point,vector}` fuse the first product, with the second product
rounded separately and translation added afterward, as verified from the
pinned Go 1.27 ARM64 instructions at geometry.go:214–234. The cross-arrowhead
center regression compares the exact output bits of independent Go API call
sites; shared/inlined Go expressions can reuse earlier rounded SSA products.
Run-45 evidence: `.reference/codex-runs/run45-matrix-{instructions.txt,go.json}`.

JSON interchange: Go encoding/json field names, uint8 enums as numbers,
interfaces tagged by Go type name (`tools/scene-probe/scenejson.go`), fonts by
content-addressed `DataFile` in probe captures. `scripts/scene-scoreboard.py
--tier model`: 897/897 Go documents from the e2e goldens round-trip exactly.
