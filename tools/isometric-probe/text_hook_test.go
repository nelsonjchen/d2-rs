// SPDX-License-Identifier: MPL-2.0
// Trace-only call capture. The harness temporarily renames the original
// textPainter.texture to d2rsOriginalTexture, leaving its body unchanged.
package d2isometricimg

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"image"
)

func (p *textPainter) texture(value string, style labelTextStyle) (*image.RGBA, textLayout, error) {
	if !d2rsTextEnabled {
		return p.d2rsOriginalTexture(value, style)
	}
	if d2rsTextPainters == nil {
		d2rsTextPainters = map[*textPainter]int{}
	}
	id, ok := d2rsTextPainters[p]
	if !ok {
		id = len(d2rsTextPainters)
		d2rsTextPainters[p] = id
	}
	encoded, _ := json.Marshal(style)
	var styleJSON map[string]any
	json.Unmarshal(encoded, &styleJSON)
	styleJSON["ForegroundPaint"] = encPaint(style.ForegroundPaint)
	styleJSON["BackgroundPaint"] = encPaint(style.BackgroundPaint)
	row := map[string]any{"painter": id, "count": p.count, "primary": p.primary, "mono": p.mono, "density": p.outputDensity, "value": value, "style": styleJSON}
	texture, layout, err := p.d2rsOriginalTexture(value, style)
	want := map[string]any{}
	if err != nil {
		want["error"] = err.Error()
	} else {
		want["layout"] = layout
		if texture != nil {
			want["width"], want["height"] = texture.Bounds().Dx(), texture.Bounds().Dy()
			want["pixels"] = fmt.Sprintf("%x", sha256.Sum256(texture.Pix))
			if vector := nativeVectorForTexture(p.ctx, texture); vector != nil {
				svg, e := nativeSurfaceSVG(p.ctx, vector, "text")
				if e != nil {
					want["surfaceError"] = e.Error()
				} else {
					want["surface"] = string(svg)
				}
			}
		}
	}
	row["want"] = want
	d2rsTextCalls = append(d2rsTextCalls, row)
	return texture, layout, err
}
