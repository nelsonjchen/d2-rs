// SPDX-License-Identifier: MPL-2.0
// Mesh wire projection shared with the original solid observer.
package d2isometricimg

import (
	"crypto/sha256"
	"fmt"
	"image"
	"image/color"
	"reflect"
)

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
