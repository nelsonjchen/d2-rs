// SPDX-License-Identifier: MPL-2.0
package d2isometricimg

import (
	"context"
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2target"
	"math"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

var d2rsGeometryRows []any

func d2rsGeometryStart(t *testing.T) {
	d2rsGeometryRows = []any{}
	t.Cleanup(func() {
		data, err := json.Marshal(map[string]any{"name": t.Name(), "operations": d2rsGeometryRows})
		if err != nil {
			t.Fatal(err)
		}
		if err = os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	})
}
func d2rsCaptionRecord(request, got any) {
	data, err := json.Marshal(map[string]any{"request": request, "got": got})
	if err != nil {
		panic(err)
	}
	d2rsGeometryRows = append(d2rsGeometryRows, json.RawMessage(data))
}
func d2rsCaptionNumber(v float64) any {
	if math.IsNaN(v) {
		return "NaN"
	}
	if math.IsInf(v, 1) {
		return "+Inf"
	}
	if math.IsInf(v, -1) {
		return "-Inf"
	}
	return v
}
func d2rsCaptionPrint(s labelSurface) any {
	return map[string]any{"center": s.center, "width": s.width, "depth": s.depth, "angle": s.angle, "align": s.align}
}
func d2rsCaptionRect(r routeCaptionRect) any {
	hull := [][2]float64{}
	for _, p := range r.hull {
		hull = append(hull, [2]float64{p.x, p.z})
	}
	return map[string]any{"x": r.x, "z": r.z, "ux": r.ux, "uz": r.uz, "hw": r.hw, "hd": r.hd, "left": r.left, "right": r.right, "front": r.front, "back": r.back, "hard": r.hard, "hull": hull}
}
func d2rsCaptionState(p *routeCaptionPlacer) any {
	runs := []any{}
	for i := 0; i < len(p.rects); {
		j := i + 1
		for j < len(p.rects) && reflect.DeepEqual(p.rects[i], p.rects[j]) {
			j++
		}
		runs = append(runs, map[string]any{"rect": d2rsCaptionRect(p.rects[i]), "count": j - i})
		i = j
	}
	return map[string]any{"rects": runs, "work": p.work, "refs": p.refs}
}

func d2rsContactVec(p Vec) any {
	return []any{d2rsCaptionNumber(p.X), d2rsCaptionNumber(p.Y), d2rsCaptionNumber(p.Z)}
}
func d2rsContactResult(p Vec, m bool) any {
	return map[string]any{"point": d2rsContactVec(p), "moved": m}
}
func d2rsContactQueue(n d2isometric.Node, p, d Vec) (Vec, bool) {
	v, m := nativeQueueContact(n, p, d)
	d2rsCaptionRecord(map[string]any{"op": "queue", "node": n, "point": d2rsContactVec(p), "direction": d2rsContactVec(d)}, d2rsContactResult(v, m))
	return v, m
}
func d2rsContactUpright(n d2isometric.Node, p, d Vec) (Vec, bool) {
	v, m := nativeUprightContact(n, p, d)
	d2rsCaptionRecord(map[string]any{"op": "upright", "node": n, "point": d2rsContactVec(p), "direction": d2rsContactVec(d)}, d2rsContactResult(v, m))
	return v, m
}
func d2rsContactCrown(n d2isometric.Node) float64 {
	v := nativeQueueCrown(n)
	d2rsCaptionRecord(map[string]any{"op": "crown", "node": n}, v)
	return v
}
func d2rsContactContext(ctx context.Context) string {
	if ctx == nil {
		return "nil"
	}
	if ctx.Err() != nil {
		return ctx.Err().Error()
	}
	return "active"
}
func d2rsContactError(err error) string {
	if err != nil {
		return err.Error()
	}
	return ""
}
func d2rsContactRoutes(ctx context.Context, es []d2isometric.Edge, ns []d2isometric.Node, paths [][]Vec, support ...map[string]float64) ([][]Vec, error) {
	req := map[string]any{"op": "contacts", "context": d2rsContactContext(ctx), "edges": es, "nodes": ns, "paths": paths, "support": support}
	data, _ := json.Marshal(req)
	v, e := nativeSolidContactRoutes(ctx, es, ns, paths, support...)
	d2rsCaptionRecord(json.RawMessage(data), map[string]any{"paths": v, "error": d2rsContactError(e), "edges": es, "nodes": ns, "sourcePaths": paths, "support": support})
	return v, e
}
func d2rsContactPipeline(ctx context.Context, es []d2isometric.Edge, ns []d2isometric.Node, bs []d2isometric.Board, support ...map[string]float64) ([][]Vec, [][]Vec, error) {
	req := map[string]any{"op": "pipeline", "context": d2rsContactContext(ctx), "edges": es, "nodes": ns, "boards": bs, "support": support}
	data, _ := json.Marshal(req)
	lanes, paths, e := nativeEdgeRoutes(ctx, es, ns, bs, support...)
	d2rsCaptionRecord(json.RawMessage(data), map[string]any{"paths": paths, "lanes": lanes, "error": d2rsContactError(e), "edges": es, "nodes": ns, "boards": bs, "support": support})
	return lanes, paths, e
}

// Material sharing is an input to projected surface grouping.
func d2rsContactProjection(ts []Triangle) any {
	ms := []any{}
	tris := []any{}
	ids := map[*Material]int{}
	for _, t := range ts {
		id := -1
		if t.Material != nil {
			var ok bool
			id, ok = ids[t.Material]
			if !ok {
				id = len(ms)
				ids[t.Material] = id
				ms = append(ms, map[string]any{"color": t.Material.Color, "texture": t.Material.Texture != nil})
			}
		}
		tris = append(tris, map[string]any{"vertices": t.V, "material": id})
	}
	return map[string]any{"materials": ms, "triangles": tris}
}

type d2rsContactIndex struct{ *routeCaptionPlacer }

func d2rsContactPlacer() *d2rsContactIndex { return &d2rsContactIndex{newRouteCaptionPlacer()} }
func (p *d2rsContactIndex) Place(points []Vec, f, w, d float64) labelSurface {
	req := map[string]any{"op": "place", "state": d2rsCaptionState(p.routeCaptionPlacer), "points": points, "fraction": f, "width": w, "depth": d}
	s := p.routeCaptionPlacer.Place(points, f, w, d)
	d2rsCaptionRecord(req, map[string]any{"surface": d2rsCaptionPrint(s), "state": d2rsCaptionState(p.routeCaptionPlacer), "points": points})
	return s
}
func (p *d2rsContactIndex) Avoid(center Vec, w, d float64) {
	req := map[string]any{"op": "avoid", "state": d2rsCaptionState(p.routeCaptionPlacer), "center": center, "width": w, "depth": d}
	p.routeCaptionPlacer.Avoid(center, w, d)
	d2rsCaptionRecord(req, d2rsCaptionState(p.routeCaptionPlacer))
}
func (p *d2rsContactIndex) AvoidMesh(ts []Triangle) {
	req := map[string]any{"op": "avoidMesh", "state": d2rsCaptionState(p.routeCaptionPlacer), "mesh": d2rsContactProjection(ts)}
	p.routeCaptionPlacer.AvoidMesh(ts)
	d2rsCaptionRecord(req, d2rsCaptionState(p.routeCaptionPlacer))
}
func (p *d2rsContactIndex) AvoidRoute(points []Vec, r float64) {
	req := map[string]any{"op": "avoidRoute", "state": d2rsCaptionState(p.routeCaptionPlacer), "points": points, "radius": r}
	p.routeCaptionPlacer.AvoidRoute(points, r)
	d2rsCaptionRecord(req, map[string]any{"state": d2rsCaptionState(p.routeCaptionPlacer), "points": points})
}
func d2rsContactScore(p *d2rsContactIndex, r routeCaptionRect) (float64, bool) {
	req := map[string]any{"op": "score", "state": d2rsCaptionState(p.routeCaptionPlacer), "rect": d2rsCaptionRect(r)}
	s, c := p.routeCaptionPlacer.score(r)
	d2rsCaptionRecord(req, map[string]any{"score": s, "complete": c, "state": d2rsCaptionState(p.routeCaptionPlacer)})
	return s, c
}
func d2rsContactFloor(bs []d2isometric.Board, ns map[string]*d2isometric.Node) float64 {
	v := hierarchyCasingFloor(bs, ns)
	d2rsCaptionRecord(map[string]any{"op": "floor", "boards": bs, "nodes": ns}, v)
	return v
}
func d2rsContactFaceSource(n d2isometric.Node, fill string) d2target.Shape {
	v := nativeFaceSource(n, fill)
	d2rsCaptionRecord(map[string]any{"op": "faceSource", "node": n, "fill": fill}, map[string]any{"shape": v, "node": n})
	return v
}
func d2rsContactClassic(n d2isometric.Node) d2isometric.Node {
	v := nativeClassicNode(n)
	d2rsCaptionRecord(map[string]any{"op": "classic", "node": n}, map[string]any{"styled": v, "node": n})
	return v
}
func d2rsContactProfile(s d2target.Shape) ([]Vec, error) {
	v, e := nativeShapeProfile(s)
	d2rsCaptionRecord(map[string]any{"op": "profile", "shape": s}, map[string]any{"profile": v, "shape": s, "error": d2rsContactError(e)})
	return v, e
}
func d2rsContactMaterial(m *Material) any {
	if m == nil {
		return nil
	}
	if m.Texture != nil || m.Vector != nil {
		panic("unexpected retained surface")
	}
	return map[string]any{"color": m.Color, "roughness": m.Roughness, "metalness": m.Metalness, "emissive": m.Emissive, "unlit": m.Unlit, "multiply": m.Multiply, "contour": m.svgContour, "solid": m.svgSolidTexture, "gradient": m.gradient}
}
func d2rsContactTriangles(ts []Triangle) any {
	out := []any{}
	for _, t := range ts {
		if t.OpacityGroup != nil || t.PaintOwner != nil || t.ShadowGround != nil || t.ShadowOpacity != nil || t.ShadowFillAlpha != nil || t.svgCoverageOnly {
			panic("unexpected triangle metadata")
		}
		out = append(out, map[string]any{"vertices": t.V, "material": d2rsContactMaterial(t.Material), "bias": t.DepthBias, "shadow": t.CastShadow, "noDepth": t.NoDepthWrite})
	}
	return out
}
func d2rsContactBuilder(b *meshBuilder) map[string]any {
	return map[string]any{"triangles": d2rsContactTriangles(b.triangles), "error": d2rsContactError(b.err), "floor": b.routeCasingFloor}
}
func d2rsContactInk(b *meshBuilder, ps []Vec, r float64, m *Material) {
	req := map[string]any{"op": "ink", "builder": d2rsContactBuilder(b), "points": ps, "radius": r, "material": d2rsContactMaterial(m)}
	b.routeInk(ps, r, m)
	d2rsCaptionRecord(req, map[string]any{"builder": d2rsContactBuilder(b), "points": ps})
}
func d2rsContactCasing(b *meshBuilder, ps []Vec, r, o float64) {
	req := map[string]any{"op": "casing", "builder": d2rsContactBuilder(b), "points": ps, "radius": r, "opacity": o}
	b.routeCasing(ps, r, o)
	d2rsCaptionRecord(req, map[string]any{"builder": d2rsContactBuilder(b), "points": ps})
}
func d2rsContactExtrude(b *meshBuilder, ps []Vec, base float64, top, side *Material) {
	req := map[string]any{"op": "extrude", "builder": d2rsContactBuilder(b), "points": ps, "base": base, "top": d2rsContactMaterial(top), "side": d2rsContactMaterial(side)}
	b.extrudedProfile(ps, base, top, side)
	d2rsCaptionRecord(req, map[string]any{"builder": d2rsContactBuilder(b), "points": ps})
}
