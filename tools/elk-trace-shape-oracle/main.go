// SPDX-License-Identifier: MPL-2.0

// Development-only oracle for crates/d2-rs-elk/src/shape_trace.rs (recipe
// elk-adapter-trace-to-shape): exported d2graph.Edge.TraceToShape over a JSON
// array of two-object requests on stdin; the same array with Expected and
// Indices filled on stdout.
package main

import (
	"encoding/json"
	"net/url"
	"os"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/lib/geo"
)

type node struct {
	X, Y, W, H                         float64
	Shape, Label, Position, IconPosition string
	LabelW, LabelH                     int
	Icon                               bool
}

type request struct {
	Name     string
	Nodes    [2]node
	Route    []*geo.Point
	Expected []*geo.Point
	Indices  [2]int
}

func main() {
	var rows []request
	if err := json.NewDecoder(os.Stdin).Decode(&rows); err != nil {
		panic(err)
	}
	for i := range rows {
		r := &rows[i]
		objects := [2]*d2graph.Object{}
		for j, n := range r.Nodes {
			o := &d2graph.Object{Box: geo.NewBox(geo.NewPoint(n.X, n.Y), n.W, n.H)}
			o.Shape.Value = n.Shape
			o.Label.Value = n.Label
			o.LabelDimensions.Width = n.LabelW
			o.LabelDimensions.Height = n.LabelH
			p, ip := n.Position, n.IconPosition
			o.LabelPosition = &p
			o.IconPosition = &ip
			if n.Icon {
				o.Icon = &url.URL{}
			}
			objects[j] = o
		}
		points := make([]*geo.Point, len(r.Route))
		for j, p := range r.Route {
			points[j] = p.Copy()
		}
		e := &d2graph.Edge{Src: objects[0], Dst: objects[1]}
		a, b := e.TraceToShape(points, 0, len(points)-1)
		r.Expected = points[a : b+1]
		r.Indices = [2]int{a, b}
	}
	if err := json.NewEncoder(os.Stdout).Encode(rows); err != nil {
		panic(err)
	}
}
