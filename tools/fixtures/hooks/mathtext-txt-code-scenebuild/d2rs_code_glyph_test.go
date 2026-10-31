// SPDX-License-Identifier: MPL-2.0
// Source Code Pro glyph advances and the code ruler over Unicode samples
// (recipes mathtext-txt-code-glyph-metrics, mathtext-txt-code-ruler-unicode).
package d2scenebuild

import (
	"context"
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2fonts"
	"github.com/d2lang/d2/lib/textmeasure"
	"golang.org/x/image/font/sfnt"
	"math"
	"os"
	"testing"
	"time"
)

func TestD2RSCodeGlyphMetrics(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type Request struct {
		Font   string  `json:"font"`
		Size   float64 `json:"size"`
		Text   string  `json:"text"`
		Cancel string  `json:"cancel"`
	}
	var requests []Request
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	if err = json.Unmarshal(data, &requests); err != nil {
		t.Fatal(err)
	}
	var out []map[string]any
	for _, r := range requests {
		style := d2fonts.FONT_STYLE_REGULAR
		if r.Font == "bold" {
			style = d2fonts.FONT_STYLE_BOLD
		}
		if r.Font == "italic" {
			style = d2fonts.FONT_STYLE_ITALIC
		}
		bytes, ok := d2fonts.FontFaces.Lookup(d2fonts.Font{Family: d2fonts.SourceCodePro, Style: style})
		if !ok {
			t.Fatal("font")
		}
		face, err := sfnt.Parse(bytes)
		if err != nil {
			t.Fatal(err)
		}
		metrics := codeFontMetrics{font: face}
		ctx, cancel := context.WithCancel(context.Background())
		if r.Cancel == "before" {
			cancel()
		}
		if r.Cancel == "deadline_before" {
			var c context.CancelFunc
			ctx, c = context.WithDeadline(ctx, time.Unix(0, 0))
			defer c()
		}
		advance, err := metrics.advance(ctx, r.Text, r.Size)
		message := ""
		if err != nil {
			message = err.Error()
		}
		out = append(out, map[string]any{"request": r, "error": message, "advance": advance, "advance_bits": math.Float64bits(advance)})
		cancel()
	}
	data, err = json.MarshalIndent(out, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
func TestD2RSCodeRulerUnicode(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	var out []map[string]any
	for _, size := range []int{8, 16, 23, 32} {
		for _, value := range []string{"🙂", "a🙂", "var x = \"🙂\"", "x = \"🙂\"", "a\t🙂", "AV", "α", "first\n🙂", "🙂\nlast", "á", "漢字", "a 🙂"} {
			ruler.LineHeightFactor = textmeasure.CODE_LINE_HEIGHT
			w, h := ruler.MeasureMono(d2fonts.SourceCodePro.Font(size, d2fonts.FONT_STYLE_REGULAR), value)
			out = append(out, map[string]any{"text": value, "size": size, "width": w, "height": h})
		}
	}
	data, _ := json.MarshalIndent(out, "", "  ")
	if err = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
