// SPDX-License-Identifier: MPL-2.0
// Original-test observer, installed only in the pinned instrumentation checkout.
package d2isometricimg

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2target"
	"image"
	"image/color"
	"math"
	"os"
	"path/filepath"
	"testing"
)

var contentRows []any

func contentStart(t *testing.T) {
	contentRows = []any{}
	contentAssetStore = filepath.Join(os.Getenv("D2RS_SCENE_API"), "..", "assets")
	t.Cleanup(func() {
		b, e := json.Marshal(map[string]any{"name": t.Name(), "operations": contentRows})
		if e != nil {
			t.Fatal(e)
		}
		if e = os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), b, 0600); e != nil {
			t.Fatal(e)
		}
	})
}
func contentRecord(req, got any) {
	b, e := json.Marshal(map[string]any{"request": req, "got": got})
	if e != nil {
		panic(e)
	}
	var v any
	if e = json.Unmarshal(b, &v); e != nil {
		panic(e)
	}
	contentRows = append(contentRows, v)
}
func contentErr(e error) string {
	if e != nil {
		return e.Error()
	}
	return ""
}
func contentContext(c context.Context) bool { return c != nil && c.Err() != nil }
func contentImage(p *image.RGBA) any {
	if p == nil {
		return nil
	}
	return map[string]any{"width": p.Rect.Dx(), "height": p.Rect.Dy(), "stride": p.Stride, "pixels": base64.StdEncoding.EncodeToString(p.Pix)}
}
func contentColor(c color.NRGBA) any { return []uint8{c.R, c.G, c.B, c.A} }
func contentSurface(s *nativeVectorSurface) any {
	if s == nil {
		return nil
	}
	return map[string]any{"document": contentEncDocument(s.document), "coverage": contentSurface(s.inkCoverage), "opacity": s.coverageOpacity, "cap": s.capBackground, "paint": contentEncPaint(s.capPaint)}
}
func contentSVG(ctx context.Context, s *nativeVectorSurface, prefix string) (string, error) {
	req := map[string]any{"op": "svg", "surface": contentSurface(s), "prefix": prefix, "canceled": contentContext(ctx)}
	v, e := nativeSurfaceSVG(ctx, s, prefix)
	contentRecord(req, map[string]any{"error": contentErr(e), "svg": v})
	return v, e
}
func contentCap(s *nativeVectorSurface, c color.NRGBA) *nativeVectorSurface {
	req := map[string]any{"op": "cap", "surface": contentSurface(s), "color": c}
	v := nativeVectorSolidCap(s, c)
	contentRecord(req, map[string]any{"error": "", "surface": contentSurface(v), "same": v == s})
	return v
}
func contentRaster(ctx context.Context, d *d2scene.Document, w, h int, at ...float64) (*image.RGBA, error) {
	times := []any{}
	for _, n := range at {
		if math.IsInf(n, 1) {
			times = append(times, "+Inf")
		} else {
			times = append(times, n)
		}
	}
	req := map[string]any{"op": "raster_doc", "document": contentEncDocument(d), "width": w, "height": h, "at": times, "canceled": contentContext(ctx)}
	p, e := rasterNativeSurfaceDocument(ctx, d, w, h, at...)
	contentRecord(req, map[string]any{"error": contentErr(e), "frame": contentImage(p), "source": contentEncDocument(d)})
	return p, e
}
func contentMatrix(m d2scene.Matrix) string {
	v := nativeSVGMatrix(m)
	contentRecord(map[string]any{"op": "matrix", "matrix": m}, map[string]any{"error": "", "value": v})
	return v
}
func contentView() Vec {
	v := nativeViewDirection()
	contentRecord(map[string]any{"op": "view"}, map[string]any{"error": "", "value": v})
	return v
}
func contentTint(e d2isometric.Edge) string {
	v := nativeRouteTint(e)
	contentRecord(map[string]any{"op": "tint", "edge": e}, map[string]any{"error": "", "value": v})
	return v
}
func contentSection(p []Vec, l []float64, a, b float64) []Vec {
	v := nativeRouteSection(p, l, a, b)
	contentRecord(map[string]any{"op": "section", "points": p, "lengths": l, "a": a, "b": b}, map[string]any{"error": "", "value": v})
	return v
}
func contentMaterial(m *Material) any {
	if m == nil {
		return nil
	}
	var tex any
	if m.Texture != nil {
		p, ok := m.Texture.(*image.RGBA)
		if !ok {
			panic("non RGBA content texture")
		}
		tex = contentImage(p)
	}
	return map[string]any{"color": m.Color, "roughness": m.Roughness, "metalness": m.Metalness, "emissive": m.Emissive, "texture": tex, "vector": contentSurface(m.Vector), "unlit": m.Unlit, "multiply": m.Multiply, "contour": m.svgContour, "solid": m.svgSolidTexture, "gradient": m.gradient}
}
func contentTriangles(ts []Triangle) any {
	out := []any{}
	ids := map[*Material]int{}
	for _, t := range ts {
		if _, ok := ids[t.Material]; !ok && t.Material != nil {
			ids[t.Material] = len(ids) + 1
		}
		out = append(out, map[string]any{"vertices": t.V, "material": contentMaterial(t.Material), "material_id": ids[t.Material], "shadow": t.CastShadow, "opacity": t.ShadowOpacity, "fill_alpha": t.ShadowFillAlpha, "ground": t.ShadowGround, "bias": t.DepthBias, "no_depth": t.NoDepthWrite, "coverage": t.svgCoverageOnly, "group": t.OpacityGroup, "owner": t.PaintOwner})
	}
	return out
}
func contentBuilder(b *meshBuilder, op string) map[string]any {
	return map[string]any{"op": op, "scale": b.scale, "density": b.outputDensity, "face_max": b.faceMaxPixels, "text": b.text != nil, "rich": b.rich != nil}
}
func contentBuilt(b *meshBuilder) any {
	return map[string]any{"error": contentErr(b.err), "triangles": contentTriangles(b.triangles)}
}
func contentNode(b *meshBuilder, n d2isometric.Node, tint string) {
	req := contentBuilder(b, "node")
	req["node"], req["tint"] = n, tint
	b.node(n, tint)
	contentRecord(req, contentBuilt(b))
}
func contentEdges(b *meshBuilder, e []d2isometric.Edge, p *routeCaptionPlacer) []packetRoute {
	req := contentBuilder(b, "edges")
	req["edges"] = e
	v := b.edges(e, p)
	contentRecord(req, contentBuilt(b))
	return v
}
func contentInk(b *meshBuilder, p []Vec, r float64, m *Material) {
	req := contentBuilder(b, "ink")
	req["points"], req["radius"], req["material"] = p, r, contentMaterial(m)
	b.routeInk(p, r, m)
	contentRecord(req, contentBuilt(b))
}
func contentTube(b *meshBuilder, p []Vec, r float64, m *Material) {
	req := contentBuilder(b, "tube")
	req["points"], req["radius"], req["material"] = p, r, contentMaterial(m)
	b.tube(p, r, m)
	contentRecord(req, contentBuilt(b))
}
func contentCasing(b *meshBuilder, p []Vec, r, o float64) {
	req := contentBuilder(b, "casing")
	req["points"], req["radius"], req["opacity"] = p, r, o
	b.routeCasing(p, r, o)
	contentRecord(req, contentBuilt(b))
}
func contentFace(b *meshBuilder, s d2target.Shape) (*image.RGBA, labelSurface) {
	req := contentBuilder(b, "face")
	req["shape"] = s
	p, a := b.nativeFace(s)
	contentRecord(req, map[string]any{"error": contentErr(b.err), "frame": contentImage(p), "area": map[string]any{"center": a.center, "width": a.width, "depth": a.depth, "angle": a.angle, "align": a.align}})
	return p, a
}
func contentShade(r *Raster, p rasterPaint, x, n Vec) (float64, float64, float64) {
	a, b, c := r.shade(p, x, n)
	contentRecord(map[string]any{"op": "shade", "paint": map[string]any{"base": p.base, "emission": p.emission, "alpha": p.alpha, "roughness": p.roughness, "metalness": p.metalness, "unlit": p.unlit, "multiply": p.multiply}, "position": x, "normal": n}, map[string]any{"error": "", "rgb": []float64{a, b, c}})
	return a, b, c
}

func contentDocument(ctx context.Context, d *d2target.Diagram, f *d2scenebuild.AssetOptions) (*d2scene.Document, error) {
	if f != nil {
		panic("uncaptured font options")
	}
	req := map[string]any{"op": "document", "diagram": d}
	doc, e := nativeSurfaceDocument(ctx, d, f)
	var v any
	if doc != nil {
		v = contentEncDocument(doc)
	}
	contentRecord(req, map[string]any{"error": contentErr(e), "document": v})
	return doc, e
}

func contentAperture(s *nativeVectorSurface, a d2scene.Box) {
	req := map[string]any{"op": "aperture", "surface": contentSurface(s), "area": a}
	nativeVectorAperture(s, a)
	contentRecord(req, map[string]any{"error": "", "surface": contentSurface(s)})
}
func contentOpacity(b *meshBuilder, fill, ink *image.RGBA, opacity float64) {
	req := map[string]any{"op": "face_opacity", "fill": contentImage(fill), "ink": contentImage(ink), "fill_vector": contentSurface(nativeVectorForTexture(b.ctx, fill)), "ink_vector": contentSurface(nativeVectorForTexture(b.ctx, ink)), "opacity": opacity}
	b.nativeFaceOpacity(fill, ink, opacity)
	contentRecord(req, map[string]any{"error": contentErr(b.err), "fill": contentImage(fill), "ink": contentImage(ink), "fill_vector": contentSurface(nativeVectorForTexture(b.ctx, fill)), "ink_vector": contentSurface(nativeVectorForTexture(b.ctx, ink))})
}
