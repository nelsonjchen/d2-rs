// SPDX-License-Identifier: MPL-2.0
// Emitted node/command budget charges (recipe mathtext-svg-emitted-budget).
package d2svgimport

import (
	"context"
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"os"
	"testing"
)

func TestD2RSEmittedBudget(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	for _, kind := range []string{"path", "rect", "g"} {
		for _, maximum := range []int{0, 1, 2, 10} {
			for _, used := range []int{0, 1, 2} {
				for _, nodes := range []int{0, 1, 2} {
					for _, maximumNodes := range []int{0, 1, 2} {
						g := svgGeometry{}
						if kind == "path" {
							g = svgGeometry{kind: geometryPath, commands: []d2scene.PathCommand{d2scene.MoveTo(0, 0), d2scene.LineTo(1, 2)}}
						} else if kind == "rect" {
							g = svgGeometry{kind: geometryRect, box: d2scene.Box{Width: 10, Height: 20}}
						}
						i := svgImporter{ctx: context.Background(), source: "math", emittedCommands: used, emittedElements: nodes, limits: Limits{MaxPathCommands: maximum, MaxElements: maximumNodes}}
						_, e := i.instantiate(&svgElement{name: kind, attrs: map[string]string{}, transform: d2scene.Identity(), geometry: g}, defaultSVGStyle(), "", 0, false)
						message := ""
						if e != nil {
							message = e.Error()
						}
						rows = append(rows, map[string]any{"name": kind, "maximum": maximum, "used": used, "nodes": nodes, "maximum_nodes": maximumNodes, "after_commands": i.emittedCommands, "after_nodes": i.emittedElements, "error": message})
					}
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
