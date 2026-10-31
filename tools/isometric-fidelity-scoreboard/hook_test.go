// SPDX-License-Identifier: MPL-2.0
// Original-test observers, installed only in the pinned trace checkout.
package d2isometricimg

import (
	"context"
	"encoding/base64"
	"encoding/binary"
	"encoding/json"
	"image"
	"math"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/internal/fontface"
)

var fidelityRows []any

func d2rsGeometryStart(t *testing.T) {
	fidelityRows = []any{}
	contentAssetStore = filepath.Join(os.Getenv("D2RS_SCENE_API"), "..", "assets")
	t.Cleanup(func() {
		b, err := json.Marshal(map[string]any{"name": t.Name(), "operations": fidelityRows})
		if err != nil {
			t.Fatal(err)
		}
		if err = os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), b, 0600); err != nil {
			t.Fatal(err)
		}
	})
}
func fidelityRecord(req, got any) {
	b, err := json.Marshal(map[string]any{"request": req, "got": got})
	if err != nil {
		panic(err)
	}
	fidelityRows = append(fidelityRows, json.RawMessage(b))
}
func fidelityErr(e error) string {
	if e != nil {
		return e.Error()
	}
	return ""
}
func fidelityShadow(w *rasterWork, ts []Triangle) (rasterShadow, error) {
	req := map[string]any{"op": "shadow", "triangles": rasterTrianglesInput(ts), "remaining": w.remaining, "canceled": w.ctx.Err() != nil}
	s, e := rasterBuildShadow(w, ts)
	var got any
	if e == nil {
		depth := make([]byte, len(s.depth)*4)
		for i, n := range s.depth {
			binary.LittleEndian.PutUint32(depth[4*i:], math.Float32bits(n))
		}
		got = map[string]any{"camera": rasterCameraInput(&s.camera), "depth": depth, "opacity": s.opacity, "bias": s.bias}
	}
	fidelityRecord(req, map[string]any{"error": fidelityErr(e), "shadow": got, "remaining": w.remaining})
	return s, e
}
func fidelityValidate(ctx context.Context, ts []Triangle) error {
	req := map[string]any{"op": "validate", "triangles": rasterTrianglesInput(ts), "canceled": ctx.Err() != nil}
	e := rasterValidate(ctx, ts)
	fidelityRecord(req, map[string]any{"error": fidelityErr(e)})
	return e
}
func fidelitySVG(ctx context.Context, s *nativeVectorSurface, prefix string) (string, error) {
	if s.inkCoverage != nil || s.capBackground != nil || s.capPaint != nil {
		panic("unexpected original surface")
	}
	req := map[string]any{"op": "svg", "document": contentEncDocument(s.document), "prefix": prefix, "canceled": ctx.Err() != nil}
	v, e := nativeSurfaceSVG(ctx, s, prefix)
	fidelityRecord(req, map[string]any{"error": fidelityErr(e), "svg": v})
	return v, e
}
func fidelityColorPaint(p fontface.COLRv1Paint) any {
	switch p := p.(type) {
	case fontface.COLRv1Solid:
		return map[string]any{"solid": p}
	case fontface.COLRv1LinearGradient:
		return map[string]any{"linear": p}
	case fontface.COLRv1Composite:
		return map[string]any{"composite": map[string]any{"mode": p.Mode, "source": fidelityColorPaint(p.Source), "backdrop": fidelityColorPaint(p.Backdrop)}}
	}
	panic("unexpected original paint plan")
}
func fidelityWriter(w *nativeSurfaceSVGWriter) any {
	return map[string]any{"defs": w.defs.String(), "next": w.next, "nodes": w.nodes, "commands": w.commands, "transform": w.transform, "linear": w.linearGradients, "error": fidelityErr(w.err)}
}
func fidelityPaint(w *nativeSurfaceSVGWriter, face *fontface.ParsedFace, p fontface.COLRv1Paint, b d2scene.Box, m d2scene.Matrix, depth int) string {
	if face != nil || len(w.fonts) != 0 || len(w.glyphs) != 0 {
		panic("unexpected original font-bearing plan")
	}
	var doc any
	if w.doc != nil {
		doc = contentEncDocument(w.doc)
	}
	req := map[string]any{"op": "paint", "document": doc, "prefix": w.prefix, "writer": fidelityWriter(w), "paint": fidelityColorPaint(p), "box": b, "matrix": m, "depth": depth, "canceled": w.ctx.Err() != nil}
	v := w.colorPaint(face, p, b, m, depth)
	fidelityRecord(req, map[string]any{"body": v, "writer": fidelityWriter(w)})
	return v
}

func fidelityOptionalNumber(p *float64) any {
	if p == nil {
		return nil
	}
	return rasterNumber(*p)
}
func rasterNumber(x float64) any {
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
func rasterVector(v Vec) any { return []any{rasterNumber(v.X), rasterNumber(v.Y), rasterNumber(v.Z)} }
func rasterError(e error) string {
	if e != nil {
		return e.Error()
	}
	return ""
}
func rasterContextInput(c context.Context) bool { return c != nil && c.Err() != nil }
func rasterTextureInput(p image.Image) any {
	if p == nil {
		return nil
	}
	var kind string
	var rect image.Rectangle
	var stride int
	var pix []byte
	switch p := p.(type) {
	case *image.RGBA:
		kind = "rgba"
		if p == nil {
			return map[string]any{"kind": kind, "nil": true}
		}
		rect, stride, pix = p.Rect, p.Stride, p.Pix
	case *image.NRGBA:
		kind = "nrgba"
		if p == nil {
			return map[string]any{"kind": kind, "nil": true}
		}
		rect, stride, pix = p.Rect, p.Stride, p.Pix
	case *image.Alpha:
		kind = "alpha"
		if p == nil {
			return map[string]any{"kind": kind, "nil": true}
		}
		rect, stride, pix = p.Rect, p.Stride, p.Pix
	default:
		panic("unsupported observed texture")
	}
	return map[string]any{"kind": kind, "rect": []int{rect.Min.X, rect.Min.Y, rect.Max.X, rect.Max.Y}, "stride": stride, "pixels": base64.StdEncoding.EncodeToString(pix), "nil": false}
}
func rasterTrianglesInput(ts []Triangle) any {
	out := []any{}
	ids := map[*Material]int{}
	groups := map[*nativeOpacityGroup]int{}
	owners := map[*nativePaintOwner]int{}
	for _, t := range ts {
		var m any
		if t.Material != nil {
			v := t.Material
			if ids[v] == 0 {
				ids[v] = len(ids) + 1
			}
			m = map[string]any{"color": v.Color, "emissive": v.Emissive, "roughness": rasterNumber(v.Roughness), "metalness": rasterNumber(v.Metalness), "texture": rasterTextureInput(v.Texture), "unlit": v.Unlit, "multiply": v.Multiply, "contour": v.svgContour, "solid": v.svgSolidTexture}
		}
		vs := []any{}
		for _, v := range t.V {
			vs = append(vs, map[string]any{"position": rasterVector(v.Position), "normal": rasterVector(v.Normal), "u": rasterNumber(v.U), "v": rasterNumber(v.V)})
		}
		var g, o any
		if t.OpacityGroup != nil {
			if groups[t.OpacityGroup] == 0 {
				groups[t.OpacityGroup] = len(groups) + 1
			}
			g = map[string]any{"id": groups[t.OpacityGroup], "opacity": t.OpacityGroup.Opacity}
		}
		if t.PaintOwner != nil {
			if owners[t.PaintOwner] == 0 {
				owners[t.PaintOwner] = len(owners) + 1
			}
			o = map[string]any{"id": owners[t.PaintOwner], "opaque": t.PaintOwner.Opaque}
		}
		out = append(out, map[string]any{"vertices": vs, "material": m, "material_id": ids[t.Material], "cast_shadow": t.CastShadow, "shadow_opacity": fidelityOptionalNumber(t.ShadowOpacity), "shadow_fill_alpha": t.ShadowFillAlpha, "shadow_ground": t.ShadowGround, "depth_bias": t.DepthBias, "no_depth_write": t.NoDepthWrite, "group": g, "owner": o, "coverage": t.svgCoverageOnly})
	}
	return out
}
func rasterCameraInput(c *rasterCamera) any {
	if c == nil {
		return nil
	}
	return map[string]any{"right": rasterVector(c.right), "up": rasterVector(c.up), "direction": rasterVector(c.direction), "center_x": c.centerX, "center_y": c.centerY, "center_depth": c.centerDepth, "scale": c.scale, "width": c.width, "height": c.height}
}
