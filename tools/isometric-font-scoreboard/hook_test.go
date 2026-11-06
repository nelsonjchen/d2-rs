// SPDX-License-Identifier: MPL-2.0
// Original-test observer installed only in the pinned instrumentation checkout.
package d2isometricimg

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"image"
	"os"
	"path/filepath"
	"sort"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2target"
)

var fontRows []any
var fontIDs map[*textPainter]int

type fontResolver struct {
	original *surfaceFixedFallback
	requests []d2fonts.FallbackRequest
}

func (r *fontResolver) ResolveFallbacks(ctx context.Context, request d2fonts.FallbackRequest) ([]d2fonts.FallbackFont, error) {
	copied := request
	copied.Runes = append([]rune(nil), request.Runes...)
	r.requests = append(r.requests, copied)
	return r.original.ResolveFallbacks(ctx, request)
}
func fontConfigureFallback(p *textPainter, options *d2scenebuild.FontFallbackOptions) {
	original, ok := options.Resolver.(*surfaceFixedFallback)
	if !ok {
		panic("unexpected original fallback")
	}
	copy := *options
	copy.Resolver = &fontResolver{original: original, requests: []d2fonts.FallbackRequest{}}
	p.configureFallbackFonts(&copy)
}
func fontDocument(d *d2scene.Document) any {
	if d == nil {
		return nil
	}
	out := contentEncDocument(d).(contentObject)
	for _, v := range out["Assets"].(contentObject) {
		if font, ok := v.(contentObject)["FontAsset"]; ok {
			m := font.(contentObject)
			b, e := os.ReadFile(m["DataFile"].(string))
			if e != nil {
				panic(e)
			}
			delete(m, "DataFile")
			m["Data"] = b
		}
	}
	return out
}

func d2rsGeometryStart(t *testing.T) {
	fontRows = []any{}
	fontIDs = map[*textPainter]int{}
	contentAssetStore = filepath.Join(os.Getenv("D2RS_SCENE_API"), "..", "assets")
	t.Cleanup(func() {
		b, e := json.Marshal(map[string]any{"name": t.Name(), "operations": fontRows})
		if e != nil {
			t.Fatal(e)
		}
		if e = os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), b, 0600); e != nil {
			t.Fatal(e)
		}
	})
}
func fontErr(e error) string {
	if e != nil {
		return e.Error()
	}
	return ""
}
func fontRecord(req, got any) {
	b, e := json.Marshal(map[string]any{"request": req, "got": got})
	if e != nil {
		panic(e)
	}
	fontRows = append(fontRows, json.RawMessage(b))
}
func fontRegistry(families ...d2fonts.FontFamily) any {
	out := map[string]any{}
	for _, family := range families {
		if family == "" {
			continue
		}
		faces := map[string]any{}
		for _, style := range d2fonts.FontStyles {
			if b, ok := d2fonts.FontFaces.Lookup(d2fonts.Font{Family: family, Style: style}); ok {
				faces[string(style)] = contentEncAsset(d2scene.FontAsset{MIMEType: "font/ttf", Data: b})
			}
		}
		out[string(family)] = faces
	}
	return out
}
func fontImage(p *image.RGBA) any {
	if p == nil {
		return nil
	}
	return map[string]any{"width": p.Rect.Dx(), "height": p.Rect.Dy(), "stride": p.Stride, "pixels": base64.StdEncoding.EncodeToString(p.Pix)}
}
func fontTexture(p *textPainter, value string, style labelTextStyle) (*image.RGBA, textLayout, error) {
	if fontIDs[p] == 0 {
		fontIDs[p] = len(fontIDs) + 1
	}
	req := map[string]any{"op": "text", "id": fontIDs[p], "count": p.count, "value": value, "style": style, "primary": p.primary, "mono": p.mono, "registry": fontRegistry(p.primary, p.mono), "fallback": nil}
	var resolver *fontResolver
	if p.fallbackFonts != nil {
		var ok bool
		resolver, ok = p.fallbackFonts.Resolver.(*fontResolver)
		if !ok {
			panic("unexpected original resolver")
		}
		req["fallback"] = map[string]any{"font": resolver.original.font, "requested": resolver.original.requested}
		req["fallback"].(map[string]any)["options"] = map[string]any{"MaxAssets": p.fallbackFonts.MaxAssets, "MaxBytes": p.fallbackFonts.MaxBytes}
	}
	tex, layout, e := p.texture(value, style)
	faces := []string{}
	for key := range p.faces {
		faces = append(faces, string(key.Family)+"/"+string(key.Style))
	}
	sort.Strings(faces)
	requested := false
	if resolver != nil {
		requested = resolver.original.requested
	}
	requests := []d2fonts.FallbackRequest{}
	if resolver != nil {
		requests = resolver.requests
	}
	fontRecord(req, map[string]any{"requests": requests, "frame": fontImage(tex), "layout": layout, "error": fontErr(e), "faces": faces, "requested": requested})
	return tex, layout, e
}
func fontRich(ctx context.Context, s d2target.Shape, style labelTextStyle, families ...d2fonts.FontFamily) (*d2scene.Document, error) {
	req := map[string]any{"op": "rich", "shape": s, "style": style, "families": families, "registry": fontRegistry(families...)}
	doc, e := richLabelDocument(ctx, s, style, families...)
	fontRecord(req, map[string]any{"document": fontDocument(doc), "error": fontErr(e)})
	return doc, e
}
