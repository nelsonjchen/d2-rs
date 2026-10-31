// SPDX-License-Identifier: MPL-2.0
// Ruler bounds with the dot glyph per family/style/size (recipe mathtext-txt-bounds-dot).
package textmeasure

import (
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2fonts"
	"os"
	"testing"
)

func TestD2RSBoundsDot(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type row struct {
		Family string  `json:"family"`
		Style  string  `json:"style"`
		Size   int     `json:"size"`
		Text   string  `json:"text"`
		Width  float64 `json:"width"`
		Height float64 `json:"height"`
	}
	rows := []row{}
	ruler, err := NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	ruler.boundsWithDot = true
	for _, family := range []d2fonts.FontFamily{d2fonts.SourceSansPro, d2fonts.SourceCodePro, d2fonts.HandDrawn} {
		for _, style := range []d2fonts.FontStyle{d2fonts.FONT_STYLE_REGULAR, d2fonts.FONT_STYLE_SEMIBOLD, d2fonts.FONT_STYLE_BOLD, d2fonts.FONT_STYLE_ITALIC} {
			for _, size := range []int{12, 16, 23, 32} {
				for _, text := range []string{"Agm", "jA", "fA", "Tj"} {
					width, height := ruler.MeasurePrecise(family.Font(size, style), text)
					rows = append(rows, row{string(family), string(style), size, text, width, height})
				}
			}
		}
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); err != nil {
		t.Fatal(err)
	}
}
