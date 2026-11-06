// SPDX-License-Identifier: MPL-2.0

// Recipe hook (tools/fixtures/langlibelk-elk-adapter.toml, elk-adapter-delete-bends),
// overlaid into d2layouts/d2elklayout. Inert unless D2RS_RECIPE_OUTPUT is set.
//
// For each synthetic graph it records the bend collision index's object/edge
// intersection counts for the listed query segments, then the routes left by
// deleteBends.
package d2elklayout

import (
	"encoding/json"
	"os"
	"testing"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/geo"
)

type d2rsBendNode struct {
	X, Y, W, H       float64
	Shape            string
	ThreeD, Multiple bool
	Columns          *int
}
type d2rsBendEdge struct {
	Source, Target             int
	Route                      []*geo.Point
	SourceColumn, TargetColumn *int
}
type d2rsBendQuery struct {
	Edge    int
	Segment [2]*geo.Point
}
type d2rsBendCase struct {
	Name     string
	Nodes    []d2rsBendNode
	Edges    []d2rsBendEdge
	Queries  []d2rsBendQuery
	Expected [][]*geo.Point
	Counts   [][5]int
}

func TestD2RSDeleteBends(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	raw, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var cases []d2rsBendCase
	if err = json.Unmarshal(raw, &cases); err != nil {
		t.Fatal(err)
	}
	for ci := range cases {
		c := &cases[ci]
		g := &d2graph.Graph{}
		for _, n := range c.Nodes {
			o := &d2graph.Object{Box: geo.NewBox(geo.NewPoint(n.X, n.Y), n.W, n.H)}
			o.Shape.Value = n.Shape
			if n.ThreeD {
				o.Style.ThreeDee = &d2graph.Scalar{Value: "true"}
			}
			if n.Multiple {
				o.Style.Multiple = &d2graph.Scalar{Value: "true"}
			}
			if n.Columns != nil {
				o.SQLTable = &d2target.SQLTable{Columns: make([]d2target.SQLColumn, *n.Columns)}
			}
			g.Objects = append(g.Objects, o)
		}
		for _, e := range c.Edges {
			route := make([]*geo.Point, len(e.Route))
			for i, p := range e.Route {
				route[i] = p.Copy()
			}
			g.Edges = append(g.Edges, &d2graph.Edge{Src: g.Objects[e.Source], Dst: g.Objects[e.Target], Route: route,
				SrcTableColumnIndex: e.SourceColumn, DstTableColumnIndex: e.TargetColumn})
		}
		index := newBendCollisionIndex(g)
		for _, q := range c.Queries {
			e := g.Edges[q.Edge]
			s := *geo.NewSegment(q.Segment[0], q.Segment[1])
			a := index.countObjectIntersects(e.Src, e.Dst, s)
			b, d, f, h := index.countEdgeIntersects(e, s)
			c.Counts = append(c.Counts, [5]int{a, b, d, f, h})
		}
		deleteBends(g)
		for _, e := range g.Edges {
			c.Expected = append(c.Expected, e.Route)
		}
	}
	raw, err = json.MarshalIndent(cases, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(out, append(raw, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
