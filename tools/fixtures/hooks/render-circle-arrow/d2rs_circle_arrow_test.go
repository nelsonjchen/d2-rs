// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/circle-arrow-source-v090.json
// (docs/FIXTURES.md): circle arrowhead ellipse matrices and radii (float bits)
// over a kind x stroke width x side x direction x origin grid. Overlaid into
// d2renderers/d2scenebuild; inert unless D2RS_RECIPE_OUTPUT is set. Harvested
// from the v0.9.0-era d2renderers/d2scenebuild/circle_arrow_probe_test.go.
package d2scenebuild

import (
	"context"
	"encoding/json"
	"fmt"
	"image/color"
	"math"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/d2themes/d2themescatalog"
	"github.com/d2lang/d2/lib/geo"
)

func TestD2RSCircleArrow(t *testing.T) {
	output := os.Getenv("D2RS_RECIPE_OUTPUT")
	if output == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	b := &builder{ctx: context.Background(), diagram: d2target.NewDiagram(), theme: d2themescatalog.Find(0)}
	rows := []map[string]any{}
	directions := [][2]float64{{80, 0}, {-80, 0}, {0, 80}, {0, -80}, {80, 80}, {-80, 80}, {31.125, -17.375}, {80, math.Ldexp(1, -36)}}
	origins := [][2]float64{{0, 0}, {-93, -41}, {137.125, 191.75}}
	for _, name := range []d2target.Arrowhead{d2target.CircleArrowhead, d2target.FilledCircleArrowhead} {
		for _, width := range []int{0, 1, 2, 3, 4, 8, 15} {
			for side := 0; side < 2; side++ {
				for di, direction := range directions {
					for oi, origin := range origins {
						route := [][2]float64{origin, {origin[0] + direction[0], origin[1] + direction[1]}}
						endpoint := route[side]
						tangent := geo.Vector{route[1][0] - route[0][0], route[1][1] - route[0][1]}
						node, err := b.arrowhead(d2target.Connection{ID: "edge", StrokeWidth: width}, name, side == 1, d2scene.Point{X: endpoint[0], Y: endpoint[1]}, tangent, d2scene.SolidPaint{Color: color.NRGBA{R: 13, G: 50, B: 178, A: 255}})
						if err != nil {
							t.Fatal(err)
						}
						ellipse := node.Primitive.(d2scene.Ellipse)
						m := node.Transform
						values := []float64{m.A, m.C, m.B, m.D, m.E, m.F, ellipse.Center.X, ellipse.Center.Y, ellipse.RadiusX, ellipse.RadiusY}
						bits := []uint64{}
						for _, v := range values {
							bits = append(bits, math.Float64bits(v))
						}
						rows = append(rows, map[string]any{"name": fmt.Sprintf("%s-width%d-side%d-dir%d-origin%d", name, width, side, di, oi), "kind": name, "width": width, "target": side == 1, "route": route, "values": values, "bits": bits, "stroke": ellipse.Stroke})
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
