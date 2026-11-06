// SPDX-License-Identifier: MPL-2.0
// Original-test observation; installed only in the pinned trace checkout.
package d2isometricimg

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/d2lang/d2/d2renderers/d2isometric"
	"image"
	"image/color"
	"os"
	"path/filepath"
	"reflect"
	"testing"
)

var solidRows []any

func d2rsGeometryStart(t *testing.T) {
	solidRows = []any{}
	t.Cleanup(func() {
		data, err := json.Marshal(map[string]any{"name": t.Name(), "operations": solidRows})
		if err != nil {
			t.Fatal(err)
		}
		if err = os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	})
}
func solidRecord(req, got any) {
	data, err := json.Marshal(map[string]any{"request": req, "got": got})
	if err != nil {
		panic(err)
	}
	solidRows = append(solidRows, json.RawMessage(data))
}
func solidErr(b *meshBuilder) string {
	if b.err != nil {
		return b.err.Error()
	}
	return ""
}
func solidTexture(tex image.Image, raw bool) any {
	if tex == nil || reflect.ValueOf(tex).IsNil() {
		return nil
	}
	r := tex.Bounds()
	pixels := make([]byte, 0, r.Dx()*r.Dy()*4)
	for y := r.Min.Y; y < r.Max.Y; y++ {
		for x := r.Min.X; x < r.Max.X; x++ {
			c := color.RGBAModel.Convert(tex.At(x, y)).(color.RGBA)
			pixels = append(pixels, c.R, c.G, c.B, c.A)
		}
	}
	v := map[string]any{"width": r.Dx(), "height": r.Dy(), "sha256": fmt.Sprintf("%x", sha256.Sum256(pixels))}
	if raw {
		v["pixels"] = pixels
	}
	return v
}
func solidMesh(ts []Triangle, raw bool) any {
	ms := []any{}
	gs := []float64{}
	os := []bool{}
	out := []any{}
	materials := map[*Material]int{}
	groups := map[*nativeOpacityGroup]int{}
	owners := map[*nativePaintOwner]int{}
	for _, t := range ts {
		mi, gi, oi := 0, 0, 0
		if t.Material != nil {
			var ok bool
			mi, ok = materials[t.Material]
			if !ok {
				mi = len(ms) + 1
				materials[t.Material] = mi
				m := t.Material
				ms = append(ms, map[string]any{"color": m.Color, "roughness": m.Roughness, "metalness": m.Metalness, "emissive": m.Emissive, "unlit": m.Unlit, "multiply": m.Multiply, "contour": m.svgContour, "solid": m.svgSolidTexture, "gradient": m.gradient, "vector": m.Vector != nil, "texture": solidTexture(m.Texture, raw)})
			}
		}
		if t.OpacityGroup != nil {
			var ok bool
			gi, ok = groups[t.OpacityGroup]
			if !ok {
				gi = len(gs) + 1
				groups[t.OpacityGroup] = gi
				gs = append(gs, t.OpacityGroup.Opacity)
			}
		}
		if t.PaintOwner != nil {
			var ok bool
			oi, ok = owners[t.PaintOwner]
			if !ok {
				oi = len(os) + 1
				owners[t.PaintOwner] = oi
				os = append(os, t.PaintOwner.Opaque)
			}
		}
		out = append(out, map[string]any{"v": t.V, "material": mi, "group": gi, "owner": oi, "shadow": t.CastShadow, "opacity": t.ShadowOpacity, "alpha": t.ShadowFillAlpha, "ground": t.ShadowGround, "bias": t.DepthBias, "noDepth": t.NoDepthWrite, "coverage": t.svgCoverageOnly})
	}
	return map[string]any{"triangles": out, "materials": ms, "groups": gs, "owners": os}
}
func solidState(b *meshBuilder, raw bool) any {
	if len(b.panels) != 0 || len(b.links) != 0 {
		panic("unexpected panels/links")
	}
	return map[string]any{"mesh": solidMesh(b.triangles, raw), "error": solidErr(b), "scale": b.scale, "density": b.outputDensity, "pixels": b.facePixels, "budget": b.faceMaxPixels, "animated": b.animatedNodes}
}
func solidObserveBuild(t *testing.T, n d2isometric.Node) *meshBuilder {
	b := solidTestBuild(t, n)
	solidRecord(map[string]any{"op": "solid", "node": n}, map[string]any{"state": solidState(b, false), "node": n})
	return b
}
func solidObserveRelief(t *testing.T, n d2isometric.Node, h bool) *meshBuilder {
	b := reliefTestBuild(t, n, h)
	solidRecord(map[string]any{"op": "relief", "node": n, "hierarchy": h}, map[string]any{"state": solidState(b, false), "node": n})
	return b
}
func solidObserveNode(b *meshBuilder, n d2isometric.Node, tint string) {
	req := map[string]any{"op": "node", "state": solidState(b, true), "node": n, "tint": tint}
	b.node(n, tint)
	solidRecord(req, map[string]any{"state": solidState(b, false), "node": n})
}
func solidObserveCanonical(b *meshBuilder, n d2isometric.Node, tint string) {
	req := map[string]any{"op": "canonical", "state": solidState(b, true), "node": n, "tint": tint}
	b.canonicalNode(n, tint)
	solidRecord(req, map[string]any{"state": solidState(b, false), "node": n})
}
func solidObservePaint(b *meshBuilder, n d2isometric.Node, kind, fill string) nativeSolidPaint {
	req := map[string]any{"op": "paint", "state": solidState(b, true), "node": n, "kind": kind, "fill": fill}
	p := b.solidPaint(n, kind, fill)
	ts := []Triangle{{Material: p.cap}, {Material: p.ink}, {Material: p.wall}}
	solidRecord(req, map[string]any{"paint": solidMesh(ts, false), "fit": p.inkFit, "alpha": p.shadowFillAlpha, "state": solidState(b, false), "node": n})
	return p
}
func solidSurface(s labelSurface) any {
	return map[string]any{"center": s.center, "width": s.width, "depth": s.depth, "angle": s.angle, "align": s.align}
}
func solidObserveKnockout(b *meshBuilder, first int, s labelSurface) {
	req := map[string]any{"op": "knockout", "state": solidState(b, true), "first": first, "surface": solidSurface(s)}
	b.routeCaptionKnockout(first, s)
	solidRecord(req, solidState(b, false))
}
func solidObserveEdges(b *meshBuilder, es []d2isometric.Edge, p *routeCaptionPlacer, scenes ...*d2isometric.Scene) {
	if len(p.rects) != 0 {
		panic("unexpected occupied placer")
	}
	req := map[string]any{"op": "edges", "state": solidState(b, true), "edges": es, "scenes": scenes, "text": b.text != nil}
	b.edges(es, p, scenes...)
	solidRecord(req, map[string]any{"state": solidState(b, false), "edges": es})
}

// Keep context import tied to the context semantics the captured builders use.
var _ = context.Background
