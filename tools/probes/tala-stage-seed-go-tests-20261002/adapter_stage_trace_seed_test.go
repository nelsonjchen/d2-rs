// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
package d2talalayout

import (
	"context"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
)

func TestDiagnosticStageSeedWinnerMetadata(t *testing.T) {
	seeds := []int64{-17, 0, 42}
	for _, test := range []struct {
		name   string
		scores []float64
		index  int
	}{
		{"negative-seed-wins", []float64{1, 2, 3}, 0},
		{"zero-seed-wins", []float64{2, 1, 3}, 1},
		{"equal-score-last-index-wins", []float64{1, 1, 1}, 2},
	} {
		for _, concurrency := range []int{1, 3} {
			t.Run(test.name, func(t *testing.T) {
				graphs := []*layoutgraph.Graph{layoutgraph.NewGraph(), layoutgraph.NewGraph(), layoutgraph.NewGraph()}
				selected, err := coordinateLocalSeeds(context.Background(), seeds, concurrency, func(index int, seed int64) localSeedAttempt {
					return localSeedAttempt{index: index, seed: seed, result: seedResult{graph: graphs[index], score: layoutScore{penalty: test.scores[index]}}}
				})
				if err != nil {
					t.Fatal(err)
				}
				if selected.graph != graphs[test.index] || selected.traceSeed == nil || *selected.traceSeed != seeds[test.index] {
					t.Fatalf("concurrency=%d selected graph=%p seed=%v expected graph=%p seed=%d", concurrency, selected.graph, selected.traceSeed, graphs[test.index], seeds[test.index])
				}
			})
		}
	}
}
