package routing

import (
	"context"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/labeling"
	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
	"github.com/d2lang/d2/lib/geo"
)

// Imported fixed routes collapse a serialized endpoint equal to its center.
// Label publication excludes the first and last retained OVG occurrences,
// even when Route.FromPort or Route.ToPort still names that serialized center.
func TestImportedCenterArrowheadPublicationWitness(t *testing.T) {
	for _, test := range []struct {
		name             string
		isTarget         bool
		serializedPoints []geo.Point
		wantPublished    geo.Point
		wantSerialized   geo.Point
	}{
		{
			name:             "source_center_collapses",
			serializedPoints: []geo.Point{{X: 10, Y: 10}, {X: 20, Y: 10}, {X: 100, Y: 10}},
			wantPublished:    geo.Point{X: 25, Y: -6},
			wantSerialized:   geo.Point{X: 15, Y: -6},
		},
		{
			name:             "target_center_collapses",
			isTarget:         true,
			serializedPoints: []geo.Point{{X: 20, Y: 10}, {X: 100, Y: 10}, {X: 110, Y: 10}},
			wantPublished:    geo.Point{X: 75, Y: -6},
			wantSerialized:   geo.Point{X: 85, Y: -6},
		},
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
			for _, point := range test.serializedPoints {
				edge.Points = append(edge.Points, point.Copy())
			}
			arrowheadLabel := &layoutgraph.Label{Text: "endpoint", Width: 20, Height: 10}
			if test.isTarget {
				edge.TargetArrowheadLabel = arrowheadLabel
			} else {
				edge.SourceArrowheadLabel = arrowheadLabel
			}
			graph.ComputeCellSize()

			// Exercise the real import/publication path without generating any
			// new edge routes. A repeated center pointer would be rejected by
			// addRoute, so this also witnesses the import's center collapse.
			ovg, err := routeAdditionalEdgesWithResourceLimits(
				context.Background(), graph, nil, defaultOVGBuildLimits(), maxRouteSearchWorkUnits,
			)
			if err != nil {
				t.Fatalf("import fixed route: %v", err)
			}
			wantRaw := []geo.Point{{X: 10, Y: 10}, {X: 20, Y: 10}, {X: 100, Y: 10}, {X: 110, Y: 10}}
			rawNodes := make([]*OVGNode, 0, len(wantRaw))
			seen := make(map[*OVGNode]bool, len(wantRaw))
			for index, point := range wantRaw {
				node := ovg.OccupiedPoints[point]
				if node == nil || *node.Point != point {
					t.Fatalf("raw node %d = %v, want %v", index, node, point)
				}
				if seen[node] {
					t.Fatalf("raw node %d repeats a retained OVG identity", index)
				}
				seen[node] = true
				rawNodes = append(rawNodes, node)
			}
			if rawNodes[0] != ovg.Centers[source] || rawNodes[3] != ovg.Centers[target] {
				t.Fatal("raw endpoints do not carry source and target center identities")
			}
			route := &Route{
				GEdge: edge, OVGNodes: rawNodes,
				FromPort: *edge.Points[0], ToPort: *edge.Points[len(edge.Points)-1],
			}
			endpoints := route.createSegmentEndpoints()
			wantEndpoints := []geo.Point{{X: 20, Y: 10}, {X: 100, Y: 10}}
			if len(endpoints) != len(wantEndpoints) {
				t.Fatalf("segment endpoints have length %d, want %d", len(endpoints), len(wantEndpoints))
			}
			for index, point := range endpoints {
				if *point != wantEndpoints[index] {
					t.Fatalf("segment endpoint %d = %v, want %v", index, *point, wantEndpoints[index])
				}
			}

			router, err := newOVGEdgeRouter(context.Background(), Default, ovg, graph, nil, nil)
			if err != nil {
				t.Fatalf("prepare publication router: %v", err)
			}
			if err := router.addRoute(route); err != nil {
				t.Fatalf("publish retained raw route: %v", err)
			}
			if len(router.positionedLabels) != 1 {
				t.Fatalf("positioned labels have length %d, want 1", len(router.positionedLabels))
			}
			published := router.positionedLabels[0]
			if *published.TopLeft != test.wantPublished || published.Width != 20 || published.Height != 10 {
				t.Fatalf("published box = (%v, %v x %v), want (%v, 20 x 10)", *published.TopLeft, published.Width, published.Height, test.wantPublished)
			}
			if published.Edge != edge || published.IsTarget != test.isTarget || published.Text != "endpoint" {
				t.Fatalf("published label metadata = %#v", published)
			}
			serialized := labeling.PositionArrowheadLabel(edge, test.isTarget, edge.Points)
			if serialized == nil || *serialized.TopLeft != test.wantSerialized {
				t.Fatalf("serialized-route label = %#v, want top-left %v", serialized, test.wantSerialized)
			}
			t.Logf("raw=%v endpoints=%v published=(%g,%g,20,10) serialized=(%g,%g,20,10)",
				wantRaw, wantEndpoints, published.TopLeft.X, published.TopLeft.Y, serialized.TopLeft.X, serialized.TopLeft.Y)
		})
	}
}
