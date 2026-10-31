// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// d2-target-probe prints the exact routed target geometry used by D2 v0.9.0
// before the scene builder or SVG renderer transforms it. Run it from the
// pinned D2 checkout so Go resolves D2's own module and dependencies:
//
//	GOTOOLCHAIN=go1.27.0 go run ../d2-rs/tools/d2-target-probe/main.go input.d2
package main

import (
	"context"
	"fmt"
	"math"
	"os"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts/d2dagrelayout"
	"github.com/d2lang/d2/d2lib"
	"github.com/d2lang/d2/lib/log"
	"github.com/d2lang/d2/lib/textmeasure"
)

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: d2-target-probe input.d2")
		os.Exit(2)
	}
	input, err := os.ReadFile(os.Args[1])
	if err != nil {
		fatal(err)
	}
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		fatal(err)
	}
	layoutResolver := func(string) (d2graph.LayoutGraph, error) {
		return d2dagrelayout.DefaultLayout, nil
	}
	layout := "dagre"
	diagram, _, err := d2lib.Compile(log.WithDefault(context.Background()), string(input), &d2lib.CompileOptions{
		Layout:         &layout,
		InputPath:      os.Args[1],
		Ruler:          ruler,
		LayoutResolver: layoutResolver,
	}, nil)
	if err != nil {
		fatal(err)
	}
	for _, edge := range diagram.Connections {
		fmt.Printf("%s width=%d\n", edge.ID, edge.StrokeWidth)
		for index, point := range edge.Route {
			fmt.Printf("  %d x=%.17g[%016x] y=%.17g[%016x]\n",
				index, point.X, math.Float64bits(point.X), point.Y, math.Float64bits(point.Y))
		}
	}
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}
