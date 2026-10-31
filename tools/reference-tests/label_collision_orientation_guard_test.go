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

func collisionOrientationScalar(value float64) map[string]interface{} {
	return map[string]interface{}{"value": fmt.Sprintf("%.17g", value), "bits": fmt.Sprintf("0x%016x", math.Float64bits(value)), "is_nan": math.IsNaN(value), "is_inf": math.IsInf(value, 0)}
}

func collisionOrientationPoint(point *geo.Point) map[string]interface{} {
	return map[string]interface{}{"x": collisionOrientationScalar(point.X), "y": collisionOrientationScalar(point.Y)}
}

func collisionOrientationLog(t *testing.T, record map[string]interface{}) {
	t.Helper()
	encoded, err := json.Marshal(record)
	if err != nil {
		t.Fatalf("encode collision evidence: %v", err)
	}
	t.Logf("COLLISION_ORIENTATION %s", encoded)
}

// Qualified Go overlap and work residues for labels derived through the actual
// source-label API. This witness makes no assertion about a native Rust run.
func TestLabelCollisionOrientationGuardWitness(t *testing.T) {
	for _, test := range []struct {
		name         string
		labelRoute   geo.Route
		priorRoute   geo.Route
		wantNaNLabel bool
		wantCrossing bool
	}{
		{"finite_overflow", geo.Route{geo.NewPoint(-5, 16), geo.NewPoint(95, 16)}, geo.Route{geo.NewPoint(3e200, 3e200), geo.NewPoint(2e200, -2e200), geo.NewPoint(4e200, -2e200)}, false, false},
		{"small_coordinate_control", geo.Route{geo.NewPoint(-5, 16), geo.NewPoint(95, 16)}, geo.Route{geo.NewPoint(300, 300), geo.NewPoint(200, -200), geo.NewPoint(400, -200)}, false, false},
		{"crossing_control", geo.Route{geo.NewPoint(-5, 16), geo.NewPoint(95, 16)}, geo.Route{geo.NewPoint(10, 30), geo.NewPoint(10, -20), geo.NewPoint(30, -20)}, false, true},
		{"coincident_nan_label", geo.Route{geo.NewPoint(0, 0), geo.NewPoint(0, 0)}, geo.Route{geo.NewPoint(0, 0), geo.NewPoint(1, 0), geo.NewPoint(2, 0)}, true, false},
	} {
		t.Run(test.name, func(t *testing.T) {
			graph := layoutgraph.NewGraph()
			source := graph.AddNode(layoutgraph.NewNode(1, 20, 20))
			target := graph.AddNode(layoutgraph.NewNode(2, 20, 20))
			source.TopLeft = geo.NewPoint(0, 0)
			target.TopLeft = geo.NewPoint(100, 0)
			edge := graph.Connect(source, target)
			edge.SourceArrowhead = layoutgraph.NoArrowhead
			edge.TargetArrowhead = layoutgraph.NoArrowhead
			edge.SourceArrowheadLabel = &layoutgraph.Label{Text: "endpoint", Width: 20, Height: 10}
			edge.Points = test.labelRoute
			otherEdge := graph.Connect(source, target)
			graph.ComputeCellSize()
			positioned := labeling.PositionArrowheadLabel(edge, false, edge.Points)
			if positioned == nil || positioned.Edge != edge || positioned.IsTarget || positioned.Text != "endpoint" || positioned.Width != 20 || positioned.Height != 10 {
				t.Fatalf("source label missing or metadata changed: %#v", positioned)
			}
			if test.wantNaNLabel {
				if !math.IsNaN(positioned.TopLeft.X) || !math.IsNaN(positioned.TopLeft.Y) {
					t.Fatalf("coincident source label = %v, want NaN origin", positioned.TopLeft)
				}
			} else if *positioned.TopLeft != (geo.Point{X: 0, Y: 0}) {
				t.Fatalf("finite source label = %v, want origin (0,0)", positioned.TopLeft)
			}
			prior := &Route{GEdge: otherEdge}
			for _, point := range test.priorRoute {
				if math.IsNaN(point.X) || math.IsNaN(point.Y) || math.IsInf(point.X, 0) || math.IsInf(point.Y, 0) {
					t.Fatal("prior route input must contain only finite coordinates")
				}
				prior.OVGNodes = append(prior.OVGNodes, NewOVGNode(point))
			}
			fakeLabel := &layoutgraph.Node{Box: positioned.Box, Graph: graph}
			overlaps := make([]bool, 0, len(test.priorRoute)-1)
			for index := 1; index < len(test.priorRoute); index++ {
				overlap := fakeLabel.OverlapsLine(test.priorRoute[index-1], test.priorRoute[index], 0)
				if overlap != (test.wantCrossing && index == 1) {
					t.Fatalf("prior segment %d overlap=%v, want crossing only on the control's first segment", index-1, overlap)
				}
				overlaps = append(overlaps, overlap)
			}

			// routing.orientation and layoutgraph.orientation have the same
			// pinned source expression. Record this available private body's
			// side values independently of the exported Node overlap result.
			topLeft := positioned.TopLeft
			topRight := geo.NewPoint(positioned.TopLeft.X+positioned.Width, positioned.TopLeft.Y)
			first, second := test.priorRoute[0], test.priorRoute[1]
			sides := []float64{orientation(topLeft, topRight, first), orientation(topLeft, topRight, second), orientation(first, second, topLeft), orientation(first, second, topRight)}
			sideRecords := make([]map[string]interface{}, 0, len(sides))
			for _, side := range sides {
				sideRecords = append(sideRecords, collisionOrientationScalar(side))
			}
			if test.wantNaNLabel {
				for _, side := range sides {
					if !math.IsNaN(side) {
						t.Fatalf("NaN box orientation = %g, want NaN", side)
					}
				}
			}
			if straddlesLine(math.NaN(), math.NaN()) {
				t.Fatal("paired NaN orientations unexpectedly straddle")
			}
			inputPoints := make([]map[string]interface{}, 0, len(test.priorRoute))
			for _, point := range test.priorRoute {
				inputPoints = append(inputPoints, collisionOrientationPoint(point))
			}
			collisionOrientationLog(t, map[string]interface{}{"kind": "geometry", "case": test.name, "label_origin": collisionOrientationPoint(positioned.TopLeft), "width": positioned.Width, "height": positioned.Height, "prior_route": inputPoints, "overlaps": overlaps, "top_boundary_sides": sideRecords, "first_pair_straddles": straddlesLine(sides[0], sides[1]), "second_pair_straddles": straddlesLine(sides[2], sides[3]), "paired_nan_straddles": straddlesLine(math.NaN(), math.NaN()), "turn_cost": collisionOrientationScalar(graph.TurnCost())})

			for _, limit := range []uint64{2, 3} {
				guard, err := newRouteSearchWorkGuard(context.Background(), Default, limit)
				if err != nil {
					t.Fatalf("create work guard: %v", err)
				}
				cost, err := positionedArrowheadLabelCostGuarded(*positioned, nil, nil, []*Route{prior}, nil, guard)
				wantCost, wantUsed := 0.0, limit
				if test.wantCrossing {
					wantCost, wantUsed = graph.TurnCost(), 2
					if wantCost <= 0 {
						t.Fatal("crossing control requires a positive turn cost")
					}
				}
				if cost != wantCost || guard.used != wantUsed {
					t.Fatalf("limit%d cost=%g used=%d, want cost%g used%d", limit, cost, guard.used, wantCost, wantUsed)
				}
				if limit == 2 && !test.wantCrossing {
					if !errors.Is(err, errRouteSearchWorkLimit) {
						t.Fatalf("limit%d error=%v, want work-limit rejection", limit, err)
					}
				} else if err != nil {
					t.Fatalf("limit%d error=%v, want success", limit, err)
				}
				collisionOrientationLog(t, map[string]interface{}{"kind": "guard", "case": test.name, "limit": limit, "used": guard.used, "cost": collisionOrientationScalar(cost), "error": fmt.Sprint(err), "success": err == nil})
			}
		})
	}
	t.Run("finite_underflow_node", func(t *testing.T) {
		box := &layoutgraph.Node{Box: geo.Box{TopLeft: geo.NewPoint(0, 0), Width: 1e-200, Height: 1e-200}}
		first, second := geo.NewPoint(-1e-200, 5e-201), geo.NewPoint(2e-200, 5e-201)
		if !box.OverlapsLine(first, second, 0) {
			t.Fatal("tiny finite horizontal segment must overlap the tiny box")
		}
		rightTop, rightBottom := geo.NewPoint(1e-200, 0), geo.NewPoint(1e-200, 1e-200)
		sides := []float64{orientation(rightTop, rightBottom, first), orientation(rightTop, rightBottom, second), orientation(first, second, rightTop), orientation(first, second, rightBottom)}
		sideRecords := make([]map[string]interface{}, 0, len(sides))
		for _, side := range sides {
			if side != 0 {
				t.Fatalf("tiny finite orientation = %g, want rounded zero", side)
			}
			sideRecords = append(sideRecords, collisionOrientationScalar(side))
		}
		collisionOrientationLog(t, map[string]interface{}{"kind": "geometry", "case": "finite_underflow_node", "label_origin": collisionOrientationPoint(box.TopLeft), "width": collisionOrientationScalar(box.Width), "height": collisionOrientationScalar(box.Height), "prior_route": []map[string]interface{}{collisionOrientationPoint(first), collisionOrientationPoint(second)}, "overlap": box.OverlapsLine(first, second, 0), "right_boundary_sides": sideRecords})
	})
}
