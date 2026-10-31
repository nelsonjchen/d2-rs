// SPDX-License-Identifier: MPL-2.0
// Ordinary label width controls per family/style/size (recipe mathtext-txt-ordinary-width).
package textmeasure

import (
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2fonts"
	"math"
	"os"
	"testing"
)

func TestD2RSOrdinaryWidthControls(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	texts := []string{"Tip", "héllo 世界 😀", "世界", "😀", "x😀y", "é", "e\u0301", "x\u200dy", "x\ufe0fy", "\u200d", "\u00ad", "a\u00adb", "👩‍👩‍👧‍👦", "🇺🇸", "A\n世界", "a\tb", "  世界  ", "j世界", "**text**", ""}
	for _, family := range []d2fonts.FontFamily{d2fonts.SourceSansPro, d2fonts.SourceCodePro} {
		for _, style := range []d2fonts.FontStyle{d2fonts.FONT_STYLE_REGULAR, d2fonts.FONT_STYLE_BOLD, d2fonts.FONT_STYLE_ITALIC} {
			for _, size := range []int{12, 16, 20} {
				for _, text := range texts {
					ruler, err := NewRuler()
					if err != nil {
						t.Fatal(err)
					}
					font := family.Font(size, style)
					w, h := ruler.MeasurePrecise(font, text)
					scaled := ruler.scaleUnicode(w, font, text)
					rows = append(rows, map[string]any{"family": family, "style": style, "size": size, "text": text, "raw_width": w, "raw_height": h, "legacy_width": scaled, "raw_width_bits": math.Float64bits(w), "legacy_width_bits": math.Float64bits(scaled)})
				}
			}
		}
	}
	b, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), b, 0644); err != nil {
		t.Fatal(err)
	}
}
