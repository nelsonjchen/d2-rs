// SPDX-License-Identifier: MPL-2.0
// Clip-path compile stages and clip resource trees
// (recipes mathtext-svg-clip, mathtext-svg-clip-tree).
package d2svgimport

import (
	"context"
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"os"
	"testing"
	"time"
)

func TestD2RSClipStages(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	for _, kind := range []svgGeometryKind{geometryNone, geometryRect, geometryEllipse, geometryPath} {
		for _, radius := range []float64{0, 3} {
			for _, width := range []float64{0, 10} {
				g := svgGeometry{kind: kind, box: d2scene.Box{X: 1, Y: 2, Width: width, Height: 20}, center: d2scene.Point{X: 1, Y: 2}, radiusX: radius, radiusY: 4, commands: []d2scene.PathCommand{d2scene.MoveTo(1, 2), d2scene.LineTo(3, 4)}}
				for _, mode := range []string{"", "cancelled", "deadline"} {
					ctx := context.Background()
					if mode == "cancelled" {
						var c context.CancelFunc
						ctx, c = context.WithCancel(ctx)
						c()
					} else if mode == "deadline" {
						var c context.CancelFunc
						ctx, c = context.WithDeadline(ctx, time.Unix(0, 0))
						defer c()
					}
					i := svgImporter{ctx: ctx}
					commands, e := i.clipCommands(&svgElement{name: "probe", geometry: g})
					message := ""
					if e != nil {
						message = e.Error()
					}
					rows = append(rows, map[string]any{"operation": "commands", "geometry": d2rsGeometry(g), "cancel": mode, "commands": d2rsCommands(commands), "error": message})
				}
			}
		}
	}
	for _, id := range []string{"", "missing", "c"} {
		for _, maximum := range []int{0, 1, 2, 10} {
			for _, used := range []int{0, 1, 2} {
				for _, maximumCommands := range []int{0, 1, 2, 10} {
					i := svgImporter{ctx: context.Background(), source: "math", useInstances: used, emittedCommands: used, limits: Limits{MaxResources: maximum, MaxPathCommands: maximumCommands}, ids: map[string]*svgElement{"c": {name: "clipPath", clipPath: &compiledClipPath{path: d2scene.Path{Commands: []d2scene.PathCommand{d2scene.MoveTo(1, 2), d2scene.LineTo(3, 4)}, FillRule: d2scene.EvenOdd}, transform: d2scene.Identity(), useInstances: 2}}}}
					clip, e := i.instantiateClipPath(id)
					message := ""
					if e != nil {
						message = e.Error()
					}
					var output any
					if clip != nil {
						output = map[string]any{"commands": d2rsCommands(clip.Path.Commands), "fill_rule": clip.Path.FillRule, "transform": [6]float64{clip.Transform.A, clip.Transform.B, clip.Transform.C, clip.Transform.D, clip.Transform.E, clip.Transform.F}, "uses": 2}
					}
					rows = append(rows, map[string]any{"operation": "instantiate", "id": id, "maximum": maximum, "used": used, "maximum_commands": maximumCommands, "after_uses": i.useInstances, "after_commands": i.emittedCommands, "output": output, "error": message})
				}
			}
		}
	}
	data, e := json.MarshalIndent(rows, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); e != nil {
		t.Fatal(e)
	}
}
func TestD2RSClipTree(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	for _, parent := range []string{"svg", "g", "defs", "path"} {
		for _, id := range []string{"", "c"} {
			for _, child := range []string{"absent", "path", "use", "g"} {
				root := &svgElement{name: parent}
				clip := &svgElement{name: "clipPath", id: id}
				root.children = []*svgElement{clip}
				if child != "absent" {
					clip.children = []*svgElement{{name: child}}
				}
				i := svgImporter{ctx: context.Background(), source: "math"}
				e := i.validateClipTree(root)
				message := ""
				if e != nil {
					message = e.Error()
				}
				rows = append(rows, map[string]any{"parent": parent, "id": id, "child": child, "error": message})
			}
		}
	}
	data, e := json.MarshalIndent(rows, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); e != nil {
		t.Fatal(e)
	}
}
