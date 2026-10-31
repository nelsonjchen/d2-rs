// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package routing

import (
	"context"
	"errors"
	"math"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/labeling"
	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
	"github.com/d2lang/d2/lib/geo"
)

// A present endpoint label remains present when its first segment has zero
// length. Its NaN geometry still enters the guarded collision inventory.
func TestCoincidentArrowheadLabelGuardWitness(t *testing.T) {
	for _, test := range []struct {
		name   string
		points geo.Route
	}{
		{"coincident_pair", geo.Route{geo.NewPoint(0, 0), geo.NewPoint(0, 0)}},
		{"leading_duplicate", geo.Route{geo.NewPoint(0, 0), geo.NewPoint(0, 0), geo.NewPoint(100, 0)}},
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
			edge.Points = test.points
			graph.ComputeCellSize()

			positioned := labeling.PositionArrowheadLabel(edge, false, edge.Points)
			if positioned == nil {
				t.Fatal("present source label was omitted")
			}
			if !math.IsNaN(positioned.TopLeft.X) || !math.IsNaN(positioned.TopLeft.Y) {
				t.Fatalf("source label top-left = (%g,%g), want (NaN,NaN)", positioned.TopLeft.X, positioned.TopLeft.Y)
			}
			if positioned.Width != 20 || positioned.Height != 10 || positioned.Edge != edge || positioned.IsTarget || positioned.Text != "endpoint" {
				t.Fatalf("source label dimensions or metadata changed: %#v", positioned)
			}

			zeroGuard, err := newRouteSearchWorkGuard(context.Background(), Default, 0)
			if err != nil {
				t.Fatalf("create zero work guard: %v", err)
			}
			zeroCost, zeroErr := positionedArrowheadLabelCostGuarded(*positioned, graph.Nodes, nil, nil, nil, zeroGuard)
			if !errors.Is(zeroErr, errRouteSearchWorkLimit) || zeroGuard.used != 0 {
				t.Fatalf("zero budget: cost=%g error=%v used=%d, want work-limit error and used=0", zeroCost, zeroErr, zeroGuard.used)
			}

			twoGuard, err := newRouteSearchWorkGuard(context.Background(), Default, 2)
			if err != nil {
				t.Fatalf("create two-unit work guard: %v", err)
			}
			cost, err := positionedArrowheadLabelCostGuarded(*positioned, graph.Nodes, nil, nil, nil, twoGuard)
			if err != nil || cost != 0 || twoGuard.used != 2 {
				t.Fatalf("two-unit budget: cost=%g error=%v used=%d, want cost=0 error=nil used=2", cost, err, twoGuard.used)
			}
			t.Logf("points=%v label=(%g,%g,%g,%g) zero=(cost=%g,error=%v,used=%d) two=(cost=%g,error=%v,used=%d)",
				edge.Points, positioned.TopLeft.X, positioned.TopLeft.Y, positioned.Width, positioned.Height,
				zeroCost, zeroErr, zeroGuard.used, cost, err, twoGuard.used)
		})
	}
}
