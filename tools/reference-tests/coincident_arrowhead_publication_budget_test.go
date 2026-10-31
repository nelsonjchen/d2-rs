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

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
	"github.com/d2lang/d2/lib/geo"
)

// Each present label reserves its own endpoint-materialization budget before
// publication. A later budget rejection retains every prior mutation.
func TestCoincidentArrowheadPublicationBudgetWitness(t *testing.T) {
	budgets := []uint64{0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 100}
	for _, limit := range budgets {
		t.Run(fmt.Sprintf("limit_%d", limit), func(t *testing.T) {
			graph := layoutgraph.NewGraph()
			source := graph.AddNode(layoutgraph.NewNode(1, 20, 20))
			target := graph.AddNode(layoutgraph.NewNode(2, 20, 20))
			source.TopLeft = geo.NewPoint(0, 0)
			target.TopLeft = geo.NewPoint(100, 0)
			edge := graph.Connect(source, target)
			edge.SourceArrowhead = layoutgraph.NoArrowhead
			edge.TargetArrowhead = layoutgraph.NoArrowhead
			edge.SourceArrowheadLabel = &layoutgraph.Label{Text: "endpoint", Width: 20, Height: 10}
			edge.TargetArrowheadLabel = &layoutgraph.Label{Text: "endpoint", Width: 20, Height: 10}
			graph.ComputeCellSize()
			sourceCenter := NewOVGNode(geo.NewPoint(10, 10))
			sourceCenter.IsNodeCenter = true
			interior := NewOVGNode(geo.NewPoint(50, 40))
			targetCenter := NewOVGNode(geo.NewPoint(110, 10))
			targetCenter.IsNodeCenter = true
			route := &Route{GEdge: edge, OVGNodes: []*OVGNode{sourceCenter, interior, targetCenter}}
			endpoints := route.createSegmentEndpoints()
			if len(endpoints) != 2 || *endpoints[0] != (geo.Point{X: 50, Y: 40}) || *endpoints[1] != (geo.Point{X: 50, Y: 40}) || endpoints[0] == endpoints[1] {
				t.Fatalf("materialized endpoints = %v, want two distinct copies of (50,40)", endpoints)
			}
			guard, err := newRouteSearchWorkGuard(context.Background(), Default, limit)
			if err != nil {
				t.Fatalf("create work guard: %v", err)
			}
			router := &ovgEdgeRouter{
				graph: graph, ovg: &OVG{}, work: guard,
				pointToRoute:      make(map[geo.Point][]*Route),
				edgeSet:           newOvgEdgeSet(),
				hasNearbyEdge:     make(map[*OVGEdge]struct{}),
				overlappingRoutes: make(map[*OVGEdge][]*Route),
			}
			err = router.addRoute(route)
			wantUsed := limit
			wantLabels, wantRoutes, wantPoints := 0, 0, 0
			switch {
			case limit < 3:
			case limit < 6:
				wantUsed = 3
			case limit < 9:
				wantUsed, wantLabels = 6, 1
			case limit < 12:
				wantLabels, wantRoutes, wantPoints = 2, 1, int(limit-9)
			default:
				wantUsed, wantLabels, wantRoutes, wantPoints = 12, 2, 1, 3
			}
			if limit < 12 {
				if !errors.Is(err, errRouteSearchWorkLimit) {
					t.Fatalf("error = %v, want work-limit rejection", err)
				}
			} else if err != nil {
				t.Fatalf("error = %v, want success", err)
			}
			if guard.used != wantUsed || len(router.positionedLabels) != wantLabels || len(router.routes) != wantRoutes || len(router.routedEdges) != wantRoutes || len(router.pointToRoute) != wantPoints {
				t.Fatalf("state = used:%d labels:%d routes:%d routedEdges:%d points:%d, want %d,%d,%d,%d,%d", guard.used, len(router.positionedLabels), len(router.routes), len(router.routedEdges), len(router.pointToRoute), wantUsed, wantLabels, wantRoutes, wantRoutes, wantPoints)
			}
			labels := make([]map[string]interface{}, 0, len(router.positionedLabels))
			for index, positioned := range router.positionedLabels {
				if math.Float64bits(positioned.TopLeft.X) != 0x7ff8000000000000 || math.Float64bits(positioned.TopLeft.Y) != 0x7ff8000000000000 {
					t.Fatalf("label %d NaN bits = %016x,%016x, want canonical positive NaN", index, math.Float64bits(positioned.TopLeft.X), math.Float64bits(positioned.TopLeft.Y))
				}
				if positioned.Edge != edge || positioned.IsTarget != (index == 1) || positioned.Text != "endpoint" || positioned.Width != 20 || positioned.Height != 10 {
					t.Fatalf("label %d has wrong source/target order or metadata: %#v", index, positioned)
				}
				labels = append(labels, map[string]interface{}{"is_target": positioned.IsTarget, "text": positioned.Text, "same_edge": positioned.Edge == edge, "width": positioned.Width, "height": positioned.Height, "x_bits": fmt.Sprintf("0x%016x", math.Float64bits(positioned.TopLeft.X)), "y_bits": fmt.Sprintf("0x%016x", math.Float64bits(positioned.TopLeft.Y))})
			}
			points := make([]map[string]interface{}, 0, len(router.pointToRoute))
			for index, node := range route.OVGNodes {
				mapped := router.pointToRoute[*node.Point]
				if index < wantPoints {
					if len(mapped) != 1 || mapped[0] != route {
						t.Fatalf("point %d mapping = %v, want this route once", index, mapped)
					}
					points = append(points, map[string]interface{}{"x": node.X, "y": node.Y, "route_count": len(mapped)})
				} else if len(mapped) != 0 {
					t.Fatalf("point %d was published before its guarded step", index)
				}
			}
			record := map[string]interface{}{"limit": limit, "used": guard.used, "success": err == nil, "error": fmt.Sprint(err), "labels": labels, "routes": len(router.routes), "routed_edges": len(router.routedEdges), "point_map": points, "segment_endpoints": []geo.Point{*endpoints[0], *endpoints[1]}}
			encoded, encodeErr := json.Marshal(record)
			if encodeErr != nil {
				t.Fatalf("encode publication state: %v", encodeErr)
			}
			t.Logf("PUBLICATION_BUDGET %s", encoded)
		})
	}
}
