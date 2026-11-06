// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Development reference only; never shipped as part of d2-rs.

package proximity

// Recipe `tala-labelled-graph-bounds` (tools/fixtures/tala-oracle.toml):
// layoutgraph Graph.BoundingBox and Graph.Area for one routed edge across
// route shapes, label widths, edge-label positions and percentages, and
// arrowhead/arrowhead-label combinations. Values are IEEE-754 bit strings.
// The package is only a host; the hook reads exported layoutgraph state.

import (
	"encoding/json"
	"fmt"
	"math"
	"os"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
	"github.com/d2lang/d2/lib/geo"
	"github.com/d2lang/d2/lib/label"
)

func TestD2RSLabelledGraphBounds(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	routes := [][][2]float64{nil, {{0, 0}, {100, 0}}, {{0, 0}, {0, 100}}, {{-40.25, 23.75}, {120.5, 140.25}}, {{10, 20}, {90, 20}, {90, 140}, {200, 140}}, {{13.25, -7.5}, {13.25, -7.5}}}
	positions := []label.Position{label.Unset, label.InsideMiddleLeft, label.InsideMiddleCenter, label.InsideMiddleRight, label.OutsideTopLeft, label.OutsideTopCenter, label.OutsideTopRight, label.OutsideBottomLeft, label.OutsideBottomCenter, label.OutsideBottomRight, label.UnlockedTop, label.UnlockedMiddle, label.UnlockedBottom}
	arrows := []string{"none", "", "triangle", "unfilled-triangle", "arrow", "line", "diamond", "filled-diamond", "cross", "circle", "filled-circle", "box", "filled-box", "cf-one", "cf-many", "cf-one-required", "cf-many-required", "unknown"}
	bits := func(v float64) string { return fmt.Sprintf("%016x", math.Float64bits(v)) }
	run := func(routeIndex, mode int, position label.Position, percentage, width float64, src, dst string) {
		g := layoutgraph.NewGraph()
		a := layoutgraph.NewNode(1, 10, 8)
		b := layoutgraph.NewNode(2, 14, 11)
		g.AddNode(a)
		g.AddNode(b)
		a.TopLeft = geo.NewPoint(-20.25, -17.5)
		b.TopLeft = geo.NewPoint(40.5, 8.25)
		e := layoutgraph.NewEdge(a, b)
		g.AddEdge(e)
		for _, p := range routes[routeIndex] {
			e.Points = append(e.Points, geo.NewPoint(p[0], p[1]))
		}
		e.SourceArrowhead = layoutgraph.Arrowhead(src)
		e.TargetArrowhead = layoutgraph.Arrowhead(dst)
		if mode == 0 || mode == 3 {
			e.Label = &layoutgraph.Label{Position: position, Width: width, Height: 31.75}
			e.LabelPercentage = percentage
		}
		if mode == 1 || mode == 3 {
			e.SourceArrowheadLabel = &layoutgraph.Label{Width: width, Height: 31.75}
		}
		if mode == 2 || mode == 3 {
			e.TargetArrowheadLabel = &layoutgraph.Label{Width: width, Height: 31.75}
		}
		low, high := g.BoundingBox()
		rows = append(rows, map[string]any{"route": routeIndex, "mode": mode, "position": position.String(), "percentage": percentage, "width": width, "src": src, "dst": dst, "area": bits(g.Area()), "bbox": [4]string{bits(low.X), bits(low.Y), bits(high.X), bits(high.Y)}})
	}
	for ri := range routes {
		for _, width := range []float64{20.75, 160.25} {
			for _, pos := range positions {
				for _, pct := range []float64{-0.25, 0.5, 1.25} {
					run(ri, 0, pos, pct, width, "none", "none")
				}
			}
			for mode := 1; mode <= 3; mode++ {
				for _, arrow := range arrows {
					run(ri, mode, label.UnlockedTop, 0.3, width, arrow, "none")
					run(ri, mode, label.UnlockedTop, 0.3, width, "none", arrow)
					run(ri, mode, label.UnlockedTop, 0.3, width, "diamond", arrow)
				}
			}
		}
	}
	// Two-space indentation and a final newline, as the original capture.
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
