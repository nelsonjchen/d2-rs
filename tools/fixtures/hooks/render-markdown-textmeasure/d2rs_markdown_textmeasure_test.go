// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render Markdown fixtures (docs/FIXTURES.md).
// Overlaid into lib/textmeasure by scripts/regen-fixtures.py; inert unless
// D2RS_RECIPE_INPUT names a job list produced by
// scripts/render_oracle_fixtures.py. Harvested from the v0.9.0-era
// tools/probes/markdown-*-source-*.go and markdown_flow_probe overlays.
package textmeasure

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/PuerkitoBio/goquery"

	"github.com/d2lang/d2/d2renderers/d2fonts"
)

type d2rsMarkdownJob struct {
	Output   string          `json:"output"`
	Kind     string          `json:"kind"`
	Paint    string          `json:"paint"`
	Requests json.RawMessage `json:"requests"`
}

func d2rsPaint(name string) MarkdownSVGOptions {
	roles := map[MarkdownColorRole]MarkdownSVGPaint{}
	switch name {
	case "none":
		return MarkdownSVGOptions{}
	case "table6", "full8":
		roles[MarkdownColorForeground] = MarkdownSVGPaint{Color: "#0A0F25"}
		roles[MarkdownColorAccent] = MarkdownSVGPaint{Color: "#0D32B2"}
		roles[MarkdownColorNeutralMuted] = MarkdownSVGPaint{Color: "#EEF1F8"}
		roles[MarkdownColorCanvas] = MarkdownSVGPaint{Color: "#FFFFFF"}
		roles[MarkdownColorCanvasSubtle] = MarkdownSVGPaint{Color: "#EEF1F8"}
		roles[MarkdownColorBorder] = MarkdownSVGPaint{Color: "#0D32B2"}
		if name == "table6" {
			break
		}
		roles[MarkdownColorBorderMuted] = MarkdownSVGPaint{Color: "#0D32B2"}
		roles[MarkdownColorMuted] = MarkdownSVGPaint{Color: "#6F7E8B"}
	default:
		panic("unknown paint " + name)
	}
	return MarkdownSVGOptions{RolePaint: roles}
}

func d2rsFamily(name string) d2fonts.FontFamily {
	switch name {
	case "mono":
		return d2fonts.SourceCodePro
	case "hand":
		return d2fonts.HandDrawn
	}
	return d2fonts.SourceSansPro
}

// legacy_cells: the original measureNode view of every table cell.
func d2rsCells(t *testing.T, ruler *Ruler, rendered string, primary, mono *d2fonts.FontFamily, size int) []map[string]any {
	doc, err := goquery.NewDocumentFromReader(strings.NewReader(rendered))
	if err != nil {
		t.Fatal(err)
	}
	ruler.boundsWithDot = true
	ruler.LineHeightFactor = MarkdownLineHeight
	cells := []map[string]any{}
	for _, n := range doc.Find("td,th").Nodes {
		style := d2fonts.FONT_STYLE_REGULAR
		header := n.Data == "th" || hasAncestorElement(n, "thead")
		if header {
			style = d2fonts.FONT_STYLE_SEMIBOLD
		}
		attrs := ruler.measureNode(0, n, primary, mono, size, style, header, false)
		children := []map[string]any{}
		for child := n.FirstChild; child != nil; child = child.NextSibling {
			ca := ruler.measureNode(0, child, primary, mono, size, style, header, false)
			children = append(children, map[string]any{"data": child.Data, "width": ca.width, "height": ca.height})
		}
		cells = append(cells, map[string]any{"tag": n.Data, "width": attrs.width, "height": attrs.height, "children": children})
	}
	ruler.boundsWithDot = false
	ruler.LineHeightFactor = 1
	return cells
}

func d2rsRunJob(t *testing.T, job d2rsMarkdownJob) []any {
	ruler, err := NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	rows := []any{}
	paint := d2rsPaint(job.Paint)
	switch job.Kind {
	case "natural", "cells":
		var requests []struct {
			Name, Markdown, Family string
			FontSize               int
		}
		if err := json.Unmarshal(job.Requests, &requests); err != nil {
			t.Fatal(err)
		}
		for _, request := range requests {
			primary := d2rsFamily(request.Family)
			mono := d2fonts.SourceCodePro
			layout, err := LayoutMarkdown(request.Markdown, ruler, &primary, &mono, request.FontSize)
			if err != nil {
				if job.Kind == "natural" {
					t.Fatal(err)
				}
				rows = append(rows, map[string]any{"request": request, "error": err.Error()})
				continue
			}
			rendered, err := RenderMarkdown(request.Markdown)
			if err != nil {
				t.Fatal(err)
			}
			row := map[string]any{"request": request, "rendered_html": rendered, "layout": layout}
			if job.Kind == "cells" {
				row["legacy_cells"] = d2rsCells(t, ruler, rendered, &primary, &mono, request.FontSize)
			}
			row["svg"] = layout.SVG(paint)
			rows = append(rows, row)
		}
	default:
		t.Fatalf("unknown job kind %q", job.Kind)
	}
	return rows
}

func TestD2RSMarkdownTextmeasure(t *testing.T) {
	input := os.Getenv("D2RS_RECIPE_INPUT")
	stage := os.Getenv("D2RS_RECIPE_STAGE")
	if input == "" || stage == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(input)
	if err != nil {
		t.Fatal(err)
	}
	var jobs []d2rsMarkdownJob
	if err := json.Unmarshal(data, &jobs); err != nil {
		t.Fatal(err)
	}
	for _, job := range jobs {
		encoded, err := json.Marshal(d2rsRunJob(t, job))
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(stage, job.Output), append(encoded, '\n'), 0o644); err != nil {
			t.Fatal(err)
		}
	}
}
