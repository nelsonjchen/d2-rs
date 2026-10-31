// SPDX-License-Identifier: MPL-2.0
// Original-test observers, installed only in the pinned trace checkout.
package d2isometricimg

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"image"
	"image/color"
	"math"
	"os"
	"path/filepath"
	"testing"
)

var rasterRows []any
var rasterInputs map[*Raster]map[string]any

func rasterObserveStart(t *testing.T) {
	rasterRows = []any{}
	rasterInputs = map[*Raster]map[string]any{}
	t.Cleanup(func() {
		b, e := json.Marshal(map[string]any{"name": t.Name(), "operations": rasterRows})
		if e != nil {
			t.Fatal(e)
		}
		if e = os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), b, 0600); e != nil {
			t.Fatal(e)
		}
	})
}
func rasterRecord(req, got any) {
	b, e := json.Marshal(map[string]any{"request": req, "got": got})
	if e != nil {
		panic(e)
	}
	var v any
	decoder := json.NewDecoder(bytes.NewReader(b))
	decoder.UseNumber()
	if e = decoder.Decode(&v); e != nil {
		panic(e)
	}
	rasterRows = append(rasterRows, v)
	solidRecord(req, got)
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
	return map[string]any{"kind": kind, "rect": []int{rect.Min.X, rect.Min.Y, rect.Max.X, rect.Max.Y}, "stride": stride, "pixels_file": residualInputBytes(pix), "nil": false}
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
			g = map[string]any{"id": groups[t.OpacityGroup], "opacity": rasterNumber(t.OpacityGroup.Opacity)}
		}
		if t.PaintOwner != nil {
			if owners[t.PaintOwner] == 0 {
				owners[t.PaintOwner] = len(owners) + 1
			}
			o = map[string]any{"id": owners[t.PaintOwner], "opaque": t.PaintOwner.Opaque}
		}
		out = append(out, map[string]any{"vertices": vs, "material": m, "material_id": ids[t.Material], "cast_shadow": t.CastShadow, "shadow_opacity": t.ShadowOpacity, "shadow_fill_alpha": t.ShadowFillAlpha, "shadow_ground": t.ShadowGround, "depth_bias": t.DepthBias, "no_depth_write": t.NoDepthWrite, "group": g, "owner": o, "coverage": t.svgCoverageOnly})
	}
	return out
}
func rasterCameraInput(c *rasterCamera) any {
	if c == nil {
		return nil
	}
	return map[string]any{"right": rasterVector(c.right), "up": rasterVector(c.up), "direction": rasterVector(c.direction), "center_x": c.centerX, "center_y": c.centerY, "center_depth": c.centerDepth, "scale": c.scale, "width": c.width, "height": c.height}
}
func rasterImageOutput(p *image.RGBA) any {
	if p == nil {
		return nil
	}
	h := sha256.Sum256(p.Pix)
	digest := hex.EncodeToString(h[:])
	folder := filepath.Join(os.Getenv("D2RS_SCENE_API"), "..", "images")
	if e := os.MkdirAll(folder, 0700); e != nil {
		panic(e)
	}
	path := filepath.Join(folder, digest+".rgba")
	if _, e := os.Stat(path); os.IsNotExist(e) {
		if e = os.WriteFile(path, p.Pix, 0600); e != nil {
			panic(e)
		}
	}
	return map[string]any{"width": p.Rect.Dx(), "height": p.Rect.Dy(), "stride": p.Stride, "sha256": digest}
}
func rasterObserveNew(ctx context.Context, w, h int, ts []Triangle, bg color.NRGBA) (*Raster, error) {
	return rasterObserveBuild(ctx, w, h, ts, bg, nil, nil, true)
}
func rasterObserveConstruct(ctx context.Context, w, h int, ts []Triangle, bg color.NRGBA, c, s *rasterCamera, ground ...*rasterCamera) (*Raster, error) {
	return rasterObserveBuild(ctx, w, h, ts, bg, c, s, false, ground...)
}
func rasterObserveBuild(ctx context.Context, w, h int, ts []Triangle, bg color.NRGBA, c, s *rasterCamera, public bool, ground ...*rasterCamera) (*Raster, error) {
	req := map[string]any{"op": "render", "width": w, "height": h, "triangles": rasterTrianglesInput(ts), "dynamic": []any{}, "background": bg, "camera": rasterCameraInput(c), "shadow_camera": rasterCameraInput(s), "canceled": rasterContextInput(ctx)}
	var r *Raster
	var e error
	if public {
		r, e = NewRaster(ctx, w, h, ts, bg)
	} else {
		r, e = newRaster(ctx, w, h, ts, bg, c, s, ground...)
	}
	var output any
	if r != nil {
		output = rasterImageOutput(r.output)
		rasterInputs[r] = req
	}
	rasterRecord(req, map[string]any{"error": rasterError(e), "frame": output})
	return r, e
}
func rasterObserveFrame(r *Raster, ctx context.Context, ts []Triangle) (*image.RGBA, error) {
	initial, ok := rasterInputs[r]
	if !ok {
		panic("unobserved Raster")
	}
	req := map[string]any{}
	for k, v := range initial {
		req[k] = v
	}
	req["dynamic"] = rasterTrianglesInput(ts)
	req["canceled"] = rasterContextInput(ctx)
	p, e := r.Frame(ctx, ts)
	rasterRecord(req, map[string]any{"error": rasterError(e), "frame": rasterImageOutput(p)})
	return p, e
}
func rasterObserveSample(p image.Image, u, v float64) (float64, float64, float64, float64) {
	a, b, c, d := rasterTexture(p, u, v)
	rasterRecord(map[string]any{"op": "sample", "texture": rasterTextureInput(p), "u": u, "v": v}, map[string]any{"error": "", "rgba": []float64{a, b, c, d}})
	return a, b, c, d
}
func rasterObserveSnapshot(ctx context.Context, ts []Triangle) ([]Triangle, error) {
	req := map[string]any{"op": "snapshot", "triangles": rasterTrianglesInput(ts), "canceled": rasterContextInput(ctx)}
	out, e := rasterSnapshot(ctx, ts)
	rasterRecord(req, map[string]any{"error": rasterError(e), "triangles": residualSnapshotTriangles(out)})
	return out, e
}
func rasterObserveMultiply(p []uint8, r, g, b, a float64) {
	before := append([]uint8(nil), p...)
	rasterMultiplyOver(p, r, g, b, a)
	rasterRecord(map[string]any{"op": "multiply", "pixel": []int{int(before[0]), int(before[1]), int(before[2]), int(before[3])}, "rgb": []float64{r, g, b}, "alpha": a}, map[string]any{"error": "", "pixel": []int{int(p[0]), int(p[1]), int(p[2]), int(p[3])}})
}
func rasterObserveSpan(p [3]rasterPoint, y, x0, x1 int) (int, int) {
	a, b := rasterRowSpan(p, y, x0, x1)
	points := []any{}
	for _, p := range p {
		points = append(points, []float64{p.x, p.y, p.z})
	}
	rasterRecord(map[string]any{"op": "span", "points": points, "y": y, "x0": x0, "x1": x1}, map[string]any{"error": "", "span": []int{a, b}})
	return a, b
}
func rasterObserveCharge(w *rasterWork, n int64) error {
	req := map[string]any{"op": "charge", "remaining": w.remaining, "canceled": rasterContextInput(w.ctx), "n": n}
	e := w.charge(n)
	rasterRecord(req, map[string]any{"error": rasterError(e), "remaining": w.remaining})
	return e
}

func residualInputBytes(p []byte) string {
	folder := filepath.Join(os.Getenv("D2RS_SCENE_API"), "..", "inputs")
	if e := os.MkdirAll(folder, 0700); e != nil {
		panic(e)
	}
	hash := sha256.Sum256(p)
	path := filepath.Join(folder, hex.EncodeToString(hash[:])+".bin")
	if _, e := os.Stat(path); os.IsNotExist(e) {
		if e = os.WriteFile(path, p, 0600); e != nil {
			panic(e)
		}
	}
	return path
}
func residualSnapshotTriangles(ts []Triangle) any {
	out := rasterTrianglesInput(ts).([]any)
	for _, raw := range out {
		v := raw.(map[string]any)
		if m, ok := v["material"].(map[string]any); ok {
			if t, ok := m["texture"].(map[string]any); ok {
				if path, ok := t["pixels_file"].(string); ok {
					p, e := os.ReadFile(path)
					if e != nil {
						panic(e)
					}
					h := sha256.Sum256(p)
					delete(t, "pixels_file")
					t["sha256"], t["bytes"] = hex.EncodeToString(h[:]), len(p)
				}
			}
		}
	}
	return out
}
