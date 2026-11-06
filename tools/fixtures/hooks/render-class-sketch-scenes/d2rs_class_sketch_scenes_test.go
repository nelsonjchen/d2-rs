// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/class-sketch-scenes-source-v090.json
// (docs/FIXTURES.md): public d2scenebuild.Build sketch scenes of one class
// shape per field/method count x font size: rough path commands (shortest
// float text), text runs and streak rectangles. Overlaid into
// d2renderers/d2scenebuild; inert unless D2RS_RECIPE_OUTPUT is set.
// Harvested from the v0.9.0-era .reference/class-sketch-20261016/source-module/main.go.
package d2scenebuild

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"strconv"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2target"
)

func TestD2RSClassSketchScenes(t *testing.T) {
	output := os.Getenv("D2RS_RECIPE_OUTPUT")
	if output == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []any{}
	for _, counts := range [][2]int{{0, 0}, {1, 1}, {3, 0}, {0, 3}} {
		for _, size := range []int{16, 20, 30} {
			s := d2target.Shape{ID: "c", Type: d2target.ShapeClass, Pos: d2target.Point{X: 10, Y: 20}, Width: 200, Height: 120, Fill: "#112233", Stroke: "#ffffff", StrokeWidth: 2, Opacity: 1, Text: d2target.Text{Label: "C", LabelWidth: 24, LabelHeight: 25, FontSize: size, FontFamily: "default", Color: "#ffffff"}, PrimaryAccentColor: "#112233", SecondaryAccentColor: "#112233", NeutralAccentColor: "#112233"}
			for i := 0; i < counts[0]; i++ {
				s.Fields = append(s.Fields, d2target.ClassField{Name: fmt.Sprintf("f%d", i), Type: "int", Visibility: "public"})
			}
			for i := 0; i < counts[1]; i++ {
				s.Methods = append(s.Methods, d2target.ClassMethod{Name: fmt.Sprintf("m%d()", i), Return: "void", Visibility: "public"})
			}
			d := d2target.NewDiagram()
			d.Root.Fill = "#ffffff"
			d.Root.Stroke = "none"
			d.Shapes = []d2target.Shape{s}
			pad := int64(0)
			doc, err := Build(context.Background(), d, Options{Pad: &pad, Sketch: true, SketchBudget: SketchBudget{MaxOperationSets: 10000, MaxOperations: 1000000, MaxPathCommands: 1000000}})
			if err != nil {
				t.Fatal(err)
			}
			nodes := []any{}
			var walk func(*d2scene.Node)
			walk = func(n *d2scene.Node) {
				if strings.HasPrefix(n.ID, "c:") {
					switch p := n.Primitive.(type) {
					case d2scene.Path:
						nums := []float64{}
						kinds := []int{}
						for _, c := range p.Commands {
							kinds = append(kinds, int(c.Kind))
							switch c.Kind {
							case d2scene.MoveCommand, d2scene.LineCommand:
								nums = append(nums, c.P1.X, c.P1.Y)
							case d2scene.CubicCommand:
								nums = append(nums, c.P1.X, c.P1.Y, c.P2.X, c.P2.Y, c.P3.X, c.P3.Y)
							default:
								t.Fatal("unexpected command")
							}
						}
						bits := []string{}
						for _, v := range nums {
							bits = append(bits, strconv.FormatFloat(v, 'g', -1, 64))
						}
						nodes = append(nodes, map[string]any{"id": n.ID, "kind": "path", "kinds": kinds, "coordinates": bits, "fill": p.Fill, "stroke": p.Stroke, "transform": n.Transform})
					case d2scene.TextRun:
						nodes = append(nodes, map[string]any{"id": n.ID, "kind": "text", "text": p.Text, "font": p.Font, "origin": p.Origin})
					case d2scene.Rect:
						nodes = append(nodes, map[string]any{"id": n.ID, "kind": "streak", "box": p.Box, "blend": n.Blend})
					}
				}
				for _, c := range n.Children {
					walk(c)
				}
			}
			walk(doc.Root)
			rows = append(rows, map[string]any{"fields": counts[0], "methods": counts[1], "font_size": size, "nodes": nodes})
		}
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(output, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
