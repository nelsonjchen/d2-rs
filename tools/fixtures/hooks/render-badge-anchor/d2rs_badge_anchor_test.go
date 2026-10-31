// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/badge-anchor-source-v090.json
// (docs/FIXTURES.md): appendix badge anchors and viewBoxes of one rendered
// shape over a kind x metadata x size x origin x stroke grid. Overlaid into
// d2renderers/d2svg; inert unless D2RS_RECIPE_OUTPUT is set. Harvested from
// the v0.9.0-era d2renderers/d2svg/badge_anchor_probe_test.go.
package d2svg

import (
	"bytes"
	"encoding/json"
	"encoding/xml"
	"fmt"
	"io"
	"os"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
	"github.com/d2lang/d2/d2target"
)

func TestD2RSBadgeAnchor(t *testing.T) {
	output := os.Getenv("D2RS_RECIPE_OUTPUT")
	if output == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	kinds := []string{"rectangle", "square", "page", "parallelogram", "document", "cylinder", "queue", "package", "step", "callout", "stored_data", "person", "diamond", "oval", "circle", "hexagon", "cloud", "c4-person"}
	sizes := [][2]int{{160, 160}, {200, 80}, {45, 240}, {10, 10}}
	origins := [][2]int{{15, 26}, {-73, -41}, {0, 0}}
	for _, stroke := range []int{0, 1, 2, 3, 8} {
		for _, kind := range kinds {
			for _, metadata := range []string{"none", "link", "tooltip", "both"} {
				for _, size := range sizes {
					for _, origin := range origins {
						s := d2target.Shape{ID: "probe", Type: kind, Pos: d2target.Point{X: origin[0], Y: origin[1]}, Width: size[0], Height: size[1], Opacity: 1, StrokeWidth: stroke, Fill: "B3", Stroke: "B1"}
						if metadata == "link" || metadata == "both" {
							s.Link = "https://example.com"
						}
						if metadata == "tooltip" || metadata == "both" {
							s.Tooltip = "Tip"
						}
						font := d2fonts.SourceSansPro
						mono := d2fonts.SourceCodePro
						svg, err := Render(&d2target.Diagram{Shapes: []d2target.Shape{s}, FontFamily: &font, MonoFontFamily: &mono}, nil)
						if err != nil {
							t.Fatal(err)
						}
						name := fmt.Sprintf("%s-%s-%dx%d-%d-%d-stroke%d", kind, metadata, size[0], size[1], origin[0], origin[1], stroke)
						decoder := xml.NewDecoder(bytes.NewReader(svg))
						anchors := []string{}
						viewbox := ""
						viewboxes := []string{}
						for {
							token, err := decoder.Token()
							if err == io.EOF {
								break
							}
							if err != nil {
								t.Fatal(err)
							}
							if el, ok := token.(xml.StartElement); ok {
								class, transform := "", ""
								for _, a := range el.Attr {
									if a.Name.Local == "class" {
										class = a.Value
									}
									if a.Name.Local == "transform" {
										transform = a.Value
									}
									if el.Name.Local == "svg" && a.Name.Local == "viewBox" {
										viewboxes = append(viewboxes, a.Value)
										if viewbox == "" {
											viewbox = a.Value
										}
									}
								}
								if strings.Contains(" "+class+" ", " appendix-icon ") {
									anchors = append(anchors, transform)
								}
							}
						}
						want := 1
						if metadata == "both" {
							want = 2
						}
						if metadata == "none" {
							want = 0
						}
						if len(anchors) != want {
							t.Fatalf("%s anchors %v", name, anchors)
						}
						rows = append(rows, map[string]any{"name": name, "shape": kind, "metadata": metadata, "stroke": stroke, "width": size[0], "height": size[1], "x": origin[0], "y": origin[1], "anchors": anchors, "viewbox": viewbox, "viewboxes": viewboxes})
					}
				}
			}
		}
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(output, data, 0o644); err != nil {
		t.Fatal(err)
	}
}
