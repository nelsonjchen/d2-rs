// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package placement

import (
	"context"
	"errors"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
	"github.com/d2lang/d2/lib/geo"
)

func newFixedOriginGapConnectedWitness(fixed bool) (*layoutgraph.Graph, *layoutgraph.Node, *layoutgraph.Node, *layoutgraph.Node) {
	g := layoutgraph.NewGraph()
	g.CellSize = 20
	receiver := layoutgraph.NewNode(1, 100, 80)
	receiver.TopLeft = geo.NewPoint(0, 0)
	if fixed {
		receiver.FixedTopLeft = geo.NewPoint(0, 0)
	}
	ahead := layoutgraph.NewNode(2, 100, 80)
	ahead.TopLeft = geo.NewPoint(600, 0)
	branch := layoutgraph.NewNode(3, 100, 80)
	branch.TopLeft = geo.NewPoint(0, 200)
	g.AddNode(receiver)
	g.AddNode(ahead)
	g.AddNode(branch)
	g.Connect(receiver, ahead)
	g.Connect(branch, ahead)
	return g, receiver, ahead, branch
}

func TestFixedOriginGapConnectedBranchWitness(t *testing.T) {
	for _, fixed := range []bool{true, false} {
		name := "unfixed-control"
		if fixed {
			name = "fixed-origin"
		}
		t.Run(name, func(t *testing.T) {
			ctx := withTestLogger(context.Background(), t)
			g, receiver, ahead, branch := newFixedOriginGapConnectedWitness(fixed)
			originalPointers := []*geo.Point{receiver.TopLeft, ahead.TopLeft, branch.TopLeft}
			originalPoints := []geo.Point{*receiver.TopLeft, *ahead.TopLeft, *branch.TopLeft}
			txn, err := g.NewRequestTransaction(ctx, layoutgraph.TransactionOptions{AffectContainers: true})
			if err != nil {
				t.Fatal(err)
			}
			txn.AddOp(func() error {
				ahead.Translate(-350, 0)
				branch.Translate(-350, 0)
				t.Logf("transaction trial fixed=%v receiver=%+v ahead=%+v branch=%+v", fixed, *receiver.TopLeft, *ahead.TopLeft, *branch.TopLeft)
				return nil
			})
			err = txn.Commit(ctx)
			if fixed {
				if !errors.Is(err, layoutgraph.ErrInvalidCandidate) {
					t.Fatalf("fixed-origin transaction error = %v; want ErrInvalidCandidate", err)
				}
				for i, n := range []*layoutgraph.Node{receiver, ahead, branch} {
					if n.TopLeft != originalPointers[i] || *n.TopLeft != originalPoints[i] {
						t.Fatalf("node %d rollback = %p %+v; want %p %+v", n.ID, n.TopLeft, *n.TopLeft, originalPointers[i], originalPoints[i])
					}
				}
			} else if err != nil {
				t.Fatalf("unfixed transaction failed: %v", err)
			}
			t.Logf("transaction result fixed=%v rejected=%v receiver=%+v ahead=%+v branch=%+v", fixed, err != nil, *receiver.TopLeft, *ahead.TopLeft, *branch.TopLeft)

			// Exercise the operation that proposes exactly this translation,
			// independently of the direct Transaction.Commit check above.
			g, receiver, ahead, branch = newFixedOriginGapConnectedWitness(fixed)
			moved, score, err := reduceGapToNeighbors(ctx, receiver, nil, gapReductionOptions{
				axis:      horizontalAxis,
				direction: forwardDirection,
			})
			if err != nil {
				t.Fatal(err)
			}
			if moved == fixed {
				t.Fatalf("gap moved = %v with fixed=%v; want %v", moved, fixed, !fixed)
			}
			wantAhead, wantBranch := geo.Point{X: 250, Y: 0}, geo.Point{X: -350, Y: 200}
			if fixed {
				wantAhead, wantBranch = geo.Point{X: 600, Y: 0}, geo.Point{X: 0, Y: 200}
			}
			if *receiver.TopLeft != (geo.Point{X: 0, Y: 0}) || *ahead.TopLeft != wantAhead || *branch.TopLeft != wantBranch {
				t.Fatalf("gap result fixed=%v receiver=%+v ahead=%+v branch=%+v; want receiver=(0,0), ahead=%+v branch=%+v", fixed, *receiver.TopLeft, *ahead.TopLeft, *branch.TopLeft, wantAhead, wantBranch)
			}
			t.Logf("gap result fixed=%v moved=%v score=%v receiver=%+v ahead=%+v branch=%+v", fixed, moved, score, *receiver.TopLeft, *ahead.TopLeft, *branch.TopLeft)
		})
	}
}
