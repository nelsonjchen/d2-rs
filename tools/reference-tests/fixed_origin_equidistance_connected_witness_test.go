// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package placement

import (
	"context"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/limits"
	"github.com/d2lang/d2/lib/geo"
)

func TestFixedOriginEquidistanceConnectedBranchWitness(t *testing.T) {
	for _, fixed := range []bool{true, false} {
		name := "unfixed-control"
		if fixed {
			name = "fixed-origin"
		}
		t.Run(name, func(t *testing.T) {
			ctx := withTestLogger(context.Background(), t)
			g := layoutgraph.NewGraph()
			g.CellSize = 20
			left := layoutgraph.NewNode(1, 100, 80)
			left.TopLeft = geo.NewPoint(0, 0)
			if fixed {
				left.FixedTopLeft = geo.NewPoint(0, 0)
			}
			center := layoutgraph.NewNode(2, 100, 80)
			center.TopLeft = geo.NewPoint(600, 0)
			right := layoutgraph.NewNode(3, 100, 80)
			right.TopLeft = geo.NewPoint(1000, 0)
			perpendicular := layoutgraph.NewNode(4, 100, 80)
			perpendicular.TopLeft = geo.NewPoint(600, 200)
			branch := layoutgraph.NewNode(5, 100, 80)
			branch.TopLeft = geo.NewPoint(0, 200)
			for _, n := range []*layoutgraph.Node{left, center, right, perpendicular, branch} {
				g.AddNode(n)
			}
			g.Connect(left, center)
			g.Connect(center, right)
			g.Connect(center, perpendicular)
			g.Connect(branch, perpendicular)
			guard, err := limits.NewWorkGuard(ctx, "FixedOriginEquidistanceWitness", limits.MaxEngineWorkUnits)
			if err != nil {
				t.Fatal(err)
			}
			moved, err := equidistanceNodeGuarded(ctx, center, g, true, guard)
			if err != nil {
				t.Fatal(err)
			}
			t.Logf("equidistance result fixed=%v moved=%v left=%+v center=%+v right=%+v perpendicular=%+v branch=%+v", fixed, moved, *left.TopLeft, *center.TopLeft, *right.TopLeft, *perpendicular.TopLeft, *branch.TopLeft)
			if !moved {
				t.Fatalf("equidistance moved=%v with fixed=%v; want true", moved, fixed)
			}
			wantCenter := geo.Point{X: 500, Y: 0}
			wantPerpendicular := geo.Point{X: 500, Y: 200}
			wantBranch := geo.Point{X: -100, Y: 200}
			if fixed {
				wantPerpendicular = geo.Point{X: 600, Y: 200}
				wantBranch = geo.Point{X: 0, Y: 200}
			}
			if *left.TopLeft != (geo.Point{X: 0, Y: 0}) || *right.TopLeft != (geo.Point{X: 1000, Y: 0}) || *center.TopLeft != wantCenter || *perpendicular.TopLeft != wantPerpendicular || *branch.TopLeft != wantBranch {
				t.Fatalf("equidistance fixed=%v left=%+v center=%+v right=%+v perpendicular=%+v branch=%+v", fixed, *left.TopLeft, *center.TopLeft, *right.TopLeft, *perpendicular.TopLeft, *branch.TopLeft)
			}
			t.Logf("equidistance fixed=%v moved=%v left=%+v center=%+v right=%+v perpendicular=%+v branch=%+v", fixed, moved, *left.TopLeft, *center.TopLeft, *right.TopLeft, *perpendicular.TopLeft, *branch.TopLeft)
		})
	}
}
