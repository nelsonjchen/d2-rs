// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package routing

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"math"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/labeling"
	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
	"github.com/d2lang/d2/lib/geo"
)

func finiteSubtractionScalar(value float64) map[string]interface{} {
	return map[string]interface{}{"value": fmt.Sprintf("%.17g", value), "bits": fmt.Sprintf("0x%016x", math.Float64bits(value)), "is_nan": math.IsNaN(value), "is_inf": math.IsInf(value, 0)}
}

func finiteSubtractionLog(t *testing.T, record map[string]interface{}) {
	t.Helper()
	encoded, err := json.Marshal(record)
	if err != nil {
		t.Fatalf("encode finite subtraction evidence: %v", err)
	}
	t.Logf("FINITE_SUBTRACTION %s", encoded)
}

// Finite endpoints can overflow an intermediate subtraction, producing paired
// NaN side values while the entire route remains outside the label box.
func TestLabelCollisionFiniteSubtractionGuardWitness(t *testing.T) {
	graph := layoutgraph.NewGraph()
	source := graph.AddNode(layoutgraph.NewNode(1, 20, 20))
	target := graph.AddNode(layoutgraph.NewNode(2, 20, 20))
	source.TopLeft = geo.NewPoint(0, 0)
	target.TopLeft = geo.NewPoint(100, 0)
	edge := graph.Connect(source, target)
	edge.SourceArrowhead = layoutgraph.NoArrowhead
	edge.TargetArrowhead = layoutgraph.NoArrowhead
	edge.SourceArrowheadLabel = &layoutgraph.Label{Text: "endpoint", Width: 20, Height: 10}
	edge.Points = geo.Route{geo.NewPoint(-5, 16), geo.NewPoint(95, 16)}
	otherEdge := graph.Connect(source, target)
	graph.ComputeCellSize()
	positioned := labeling.PositionArrowheadLabel(edge, false, edge.Points)
	if positioned == nil || *positioned.TopLeft != (geo.Point{X: 0, Y: 0}) || positioned.Width != 20 || positioned.Height != 10 || positioned.Edge != edge || positioned.IsTarget || positioned.Text != "endpoint" {
		t.Fatalf("source label = %#v, want (0,0,20,10) and preserved source metadata", positioned)
	}
	points := geo.Route{geo.NewPoint(1e308, 1e308), geo.NewPoint(-1e308, 5e307), geo.NewPoint(-5e307, 1e308)}
	prior := &Route{GEdge: otherEdge}
	pointRecords := make([]map[string]interface{}, 0, len(points))
	for _, point := range points {
		if math.IsNaN(point.X) || math.IsNaN(point.Y) || math.IsInf(point.X, 0) || math.IsInf(point.Y, 0) {
			t.Fatal("prior route must contain only finite input coordinates")
		}
		prior.OVGNodes = append(prior.OVGNodes, NewOVGNode(point))
		pointRecords = append(pointRecords, map[string]interface{}{"x": finiteSubtractionScalar(point.X), "y": finiteSubtractionScalar(point.Y)})
	}
	fakeLabel := &layoutgraph.Node{Box: positioned.Box, Graph: graph}
	overlaps := make([]bool, 0, 2)
	for index := 1; index < len(points); index++ {
		overlap := fakeLabel.OverlapsLine(points[index-1], points[index], 0)
		if overlap {
			t.Fatalf("segment %d unexpectedly overlaps source label", index-1)
		}
		overlaps = append(overlaps, overlap)
	}
	rightTop, rightBottom := geo.NewPoint(20, 0), geo.NewPoint(20, 10)
	sides := []float64{orientation(rightTop, rightBottom, points[0]), orientation(rightTop, rightBottom, points[1]), orientation(points[0], points[1], rightTop), orientation(points[0], points[1], rightBottom)}
	if !math.IsInf(sides[0], 1) || !math.IsInf(sides[1], -1) || !math.IsNaN(sides[2]) || !math.IsNaN(sides[3]) {
		t.Fatalf("right boundary sides = %v, want [+Inf,-Inf,NaN,NaN]", sides)
	}
	if !straddlesLine(sides[0], sides[1]) || straddlesLine(sides[2], sides[3]) {
		t.Fatal("infinite opposite sides must straddle; paired NaNs must not")
	}
	sideRecords := make([]map[string]interface{}, 0, len(sides))
	for _, side := range sides {
		sideRecords = append(sideRecords, finiteSubtractionScalar(side))
	}
	finiteSubtractionLog(t, map[string]interface{}{"kind": "geometry", "label_origin": []float64{positioned.TopLeft.X, positioned.TopLeft.Y}, "width": positioned.Width, "height": positioned.Height, "prior_route": pointRecords, "overlaps": overlaps, "right_boundary_sides": sideRecords, "x_subtraction": finiteSubtractionScalar(points[1].X - points[0].X), "first_pair_straddles": straddlesLine(sides[0], sides[1]), "second_pair_straddles": straddlesLine(sides[2], sides[3])})
	for _, limit := range []uint64{2, 3} {
		guard, err := newRouteSearchWorkGuard(context.Background(), Default, limit)
		if err != nil {
			t.Fatalf("create work guard: %v", err)
		}
		cost, err := positionedArrowheadLabelCostGuarded(*positioned, nil, nil, []*Route{prior}, nil, guard)
		if cost != 0 || guard.used != limit {
			t.Fatalf("limit%d cost=%g used=%d, want cost0 used%d", limit, cost, guard.used, limit)
		}
		if limit == 2 {
			if !errors.Is(err, errRouteSearchWorkLimit) {
				t.Fatalf("limit2 error=%v, want work-limit rejection", err)
			}
		} else if err != nil {
			t.Fatalf("limit3 error=%v, want success", err)
		}
		finiteSubtractionLog(t, map[string]interface{}{"kind": "guard", "limit": limit, "used": guard.used, "cost": finiteSubtractionScalar(cost), "error": fmt.Sprint(err), "success": err == nil})
	}
}
