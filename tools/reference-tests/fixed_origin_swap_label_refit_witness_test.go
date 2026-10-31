// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package placement

import (
	"context"
	"math"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
	"github.com/d2lang/d2/lib/geo"
	"github.com/d2lang/d2/lib/label"
)

func TestFixedOriginSwapLabelRefitWitness(t *testing.T) {
	for _, fixed := range []bool{true, false} {
		name := "unfixed-control"
		if fixed {
			name = "fixed-origin"
		}
		t.Run(name, func(t *testing.T) {
			ctx := withTestLogger(context.Background(), t)
			g := layoutgraph.NewGraph()
			g.CellSize = 20
			container := layoutgraph.NewNode(1, 1160, 200)
			container.TopLeft = geo.NewPoint(0, 0)
			fixedPeer := layoutgraph.NewNode(2, 100, 80)
			fixedPeer.TopLeft = geo.NewPoint(0, 1000)
			if fixed {
				fixedPeer.FixedTopLeft = geo.NewPoint(0, 1000)
			}
			wideLabel := layoutgraph.NewNode(3, 100, 80)
			wideLabel.TopLeft = geo.NewPoint(400, 60)
			wideLabel.Label = &layoutgraph.Label{Text: "wide", Width: 800, Height: 20, Position: label.InsideMiddleCenter}
			left := layoutgraph.NewNode(4, 100, 80)
			left.TopLeft = geo.NewPoint(60, 60)
			right := layoutgraph.NewNode(5, 100, 80)
			right.TopLeft = geo.NewPoint(1000, 60)
			g.AddNewNodeToContainer(nil, container)
			g.AddNewNodeToContainer(nil, fixedPeer)
			for _, n := range []*layoutgraph.Node{wideLabel, left, right} {
				g.AddNewNodeToContainer(container, n)
			}
			g.Connect(wideLabel, fixedPeer)
			logGeometry := func(stage string) {
				for _, n := range g.Nodes {
					t.Logf("geometry fixed=%v stage=%s id=%d x_bits=0x%016x y_bits=0x%016x width_bits=0x%016x height_bits=0x%016x", fixed, stage, n.ID, math.Float64bits(n.TopLeft.X), math.Float64bits(n.TopLeft.Y), math.Float64bits(n.Width), math.Float64bits(n.Height))
				}
			}
			logGeometry("before-noop")
			baseline, err := g.NewRequestTransaction(ctx, layoutgraph.TransactionOptions{AffectContainers: true})
			if err != nil {
				t.Fatal(err)
			}
			baseline.AddOp(func() error { return nil })
			if err := baseline.Commit(ctx); err != nil {
				t.Fatalf("initial geometry is not a valid production transaction: %v", err)
			}
			if *container.TopLeft != (geo.Point{}) || container.Width != 1160 || container.Height != 200 {
				t.Fatalf("initial geometry is not refit-stable: container=%+v size=%v,%v", *container.TopLeft, container.Width, container.Height)
			}
			logGeometry("after-noop")
			originalFixed := fixedPeer.TopLeft
			txn, err := g.NewRequestTransaction(ctx, layoutgraph.TransactionOptions{AffectContainers: true})
			if err != nil {
				t.Fatal(err)
			}
			txn.AddOp(func() error { swapPositions(wideLabel, left); return nil })
			err = txn.Commit(ctx)
			logGeometry("after-swap")
			if fixed {
				if err != layoutgraph.ErrInvalidCandidate {
					t.Fatalf("fixed transaction error=%v, want ErrInvalidCandidate", err)
				}
				if *container.TopLeft != (geo.Point{}) || *wideLabel.TopLeft != (geo.Point{X: 400, Y: 60}) || *left.TopLeft != (geo.Point{X: 60, Y: 60}) {
					t.Fatalf("rejected geometry was not restored: container=%+v wide=%+v left=%+v", *container.TopLeft, *wideLabel.TopLeft, *left.TopLeft)
				}
			} else {
				if err != nil {
					t.Fatal(err)
				}
				if *container.TopLeft != (geo.Point{X: -350, Y: 0}) || container.Width != 1510 || *wideLabel.TopLeft != (geo.Point{X: 60, Y: 60}) || *left.TopLeft != (geo.Point{X: 400, Y: 60}) {
					t.Fatalf("unexpected accepted refit: container=%+v width=%v wide=%+v left=%+v", *container.TopLeft, container.Width, *wideLabel.TopLeft, *left.TopLeft)
				}
			}
			if fixedPeer.TopLeft != originalFixed || *fixedPeer.TopLeft != (geo.Point{X: 0, Y: 1000}) {
				t.Fatalf("fixed origin peer moved: %+v", *fixedPeer.TopLeft)
			}
			t.Logf("swap fixed=%v error=%v container=%+v size=%v,%v wide=%+v left=%+v fixedPeer=%+v", fixed, err, *container.TopLeft, container.Width, container.Height, *wideLabel.TopLeft, *left.TopLeft, *fixedPeer.TopLeft)
		})
	}
}
