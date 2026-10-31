// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/positioned-tooltip-source-v090.json
// (docs/FIXTURES.md): positioned tooltip shapes over a position x text x
// font x link grid, their target bounds, Markdown layout, and rendered SVG.
// Overlaid into d2renderers/d2svg by scripts/render_oracle_fixtures.py, which
// then extracts the callout boxes, tails, texts and viewBoxes from the SVGs.
// Inert unless D2RS_TOOLTIP_OUTPUT/D2RS_TOOLTIP_SVG_DIR are set. Harvested from
// the v0.9.0-era d2renderers/d2svg/positioned_tooltip_probe_test.go.
package d2svg

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
	"github.com/d2lang/d2/d2target"
)

func TestD2RSPositionedTooltip(t *testing.T) {
	output, directory := os.Getenv("D2RS_TOOLTIP_OUTPUT"), os.Getenv("D2RS_TOOLTIP_SVG_DIR")
	if output == "" || directory == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	font := d2fonts.SourceSansPro
	mono := d2fonts.SourceCodePro
	if err := os.MkdirAll(directory, 0o755); err != nil {
		t.Fatal(err)
	}
	texts := []string{"Tip", "**bold** and `code`", "héllo 世界 😀", "first  \nsecond", "first\n\nsecond", "# Heading\n\nbody", "- one\n- **two**", "| A | B |\n| - | - |\n| 1 | 2 |", "<unsupported>text</unsupported>", ""}
	for _, position := range []string{"top-left", "top-center", "top-right", "center-left", "center-right", "bottom-left", "bottom-center", "bottom-right"} {
		for ti, text := range texts {
			for _, family := range []string{"DEFAULT", "mono", "hand-drawn"} {
				for _, linked := range []bool{false, true} {
					name := fmt.Sprintf("%s-text%d-%s-link%t", position, ti, family, linked)
					s := d2target.Shape{ID: "probe", Type: "rectangle", Pos: d2target.Point{X: -73, Y: -41}, Width: 160, Height: 80, Opacity: 1, StrokeWidth: 3, Fill: "B3", Stroke: "B1", Tooltip: text, TooltipPosition: position, FontFamily: family}
					if linked {
						s.Link = "https://example.com"
					}
					diagram := &d2target.Diagram{Shapes: []d2target.Shape{s}, FontFamily: &font, MonoFontFamily: &mono}
					topLeft, bottomRight := diagram.BoundingBox()
					markdown := newMarkdownRenderer(&font, &mono, nil)
					layout, layoutErr := markdown.layout(text, family, d2fonts.FONT_SIZE_M)
					row := map[string]any{"name": name, "shape": s, "top_left": topLeft, "bottom_right": bottomRight, "layout": layout, "layout_error": "", "render_error": ""}
					if layoutErr != nil {
						row["layout_error"] = layoutErr.Error()
					}
					svg, err := Render(diagram, nil)
					if err != nil {
						row["render_error"] = err.Error()
					} else {
						if err = os.WriteFile(filepath.Join(directory, name+".svg"), svg, 0o644); err != nil {
							t.Fatal(err)
						}
						row["svg_file"] = name + ".svg"
					}
					rows = append(rows, row)
				}
			}
		}
	}
	data, err := json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(output, data, 0o644); err != nil {
		t.Fatal(err)
	}
}
