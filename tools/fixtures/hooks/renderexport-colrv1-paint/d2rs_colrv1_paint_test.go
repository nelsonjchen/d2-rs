// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): complete d2raster frames for bundled Noto
// Color Emoji COLRv1 glyphs at several sizes. Harvested from the v0.9.0
// probe colrv1_paint_probe_test.go; inert unless run by regen-fixtures.py.
package d2raster

import (
	"context"
	"encoding/json"
	"image/color"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
	"github.com/d2lang/d2/d2renderers/d2scene"
)

func TestD2RSCOLRv1Paint(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	resolver, err := d2fonts.NewBundledFallbackResolver(nil, d2fonts.BundledFallbackLimits{MaxRequestedRunes: 16, MaxBundledBytes: 8 * 1024 * 1024, MaxResolvedBytes: 8 * 1024 * 1024})
	if err != nil {
		t.Fatal(err)
	}
	fonts, err := resolver.ResolveFallbacks(context.Background(), d2fonts.FallbackRequest{Runes: []rune("😀")})
	if err != nil {
		t.Fatal(err)
	}
	rows := []any{}
	for _, size := range []float64{10, 16, 13.6, 32} {
		for _, glyph := range []uint32{3676, 1875, 3744, 1215, 168, 1829} {
			const id = d2scene.AssetID("noto")
			doc := d2scene.NewDocument(d2scene.Box{Width: 96, Height: 96}, d2scene.NewNode(d2scene.TextRun{Origin: d2scene.Point{X: 8.25, Y: 48.75}, Font: d2scene.Font{Family: "Noto Color Emoji", Size: size, Asset: id}, Fill: d2scene.SolidPaint{Color: color.NRGBA{A: 255}}, Glyphs: []d2scene.Glyph{{ID: glyph, Advance: 40}}}))
			doc.Assets[id] = d2scene.FontAsset{MIMEType: "font/ttf", Data: fonts[0].Data}
			options := testOptions()
			options.MaxNodes = 20000
			frame, err := Render(context.Background(), doc, options)
			row := map[string]any{"size": size, "glyph_id": glyph, "error": ""}
			if err != nil {
				row["error"] = err.Error()
			} else {
				row["width"] = frame.Bounds().Dx()
				row["height"] = frame.Bounds().Dy()
				row["pixels"] = frame.Pix
			}
			rows = append(rows, row)
		}
	}
	b, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, b, 0o644); err != nil {
		t.Fatal(err)
	}
}
