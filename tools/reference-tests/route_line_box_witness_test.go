// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package layoutgraph

import (
	"github.com/d2lang/d2/lib/geo"
	"testing"
)

func TestCodexRouteLineBoxWitnesses(t *testing.T) {
	cases := []struct {
		name       string
		start, end geo.Point
		box        geo.Box
		want       bool
	}{
		{"thin-positive-interval", geo.Point{X: 0, Y: 0}, geo.Point{X: 100, Y: 100}, geo.Box{TopLeft: &geo.Point{X: 50, Y: 50}, Width: 0.1, Height: 0.1}, true},
		{"corner-only", geo.Point{X: 0, Y: 0}, geo.Point{X: 100, Y: 100}, geo.Box{TopLeft: &geo.Point{X: 50, Y: 49}, Width: 1, Height: 1}, false},
		{"short-bottom-left-crossing", geo.Point{X: 4299, Y: 1484}, geo.Point{X: 3170, Y: 827}, geo.Box{TopLeft: &geo.Point{X: 3453, Y: 916}, Width: 117, Height: 76}, true},
	}
	for _, c := range cases {
		t.Run(c.name, func(t *testing.T) {
			if got := segmentIntersectsBox(&c.start, &c.end, &c.box); got != c.want {
				t.Fatalf("got %v want %v", got, c.want)
			}
		})
	}
}
