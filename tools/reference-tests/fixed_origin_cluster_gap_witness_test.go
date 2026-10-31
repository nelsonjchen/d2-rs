// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package placement

import (
	"context"
	"math"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/grouping"
	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
	"github.com/d2lang/d2/lib/geo"
)

func TestFixedOriginClusterGapWitness(t *testing.T) {
	for _, fixed := range []bool{true, false} {
		name := "unfixed-control"
		if fixed {
			name = "fixed-origin"
		}
		t.Run(name, func(t *testing.T) {
			ctx := withTestLogger(context.Background(), t)
			g := layoutgraph.NewGraph()
			g.CellSize = 20
			first := layoutgraph.NewNode(1, 100, 80)
			first.TopLeft = geo.NewPoint(0, 0)
			second := layoutgraph.NewNode(2, 100, 80)
			second.TopLeft = geo.NewPoint(160, 0)
			ahead := layoutgraph.NewNode(3, 100, 80)
			ahead.TopLeft = geo.NewPoint(600, 0)
			branch := layoutgraph.NewNode(4, 100, 80)
			branch.TopLeft = geo.NewPoint(0, 200)
			peer := layoutgraph.NewNode(5, 100, 80)
			peer.TopLeft = geo.NewPoint(0, 1000)
			if fixed {
				peer.FixedTopLeft = geo.NewPoint(0, 1000)
			}
			for _, n := range []*layoutgraph.Node{first, second, ahead, branch, peer} {
				g.AddNewNodeToContainer(nil, n)
			}
			vessel := layoutgraph.NewNode(6, 260, 80)
			vessel.TopLeft = geo.NewPoint(0, 0)
			vessel.SetClusterVessel(true)
			cluster := &layoutgraph.Cluster{Nodes: []*layoutgraph.Node{first, second}, Vessel: vessel, Graph: g, Arrangement: layoutgraph.Row, DesiredArrangement: layoutgraph.Row, Padding: 60}
			grouping.AddCluster(g, cluster)
			g.Connect(vessel, ahead)
			g.Connect(vessel, ahead)
			g.Connect(vessel, ahead)
			g.Connect(vessel, ahead)
			g.Connect(branch, ahead)
			g.Connect(branch, ahead)
			logGeometry := func(stage string) {
				for _, n := range append(append([]*layoutgraph.Node{}, g.Nodes...), first, second) {
					t.Logf("fixed=%v stage=%s id=%d x_bits=0x%016x y_bits=0x%016x width_bits=0x%016x height_bits=0x%016x", fixed, stage, n.ID, math.Float64bits(n.TopLeft.X), math.Float64bits(n.TopLeft.Y), math.Float64bits(n.Width), math.Float64bits(n.Height))
				}
			}
			logGeometry("before-noop")
			baseline, err := g.NewRequestTransaction(ctx, layoutgraph.TransactionOptions{AffectContainers: true})
			if err != nil {
				t.Fatal(err)
			}
			baseline.AddOp(func() error { return nil })
			if err := baseline.Commit(ctx); err != nil {
				t.Fatalf("invalid baseline: %v", err)
			}
			if *vessel.TopLeft != (geo.Point{}) || vessel.Width != 260 || vessel.Height != 80 || *ahead.TopLeft != (geo.Point{X: 600}) || *branch.TopLeft != (geo.Point{Y: 200}) || *first.TopLeft != (geo.Point{}) || *second.TopLeft != (geo.Point{X: 160}) {
				t.Fatal("no-op changed baseline")
			}
			logGeometry("after-noop")
			originalPeer := peer.TopLeft
			moved, _, err := reduceGapToNeighbors(ctx, vessel, nil, gapReductionOptions{axis: horizontalAxis, direction: forwardDirection, attemptRecoverSymmetry: false})
			if err != nil {
				t.Fatal(err)
			}
			logGeometry("after-gap")
			if moved != !fixed {
				t.Fatalf("moved=%v with fixed=%v", moved, fixed)
			}
			wantAhead, wantBranch := geo.Point{X: 410}, geo.Point{X: -190, Y: 200}
			if fixed {
				wantAhead, wantBranch = geo.Point{X: 600}, geo.Point{Y: 200}
			}
			if *ahead.TopLeft != wantAhead || *branch.TopLeft != wantBranch || peer.TopLeft != originalPeer || *peer.TopLeft != (geo.Point{Y: 1000}) || *vessel.TopLeft != (geo.Point{}) {
				t.Fatalf("unexpected result fixed=%v vessel=%+v ahead=%+v branch=%+v peer=%+v", fixed, *vessel.TopLeft, *ahead.TopLeft, *branch.TopLeft, *peer.TopLeft)
			}
		})
	}
}
