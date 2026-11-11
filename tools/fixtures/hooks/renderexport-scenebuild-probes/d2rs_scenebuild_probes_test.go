// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): d2scenebuild Markdown document links,
// retargeted to the manifest pin. Inert unless run by scripts/regen-fixtures.py.
package d2scenebuild

import (
	"context"
	"encoding/json"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/textmeasure"
)

func d2rsWrite(t *testing.T, path string, data []byte) {
	if err := os.WriteFile(path, data, 0o644); err != nil {
		t.Fatal(err)
	}
}

// tools/probes/markdown-document-scale-source-20261008.go: Markdown text shapes
// built at a scale with appendix/link budgets; layout, document links and
// viewport for the committed requests
// (tools/fixtures/inputs/renderexport/markdown-document-links-requests.json).
func TestD2RSMarkdownDocumentLinks(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var requests []struct {
		Name, Markdown, Family         string
		FontSize                       int
		Width, Height                  int
		Appendix, DisableLinks         bool
		MaxRegions, MaxStringBytes     int
		Scale                          float64
	}
	if err := json.Unmarshal(data, &requests); err != nil {
		t.Fatal(err)
	}
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	rows := []map[string]any{}
	for _, r := range requests {
		primary := d2fonts.SourceSansPro
		if r.Family == "mono" {
			primary = d2fonts.SourceCodePro
		}
		if r.Family == "hand" {
			primary = d2fonts.HandDrawn
		}
		mono := d2fonts.SourceCodePro
		layout, layoutErr := textmeasure.LayoutMarkdown(r.Markdown, ruler, &primary, &mono, r.FontSize)
		row := map[string]any{"request": r}
		if layoutErr != nil {
			row["layout_error"] = layoutErr.Error()
			rows = append(rows, row)
			continue
		}
		row["layout"] = layout
		w, h := r.Width, r.Height
		if w < 0 {
			w = layout.Width
		}
		if h < 0 {
			h = layout.Height
		}
		diagram := d2target.NewDiagram()
		diagram.FontFamily = &primary
		font := "default"
		if r.Family == "mono" {
			font = "mono"
		}
		if r.Family == "hand" {
			font = "hand"
		}
		shape := d2target.Shape{ID: "markdown", Type: d2target.ShapeText, Pos: d2target.Point{X: 0, Y: 0}, Width: w, Height: h, Fill: "N7", Stroke: "N1", StrokeWidth: 0, Opacity: 1, LabelPosition: "INSIDE_TOP_LEFT", Text: d2target.Text{Label: r.Markdown, Language: "markdown", FontFamily: font, FontSize: r.FontSize, LabelWidth: w, LabelHeight: h}}
		if r.DisableLinks {
			shape.Link = "https://outer.example.com"
			shape.PrettyLink = "outer"
		}
		diagram.Shapes = []d2target.Shape{shape}
		scale := r.Scale
		document, buildErr := Build(context.Background(), diagram, Options{Scale: &scale, Appendix: r.Appendix, LinkBudget: LinkBudget{MaxRegions: r.MaxRegions, MaxStringBytes: r.MaxStringBytes}})
		if buildErr != nil {
			row["error"] = buildErr.Error()
		} else {
			row["links"] = document.Links
			row["viewbox"] = document.ViewBox
			row["logical_width"] = document.LogicalWidth
			row["logical_height"] = document.LogicalHeight
		}
		rows = append(rows, row)
	}
	data, err = json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	d2rsWrite(t, out, append(data, '\n'))
}
