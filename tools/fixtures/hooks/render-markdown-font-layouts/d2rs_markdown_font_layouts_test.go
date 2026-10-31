// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/markdown-font-layout-source-v090.json
// and markdown-heading-layout-source-v090.json (docs/FIXTURES.md): Markdown
// LayoutMarkdown/MeasureMarkdown geometry plus CSS font-box metrics per family
// x size x source. Overlaid into lib/textmeasure; inert unless
// D2RS_RECIPE_OUTPUTS is set. Harvested from the v0.9.0-era
// lib/textmeasure/markdown_{font_baseline,heading_font}_probe_test.go.
package textmeasure

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
)

func TestD2RSMarkdownFontLayouts(t *testing.T) {
	outputs := os.Getenv("D2RS_RECIPE_OUTPUTS")
	if outputs == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type control struct {
		Family  string          `json:"family"`
		Size    int             `json:"size"`
		Source  string          `json:"source"`
		Width   int             `json:"width"`
		Height  int             `json:"height"`
		Ascent  float64         `json:"ascent"`
		Descent float64         `json:"descent"`
		Layout  *MarkdownLayout `json:"layout"`
	}
	headings := []string{}
	for _, body := range []string{" Agm", " `Agm`"} {
		for level := 1; level <= 6; level++ {
			headings = append(headings, strings.Repeat("#", level)+body)
		}
	}
	grids := [][]string{
		{"`Agm`", "**`Agm`**", "*`Agm`*", "plain `Agm` text", "Agm", "`Agm`  \n`gy`"},
		headings,
	}
	paths := filepath.SplitList(outputs)
	if len(paths) != len(grids) {
		t.Fatalf("want %d outputs, got %d", len(grids), len(paths))
	}
	for i, sources := range grids {
		rows := []control{}
		for _, family := range []d2fonts.FontFamily{d2fonts.SourceSansPro, d2fonts.SourceCodePro, d2fonts.HandDrawn} {
			for _, size := range []int{12, 16, 23, 32} {
				ruler, err := NewRuler()
				if err != nil {
					t.Fatal(err)
				}
				for _, source := range sources {
					mono := d2fonts.SourceCodePro
					layout, err := LayoutMarkdown(source, ruler, &family, &mono, size)
					if err != nil {
						t.Fatal(err)
					}
					width, height, err := MeasureMarkdown(source, ruler, &family, &mono, size)
					if err != nil {
						t.Fatal(err)
					}
					ascent, descent, ok := ruler.cssFontBoxMetrics(family.Font(size, d2fonts.FONT_STYLE_REGULAR), float64(size))
					if !ok {
						t.Fatal("font metrics unavailable")
					}
					rows = append(rows, control{string(family), size, source, width, height, ascent, descent, layout})
				}
			}
		}
		data, err := json.MarshalIndent(rows, "", "  ")
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(paths[i], append(data, '\n'), 0o644); err != nil {
			t.Fatal(err)
		}
	}
}
