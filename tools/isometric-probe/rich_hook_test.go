// SPDX-License-Identifier: MPL-2.0
// Trace-only call capture. The harness temporarily renames the original
// richLabelPainter.texture to d2rsOriginalTexture, leaving its body unchanged.
package d2isometricimg

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"github.com/d2lang/d2/d2target"
	"image"
)

func (p *richLabelPainter) texture(value d2target.Shape, style labelTextStyle) (*image.RGBA, error) {
	if !d2rsRichEnabled {
		return p.d2rsOriginalTexture(value, style)
	}
	if d2rsRichPainters == nil {
		d2rsRichPainters = map[*richLabelPainter]int{}
	}
	id, ok := d2rsRichPainters[p]
	if !ok {
		id = len(d2rsRichPainters)
		d2rsRichPainters[p] = id
	}
	encoded, _ := json.Marshal(style)
	var styleJSON map[string]any
	json.Unmarshal(encoded, &styleJSON)
	styleJSON["ForegroundPaint"] = encPaint(style.ForegroundPaint)
	styleJSON["BackgroundPaint"] = encPaint(style.BackgroundPaint)
	row := map[string]any{"painter": id, "count": p.count, "primary": p.primary, "mono": p.mono, "density": p.outputDensity, "value": value, "themeID": p.themeID, "overrides": p.themeOverrides, "style": styleJSON}
	texture, err := p.d2rsOriginalTexture(value, style)
	want := map[string]any{}
	if err != nil {
		want["error"] = err.Error()
	} else {
		want["viewBox"] = p.lastDocument.ViewBox
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
	d2rsRichCalls = append(d2rsRichCalls, row)
	return texture, err
}
