// SPDX-License-Identifier: MPL-2.0
package d2isometricimg

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"image"
	"image/color"
	"os"
	"path/filepath"
	"testing"
)

var d2rsRasterRows []interface{}
var d2rsRasterIDs map[*Raster]int

func d2rsRasterStart(t *testing.T) {
	d2rsRasterRows = []interface{}{}
	d2rsRasterIDs = map[*Raster]int{}
	t.Cleanup(func() {
		data, err := json.Marshal(map[string]interface{}{"name": t.Name(), "operations": d2rsRasterRows})
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	})
}

func d2rsRasterColor(c color.NRGBA) [4]uint8 { return [4]uint8{c.R, c.G, c.B, c.A} }
func d2rsRasterVec(v Vec) []float64          { return []float64{v.X, v.Y, v.Z} }
func d2rsRasterTriangles(ts []Triangle) []interface{} {
	out := []interface{}{}
	for _, t := range ts {
		vertices := []interface{}{}
		for _, v := range t.V {
			vertices = append(vertices, map[string]interface{}{"p": d2rsRasterVec(v.Position), "n": d2rsRasterVec(v.Normal), "u": v.U, "v": v.V})
		}
		var material interface{}
		if m := t.Material; m != nil {
			if m.Vector != nil || m.svgContour || m.svgSolidTexture || m.gradient != "" {
				panic("uncaptured raster material field")
			}
			var texture interface{}
			if m.Texture != nil {
				tex, ok := m.Texture.(*image.RGBA)
				if !ok {
					panic("unsupported texture type in raster observer")
				}
				texture = map[string]interface{}{"width": tex.Rect.Dx(), "height": tex.Rect.Dy(), "stride": tex.Stride, "pixels": base64.StdEncoding.EncodeToString(tex.Pix)}
			}
			material = map[string]interface{}{"color": d2rsRasterColor(m.Color), "roughness": m.Roughness, "metalness": m.Metalness, "emissive": d2rsRasterColor(m.Emissive), "unlit": m.Unlit, "multiply": m.Multiply, "texture": texture}
		}
		if t.OpacityGroup != nil || t.PaintOwner != nil || t.ShadowOpacity != nil || t.ShadowFillAlpha != nil || t.ShadowGround != nil || t.svgCoverageOnly {
			panic("uncaptured triangle field")
		}
		out = append(out, map[string]interface{}{"v": vertices, "material": material, "cast_shadow": t.CastShadow, "depth_bias": t.DepthBias, "no_depth_write": t.NoDepthWrite})
	}
	return out
}
func d2rsRasterImage(p *image.RGBA, err error) interface{} {
	message := ""
	if err != nil {
		message = err.Error()
	}
	var frame interface{}
	if p != nil {
		frame = map[string]interface{}{"width": p.Rect.Dx(), "height": p.Rect.Dy(), "stride": p.Stride, "pixels": base64.StdEncoding.EncodeToString(p.Pix)}
	}
	return map[string]interface{}{"frame": frame, "error": message}
}
func d2rsRasterNew(ctx context.Context, width, height int, ts []Triangle, bg color.NRGBA) (*Raster, error) {
	request := map[string]interface{}{"op": "new", "width": width, "height": height, "triangles": d2rsRasterTriangles(ts), "background": d2rsRasterColor(bg), "canceled": ctx.Err() != nil}
	r, err := NewRaster(ctx, width, height, ts, bg)
	var output *image.RGBA
	if r != nil {
		d2rsRasterIDs[r] = len(d2rsRasterIDs) + 1
		output = r.output
	}
	request["id"] = d2rsRasterIDs[r]
	d2rsRasterRows = append(d2rsRasterRows, map[string]interface{}{"request": request, "got": d2rsRasterImage(output, err)})
	return r, err
}
func d2rsRasterFrame(r *Raster, ctx context.Context, dynamic []Triangle) (*image.RGBA, error) {
	request := map[string]interface{}{"op": "frame", "id": d2rsRasterIDs[r], "triangles": d2rsRasterTriangles(dynamic), "canceled": ctx.Err() != nil}
	p, err := r.Frame(ctx, dynamic)
	d2rsRasterRows = append(d2rsRasterRows, map[string]interface{}{"request": request, "got": d2rsRasterImage(p, err)})
	return p, err
}
