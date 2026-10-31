// SPDX-License-Identifier: MPL-2.0
// Installed only in the manifest's instrumentation checkout during capture.
package d2isometricimg

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"image"
	"math"
	"net/url"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2target"
)

var d2rsSurfaceRows []interface{}
var d2rsSurfaceIDs map[interface{}]int
var d2rsSurfaceTextures map[*image.RGBA]int

func d2rsSurfaceStart(t *testing.T) {
	d2rsSurfaceRows = []interface{}{}
	d2rsSurfaceIDs = map[interface{}]int{}
	d2rsSurfaceTextures = map[*image.RGBA]int{}
	t.Cleanup(func() {
		data, err := json.Marshal(map[string]interface{}{"name": t.Name(), "operations": d2rsSurfaceRows})
		if err != nil {
			t.Fatal(err)
		}
		if err = os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	})
}
func d2rsSurfaceID(p interface{}) int {
	if n, ok := d2rsSurfaceIDs[p]; ok {
		return n
	}
	n := len(d2rsSurfaceIDs) + 1
	d2rsSurfaceIDs[p] = n
	return n
}
func d2rsSurfaceRecord(req, got interface{}) {
	if d2rsSurfaceIDs != nil {
		d2rsSurfaceRows = append(d2rsSurfaceRows, map[string]interface{}{"request": req, "got": got})
	}
}
func d2rsSurfaceError(err error) string {
	if err != nil {
		return err.Error()
	}
	return ""
}
func d2rsSurfaceCtx(ctx context.Context) string {
	if ctx == nil {
		return "nil"
	}
	if ctx.Err() != nil {
		return ctx.Err().Error()
	}
	return ""
}
func d2rsSurfaceVector(ctx context.Context) bool {
	return ctx != nil && ctx.Value(nativeVectorKey{}) != nil
}
func d2rsSurfaceFloat(x float64) interface{} {
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
func d2rsSurfaceResult(ctx context.Context, frame *image.RGBA, err error) map[string]interface{} {
	out := map[string]interface{}{"error": d2rsSurfaceError(err), "frame": nil, "surface": nil, "surface_error": "", "texture_id": 0}
	if frame == nil {
		return out
	}
	if d2rsSurfaceTextures != nil {
		if _, ok := d2rsSurfaceTextures[frame]; !ok {
			d2rsSurfaceTextures[frame] = len(d2rsSurfaceTextures) + 1
		}
		out["texture_id"] = d2rsSurfaceTextures[frame]
	}
	out["frame"] = map[string]interface{}{"width": frame.Rect.Dx(), "height": frame.Rect.Dy(), "stride": frame.Stride, "pixels": base64.StdEncoding.EncodeToString(frame.Pix)}
	if v := nativeVectorForTexture(ctx, frame); v != nil {
		svg, e := nativeSurfaceSVG(ctx, v, "surface")
		out["surface"] = svg
		out["surface_error"] = d2rsSurfaceError(e)
	}
	return out
}
func newTextPainter(ctx context.Context, count int) (*textPainter, error) {
	p, err := d2rsOriginalNewTextPainter(ctx, count)
	id := 0
	if p != nil {
		id = d2rsSurfaceID(p)
	}
	d2rsSurfaceRecord(map[string]interface{}{"op": "new_text", "id": id, "context": d2rsSurfaceCtx(ctx), "count": count, "vectors": d2rsSurfaceVector(ctx)}, map[string]interface{}{"error": d2rsSurfaceError(err), "nil": p == nil})
	return p, err
}
func (p *textPainter) texture(value string, style labelTextStyle) (*image.RGBA, textLayout, error) {
	request := map[string]interface{}{"op": "text", "id": d2rsSurfaceID(p), "context": d2rsSurfaceCtx(p.ctx), "value": value, "style": style, "density": p.outputDensity, "primary": p.primary, "mono": p.mono}
	texture, layout, err := p.d2rsOriginalTexture(value, style)
	out := d2rsSurfaceResult(p.ctx, texture, err)
	out["layout"] = layout
	d2rsSurfaceRecord(request, out)
	return texture, layout, err
}
func d2rsTextGlyphWork(p *textPainter, n int) {
	p.glyphWork = n
	d2rsSurfaceRecord(map[string]interface{}{"op": "glyph_work", "id": d2rsSurfaceID(p), "value": n}, map[string]interface{}{"error": ""})
}
func newRichLabelPainter(ctx context.Context, count int) (*richLabelPainter, error) {
	p, err := d2rsOriginalNewRichLabelPainter(ctx, count)
	id := 0
	if p != nil {
		id = d2rsSurfaceID(p)
	}
	d2rsSurfaceRecord(map[string]interface{}{"op": "new_rich", "id": id, "context": d2rsSurfaceCtx(ctx), "count": count, "vectors": d2rsSurfaceVector(ctx)}, map[string]interface{}{"error": d2rsSurfaceError(err), "nil": p == nil})
	return p, err
}
func (p *richLabelPainter) texture(value d2target.Shape, style labelTextStyle) (*image.RGBA, error) {
	request := map[string]interface{}{"op": "rich", "id": d2rsSurfaceID(p), "context": d2rsSurfaceCtx(p.ctx), "value": value, "style": style, "density": p.outputDensity, "primary": p.primary, "mono": p.mono}
	texture, err := p.d2rsOriginalTexture(value, style)
	out := d2rsSurfaceResult(p.ctx, texture, err)
	out["document"] = nil
	if p.lastDocument != nil {
		out["document"] = map[string]interface{}{"view_box": p.lastDocument.ViewBox, "links": p.lastDocument.Links}
	}
	d2rsSurfaceRecord(request, out)
	return texture, err
}
func newSurfaceIconPainter(ctx context.Context, count int, assets *d2scenebuild.AssetOptions) (*surfaceIconPainter, error) {
	if assets != nil {
		panic("surface observer does not admit custom asset options")
	}
	p, err := d2rsOriginalNewSurfaceIconPainter(ctx, count, assets)
	id := 0
	if p != nil {
		id = d2rsSurfaceID(p)
	}
	d2rsSurfaceRecord(map[string]interface{}{"op": "new_icon", "id": id, "context": d2rsSurfaceCtx(ctx), "count": count, "vectors": d2rsSurfaceVector(ctx)}, map[string]interface{}{"error": d2rsSurfaceError(err), "nil": p == nil})
	return p, err
}
func (p *surfaceIconPainter) texture(source *url.URL, width, depth float64, radius ...float64) (*image.RGBA, error) {
	request := map[string]interface{}{"op": "icon", "id": d2rsSurfaceID(p), "context": d2rsSurfaceCtx(p.ctx), "source": source.String(), "width": width, "depth": depth, "radius": radius, "density": p.outputDensity}
	texture, err := p.d2rsOriginalTexture(source, width, depth, radius...)
	d2rsSurfaceRecord(request, d2rsSurfaceResult(p.ctx, texture, err))
	return texture, err
}
func d2rsIconSourceBudget(p *surfaceIconPainter, n int) {
	p.remaining.MaxSourceBytes = n
	d2rsSurfaceRecord(map[string]interface{}{"op": "icon_source_budget", "id": d2rsSurfaceID(p), "value": n}, map[string]interface{}{"error": ""})
}
func surfaceIconDimensions(width, depth float64, maximum int) (int, int) {
	return d2rsOriginalSurfaceIconDimensions(width, depth, maximum)
}
func d2rsSurfaceArea(s labelSurface) interface{} {
	return map[string]interface{}{"center": s.center, "width": s.width, "depth": s.depth, "angle": s.angle}
}
func surfaceIconLayout(s labelSurface, original d2target.Shape, scale float64, kind string) (labelSurface, labelSurface) {
	a, b := d2rsOriginalSurfaceIconLayout(s, original, scale, kind)
	d2rsSurfaceRecord(map[string]interface{}{"op": "icon_layout", "surface": d2rsSurfaceArea(s), "original": original, "scale": scale, "kind": kind}, map[string]interface{}{"error": "", "icon": d2rsSurfaceArea(a), "text": d2rsSurfaceArea(b)})
	return a, b
}
func fitRichViewport(doc *d2scene.Document, style labelTextStyle) error {
	return d2rsOriginalFitRichViewport(doc, style)
}

// The following wrappers are used only by direct calls in selected tests.
func d2rsFitRichViewport(doc *d2scene.Document, style labelTextStyle) error {
	box := doc.ViewBox
	err := fitRichViewport(doc, style)
	d2rsSurfaceRecord(map[string]interface{}{"op": "fit_rich", "box": box, "style": style}, map[string]interface{}{"error": d2rsSurfaceError(err), "box": doc.ViewBox})
	return err
}
func d2rsSurfaceTextureDimensions(width, depth float64, maximum, budget int) (int, int) {
	w, h := surfaceTextureDimensions(width, depth, maximum, budget)
	d2rsSurfaceRecord(map[string]interface{}{"op": "dimensions", "width": d2rsSurfaceFloat(width), "depth": d2rsSurfaceFloat(depth), "maximum": maximum, "budget": budget, "density": 0}, map[string]interface{}{"error": "", "width": w, "height": h})
	return w, h
}
func d2rsSurfaceTextureDimensionsAtDensity(width, depth float64, maximum, budget int, density float64) (int, int) {
	w, h := surfaceTextureDimensionsAtDensity(width, depth, maximum, budget, density)
	d2rsSurfaceRecord(map[string]interface{}{"op": "dimensions", "width": d2rsSurfaceFloat(width), "depth": d2rsSurfaceFloat(depth), "maximum": maximum, "budget": budget, "density": d2rsSurfaceFloat(density)}, map[string]interface{}{"error": "", "width": w, "height": h})
	return w, h
}
func d2rsFitNativeLabel(value string, flag bool, measure func(string, float64) (float64, error), width, depth, size float64, lines int) (textLayout, error) {
	layout, err := fitNativeLabel(value, flag, measure, width, depth, size, lines)
	d2rsSurfaceRecord(map[string]interface{}{"op": "fit_text", "value": value, "width": width, "depth": depth, "size": size, "measure": "rune_count_times_size"}, map[string]interface{}{"error": d2rsSurfaceError(err), "layout": layout})
	return layout, err
}

func d2rsSurfaceIconDimensions(width, depth float64, maximum int) (int, int) {
	w, h := surfaceIconDimensions(width, depth, maximum)
	d2rsSurfaceRecord(map[string]interface{}{"op": "icon_dimensions", "width": d2rsSurfaceFloat(width), "depth": d2rsSurfaceFloat(depth), "maximum": maximum}, map[string]interface{}{"error": "", "width": w, "height": h})
	return w, h
}
