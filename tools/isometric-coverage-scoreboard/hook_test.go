// SPDX-License-Identifier: MPL-2.0
// Input-only observers; installed solely in the pinned trace checkout.
package d2isometricimg

import (
	"context"
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2isometric"
	"image"
	"math"
	"os"
	"path/filepath"
	"testing"
)

var coverageRows []any
var coverageWriters map[*nativeSVGWriter]int
var coverageSurfaces map[*nativeVectorSurface]int

func d2rsGeometryStart(t *testing.T) {
	coverageRows = []any{}
	coverageWriters = map[*nativeSVGWriter]int{}
	coverageSurfaces = map[*nativeVectorSurface]int{}
	contentAssetStore = filepath.Join(os.Getenv("D2RS_SCENE_API"), "..", "assets")
	t.Cleanup(func() {
		b, e := json.Marshal(map[string]any{"name": t.Name(), "operations": coverageRows})
		if e != nil {
			t.Fatal(e)
		}
		if e = os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), b, 0600); e != nil {
			t.Fatal(e)
		}
	})
}
func coverageRecord(r, g any) {
	b, e := json.Marshal(map[string]any{"request": r, "got": g})
	if e != nil {
		panic(e)
	}
	coverageRows = append(coverageRows, json.RawMessage(b))
}
func coverageError(e error) string {
	if e != nil {
		return e.Error()
	}
	return ""
}
func coveragePoints(ps []svgPoint) any {
	out := []any{}
	for _, p := range ps {
		out = append(out, []float64{p.x, p.y, p.z})
	}
	return out
}
func coveragePolygons(ps [][]svgPoint) any {
	out := []any{}
	for _, p := range ps {
		out = append(out, coveragePoints(p))
	}
	return out
}
func coverageMesh(ts []Triangle, raw bool) any {
	// Preserve nonfinite receiver inputs without changing their semantic values.
	safe := append([]Triangle(nil), ts...)
	grounds := []any{}
	surfaces := []any{}
	for i, t := range ts {
		safe[i].ShadowGround = nil
		var g any
		if t.ShadowGround != nil {
			g = d2rsGeometryNumber(*t.ShadowGround)
		}
		grounds = append(grounds, g)
		var s any
		if t.Material != nil && t.Material.Vector != nil {
			if raw {
				s = contentSurface(t.Material.Vector)
			} else {
				s = true
			}
		} else if !raw {
			s = false
		}
		surfaces = append(surfaces, s)
	}
	return map[string]any{"mesh": solidMesh(safe, raw), "grounds": grounds, "surfaces": surfaces}
}
func coverageGround(ts []Triangle, g float64, v Vec) []Triangle {
	r := map[string]any{"op": "ground", "mesh": coverageMesh(ts, true), "ground": g, "view": v}
	out := rasterGroundTriangles(ts, g, v)
	coverageRecord(r, map[string]any{"mesh": coverageMesh(out, false), "source": coverageMesh(ts, false)})
	return out
}
func coverageReceivers(ts []Triangle, bs []d2isometric.Board, spans []hierarchyShadowSpan) {
	ss := []any{}
	for _, s := range spans {
		ss = append(ss, map[string]any{"first": s.first, "last": s.last, "board": s.board})
	}
	r := map[string]any{"op": "receivers", "mesh": coverageMesh(ts, true), "boards": bs, "spans": ss}
	hierarchyShadowReceivers(ts, bs, spans)
	coverageRecord(r, map[string]any{"mesh": coverageMesh(ts, false)})
}
func coverageSnapshot(ctx context.Context, ts []Triangle) ([]Triangle, error) {
	r := map[string]any{"op": "snapshot", "mesh": coverageMesh(ts, true)}
	out, e := rasterSnapshot(ctx, ts)
	coverageRecord(r, map[string]any{"mesh": coverageMesh(out, false), "source": coverageMesh(ts, false), "error": coverageError(e)})
	return out, e
}
func coverageValidate(ctx context.Context, ts []Triangle) error {
	r := map[string]any{"op": "validate", "mesh": coverageMesh(ts, true)}
	e := rasterValidate(ctx, ts)
	coverageRecord(r, map[string]any{"error": coverageError(e)})
	return e
}
func coverageExpand(p []svgPoint, w float64) []svgPoint {
	out := svgExpandCoverPolygon(p, w)
	coverageRecord(map[string]any{"op": "expand", "points": coveragePoints(p), "width": w}, coveragePoints(out))
	return out
}
func coverageBatches(bs []*svgPaintBatch, raw bool) any {
	ts := []Triangle{}
	out := []any{}
	for _, b := range bs {
		ts = append(ts, b.triangle)
		out = append(out, map[string]any{"polygons": coveragePolygons(b.polygons), "points": coveragePoints(b.points[:]), "depth": b.depth, "first": b.first, "affine": b.affine, "texture": b.texture, "color": b.color})
	}
	return map[string]any{"mesh": coverageMesh(ts, raw), "batches": out}
}
func coveragePrepare(bs []*svgPaintBatch) map[*svgPaintBatch]*svgPaintCover {
	r := map[string]any{"op": "prepare", "batches": coverageBatches(bs, true)}
	got := svgPreparePaintCovers(bs)
	ids := map[*svgPaintCover]int{}
	out := []int{}
	for _, b := range bs {
		n := 0
		if c := got[b]; c != nil {
			if ids[c] == 0 {
				ids[c] = len(ids) + 1
			}
			n = ids[c]
		}
		out = append(out, n)
	}
	coverageRecord(r, map[string]any{"covers": out, "source": coverageBatches(bs, false)})
	return got
}
func coverageWrite(w *nativeSVGWriter, c *svgPaintCover, cam rasterCamera) {
	r := map[string]any{"op": "cover", "batches": coverageBatches(c.batches, true), "written": c.written, "prefix": w.buf.String(), "camera": coverageCamera(cam)}
	writeSVGPaintCover(w, c, cam)
	coverageRecord(r, map[string]any{"svg": w.buf.String(), "error": coverageError(w.err), "written": c.written, "source": coverageBatches(c.batches, false)})
}
func coverageShadow(t *testing.T, s *nativeScene) *image.NRGBA {
	w := &nativeSVGWriter{ctx: context.Background()}
	r := map[string]any{"op": "shadow", "mesh": coverageMesh(s.triangles, true), "camera": coverageCamera(s.camera), "width": s.width, "height": s.height}
	writeSVGGroundShadow(w, s)
	coverageRecord(r, map[string]any{"svg": w.buf.String(), "error": coverageError(w.err)})
	return rasterSVGShadowCoverage(t, s)
}
func coverageVisibility(ctx context.Context, fs []svgVisibilityFace) ([][][]svgPoint, error) {
	ids := map[*nativeOpacityGroup]int{}
	owners := map[*nativePaintOwner]int{}
	faces := []any{}
	for _, f := range fs {
		g, o := 0, 0
		if f.group != nil {
			if ids[f.group] == 0 {
				ids[f.group] = len(ids) + 1
			}
			g = ids[f.group]
		}
		if f.owner != nil {
			if owners[f.owner] == 0 {
				owners[f.owner] = len(owners) + 1
			}
			o = owners[f.owner]
		}
		faces = append(faces, map[string]any{"points": coveragePoints(f.points), "opaque": f.opaque, "order": f.order, "group": g, "owner": o, "contour": f.contour})
	}
	out, e := svgVisibleFaces(ctx, fs)
	v := []any{}
	for _, ps := range out {
		v = append(v, coveragePolygons(ps))
	}
	coverageRecord(map[string]any{"op": "visibility", "faces": faces}, map[string]any{"visible": v, "error": coverageError(e)})
	return out, e
}
func coverageProject(c rasterCamera, p Vec) rasterPoint {
	out := c.project(p)
	coverageRecord(map[string]any{"op": "project", "camera": coverageCamera(c), "point": p}, []float64{out.x, out.y, out.z})
	return out
}
func coverageLinks(w *nativeSVGWriter, s *nativeScene) {
	ls := []any{}
	for _, l := range s.links {
		ls = append(ls, map[string]any{"points": l.points, "region": l.region})
	}
	writeSVGLinks(w, s)
	coverageRecord(map[string]any{"op": "links", "camera": coverageCamera(s.camera), "links": ls}, map[string]any{"svg": w.buf.String(), "error": coverageError(w.err)})
}
func coverageScene(ctx context.Context, s *nativeScene) ([]byte, error) {
	r := map[string]any{"op": "scene", "mesh": coverageMesh(s.triangles, true), "camera": coverageCamera(s.camera), "width": s.width, "height": s.height, "background": s.background}
	out, e := nativeSceneSVG(ctx, s)
	coverageRecord(r, map[string]any{"svg": string(out), "error": coverageError(e)})
	return out, e
}
func coverageDefinition(w *nativeSVGWriter, s *nativeVectorSurface) string {
	if coverageWriters[w] == 0 {
		coverageWriters[w] = len(coverageWriters) + 1
	}
	var doc any
	if coverageSurfaces[s] == 0 {
		coverageSurfaces[s] = len(coverageSurfaces) + 1
		doc = contentSurface(s)
	}
	r := map[string]any{"op": "definition", "writer": coverageWriters[w], "surface": coverageSurfaces[s], "document": doc}
	id := w.surfaceDefinition(s)
	coverageRecord(r, map[string]any{"id": id, "svg": w.buf.String(), "error": coverageError(w.err)})
	return id
}

func d2rsGeometryNumber(x float64) any {
	if math.IsNaN(x) {
		return "NaN"
	}
	if math.IsInf(x, 1) {
		return "+Inf"
	}
	if math.IsInf(x, -1) {
		return "-Inf"
	}
	return x
}
func coverageCamera(c rasterCamera) any {
	return map[string]any{"right": c.right, "up": c.up, "direction": c.direction, "centerX": c.centerX, "centerY": c.centerY, "centerDepth": c.centerDepth, "scale": c.scale, "width": c.width, "height": c.height}
}
func contentSurface(s *nativeVectorSurface) any {
	if s == nil {
		return nil
	}
	return map[string]any{"document": contentEncDocument(s.document), "coverage": contentSurface(s.inkCoverage), "opacity": s.coverageOpacity, "cap": s.capBackground, "paint": contentEncPaint(s.capPaint)}
}

func coverageMarkdown(n d2isometric.Node) bool {
	out := nativeMarkdownCard(n)
	coverageRecord(map[string]any{"op": "markdown", "node": n}, out)
	return out
}
