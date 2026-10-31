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

type d2rsCaptionIndex struct{ *routeCaptionPlacer }

func d2rsCaptionPlacer() *d2rsCaptionIndex { return &d2rsCaptionIndex{newRouteCaptionPlacer()} }
func (p *d2rsCaptionIndex) Place(points []Vec, fraction, width, depth float64) labelSurface {
	req := map[string]any{"op": "place", "state": d2rsCaptionState(p.routeCaptionPlacer), "points": points, "fraction": d2rsCaptionNumber(fraction), "width": d2rsCaptionNumber(width), "depth": d2rsCaptionNumber(depth)}
	s := p.routeCaptionPlacer.Place(points, fraction, width, depth)
	d2rsCaptionRecord(req, map[string]any{"surface": d2rsCaptionPrint(s), "state": d2rsCaptionState(p.routeCaptionPlacer), "points": points})
	return s
}
func d2rsCaptionOverlap(a, b routeCaptionRect) bool {
	v := captionOverlap(a, b)
	d2rsCaptionRecord(map[string]any{"op": "overlap", "a": d2rsCaptionRect(a), "b": d2rsCaptionRect(b)}, v)
	return v
}
func d2rsCaptionSurface(points []Vec, c d2target.Connection, w, d, scale float64) (labelSurface, bool) {
	req := map[string]any{"op": "surface", "points": points, "connection": c, "width": w, "depth": d, "scale": scale}
	s, ok := nativeConnectionCaptionSurface(points, c, w, d, scale)
	d2rsCaptionRecord(req, map[string]any{"surface": d2rsCaptionPrint(s), "ok": ok, "points": points, "connection": c})
	return s, ok
}
func d2rsCaptionSize(style d2target.Text, icon bool, scale float64) (float64, float64) {
	w, d := nativeRouteCaptionSize(style, icon, scale)
	d2rsCaptionRecord(map[string]any{"op": "size", "style": style, "icon": icon, "scale": scale}, map[string]any{"width": w, "depth": d})
	return w, d
}
func d2rsCaptionDashes(points []Vec, lengths []float64, start, end, dash float64, budget int) [][]Vec {
	v := nativeRouteDashes(points, lengths, start, end, dash, budget)
	d2rsCaptionRecord(map[string]any{"op": "dashes", "points": points, "lengths": lengths, "start": start, "end": end, "dash": dash, "budget": budget}, v)
	return v
}
func d2rsCaptionRounded(points []Vec) []Vec {
	v := nativeRoundedRoute(points)
	d2rsCaptionRecord(map[string]any{"op": "rounded", "points": points}, v)
	return v
}
func d2rsCaptionFacet(f *classicInkFacet) any {
	return map[string]any{"points": f.points, "bias": f.bias, "keys": f.keys, "normal": f.normal}
}
func d2rsCaptionDepth(f classicInkFacet, p Vec) (float64, bool) {
	d, ok := f.depthAt(p)
	d2rsCaptionRecord(map[string]any{"op": "depth", "face": d2rsCaptionFacet(&f), "point": p}, map[string]any{"depth": d, "inside": ok})
	return d, ok
}
func d2rsCaptionSegments(ctx context.Context, n d2isometric.Node, ts []Triangle, radius ...float64) ([]classicInkSegment, error) {
	triangles := []any{}
	for _, tr := range ts {
		var material any
		if tr.Material != nil {
			material = map[string]any{"color": tr.Material.Color, "texture": tr.Material.Texture != nil}
		}
		triangles = append(triangles, map[string]any{"vertices": tr.V, "bias": tr.DepthBias, "noDepth": tr.NoDepthWrite, "shadow": tr.CastShadow, "material": material})
	}
	v, err := classicInkSegments(ctx, n, ts, radius...)
	segments := []any{}
	for _, s := range v {
		supports := []any{}
		for _, f := range s.support {
			supports = append(supports, d2rsCaptionFacet(f))
		}
		segments = append(segments, map[string]any{"a": s.a, "c": s.c, "support": supports})
	}
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsCaptionRecord(map[string]any{"op": "segments", "canceled": ctx.Err() != nil, "node": n, "triangles": triangles, "radius": radius}, map[string]any{"segments": segments, "error": message})
	return v, err
}
func d2rsCaptionFace(b *meshBuilder, vertices [3]Vertex, support []*classicInkFacet, material *Material, camera rasterCamera) {
	faces := []any{}
	for _, f := range support {
		faces = append(faces, d2rsCaptionFacet(f))
	}
	req := map[string]any{"op": "face", "vertices": vertices, "support": faces, "color": material.Color, "roughness": material.Roughness, "metalness": material.Metalness, "emissive": material.Emissive, "multiply": material.Multiply, "contour": material.svgContour, "solid": material.svgSolidTexture, "gradient": material.gradient, "unlit": material.Unlit, "camera": map[string]any{"right": camera.right, "up": camera.up, "direction": camera.direction}}
	if len(b.triangles) != 0 || b.err != nil || material.Texture != nil {
		panic("unexpected face fixture")
	}
	b.classicInkFace(vertices, support, material, camera)
	triangles := []any{}
	for _, t := range b.triangles {
		if t.Material != material || t.OpacityGroup != nil || t.ShadowGround != nil || t.PaintOwner != nil {
			panic("unexpected face ownership")
		}
		triangles = append(triangles, map[string]any{"vertices": t.V, "shadow": t.CastShadow, "noDepth": t.NoDepthWrite, "bias": t.DepthBias})
	}
	message := ""
	if b.err != nil {
		message = b.err.Error()
	}
	d2rsCaptionRecord(req, map[string]any{"triangles": triangles, "error": message})
}
