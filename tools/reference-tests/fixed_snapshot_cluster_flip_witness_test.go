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
	"github.com/d2lang/d2/lib/shape"
)

func TestFixedSnapshotClusterFlipWitness(t *testing.T) {
	for _, fixed := range []bool{true, false} {
		name := "unfixed-control"
		if fixed {
			name = "fixed-snapshot"
		}
		t.Run(name, func(t *testing.T) {
			ctx := withTestLogger(context.Background(), t)
			g := layoutgraph.NewGraph()
			g.CellSize = 20
			container := layoutgraph.NewNode(1, 760, 340)
			container.TopLeft = geo.NewPoint(0, 0)
			container.SetShape(shape.CIRCLE_TYPE)
			first := layoutgraph.NewNode(2, 60, 80)
			first.TopLeft = geo.NewPoint(60, 60)
			second := layoutgraph.NewNode(3, 60, 80)
			second.TopLeft = geo.NewPoint(60, 200)
			peer := layoutgraph.NewNode(4, 100, 80)
			peer.TopLeft = geo.NewPoint(600, 60)
			if fixed {
				peer.FixedTopLeft = geo.NewPoint(540, 0)
			}
			g.AddNewNodeToContainer(nil, container)
			for _, n := range []*layoutgraph.Node{first, second, peer} {
				g.AddNewNodeToContainer(container, n)
			}
			vessel := layoutgraph.NewNode(5, 60, 220)
			vessel.TopLeft = geo.NewPoint(60, 60)
			vessel.SetClusterVessel(true)
			cluster := &layoutgraph.Cluster{Nodes: []*layoutgraph.Node{first, second}, Container: container, Vessel: vessel, Graph: g, Arrangement: layoutgraph.Column, DesiredArrangement: layoutgraph.Row, Padding: 60}
			grouping.AddCluster(g, cluster)
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
			stablePosition, stableWidth, stableHeight := *container.TopLeft, container.Width, container.Height
			stablePeer := *peer.TopLeft
			baseline2, err := g.NewRequestTransaction(ctx, layoutgraph.TransactionOptions{AffectContainers: true})
			if err != nil {
				t.Fatal(err)
			}
			baseline2.AddOp(func() error { return nil })
			if err := baseline2.Commit(ctx); err != nil {
				t.Fatalf("invalid second baseline: %v", err)
			}
			if *container.TopLeft != stablePosition || container.Width != stableWidth || container.Height != stableHeight || *peer.TopLeft != stablePeer {
				t.Fatal("second no-op changed baseline")
			}

			logGeometry("after-noop")
			changed, err := optimizeCluster(ctx, cluster, true)
			if err != nil {
				t.Fatal(err)
			}
			logGeometry("after-flip")
			if fixed {
				if changed || cluster.Arrangement != layoutgraph.Column || *peer.TopLeft != stablePeer || *container.TopLeft != stablePosition || container.Width != stableWidth || container.Height != stableHeight {
					t.Fatalf("fixed flip changed baseline: changed=%v arrangement=%v peer=%+v container=%+v size=%v,%v", changed, cluster.Arrangement, *peer.TopLeft, *container.TopLeft, container.Width, container.Height)
				}
			} else {
				if !changed || cluster.Arrangement != layoutgraph.Row || *peer.TopLeft == stablePeer {
					t.Fatalf("unfixed flip did not move sibling: changed=%v arrangement=%v peer=%+v", changed, cluster.Arrangement, *peer.TopLeft)
				}
			}

		})
	}
}
