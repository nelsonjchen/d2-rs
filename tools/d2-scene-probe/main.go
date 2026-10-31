// SPDX-License-Identifier: MPL-2.0
//
// d2-scene-probe exposes the typed scene built by the pinned D2 source without
// going through its SVG serializer. It is a diagnostic oracle for comparing
// source scene commands and transforms with the Rust renderer's parsed path.
// Run from the pinned D2 checkout:
//
//	GOTOOLCHAIN=go1.27.0 go run ../d2-rs/tools/d2-scene-probe/main.go input.d2
package main

import (
	"context"
	"encoding/json"
	"fmt"
	"math"
	"os"
	"runtime"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts/d2dagrelayout"
	"github.com/d2lang/d2/d2lib"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/lib/log"
	"github.com/d2lang/d2/lib/textmeasure"
)

type point struct {
	X    float64 `json:"x"`
	XHex string  `json:"x_bits"`
	Y    float64 `json:"y"`
	YHex string  `json:"y_bits"`
}

type command struct {
	Kind    d2scene.PathCommandKind `json:"kind"`
	P1      point                   `json:"p1"`
	P2      point                   `json:"p2"`
	P3      point                   `json:"p3"`
	RadiusX float64                 `json:"radius_x,omitempty"`
	RadiusY float64                 `json:"radius_y,omitempty"`
}

type pathSnapshot struct {
	NodeID    string          `json:"node_id"`
	Parents   []string        `json:"parents"`
	MaskedBy  []string        `json:"masked_by"`
	Transform d2scene.Matrix  `json:"local_transform"`
	Opacity   float64         `json:"opacity"`
	HasMask   bool            `json:"has_mask"`
	Commands  []command       `json:"commands"`
	Stroke    *d2scene.Stroke `json:"stroke,omitempty"`
	Fill      d2scene.Paint   `json:"fill,omitempty"`
}

type report struct {
	D2Commit  string         `json:"d2_commit"`
	GoVersion string         `json:"go_version"`
	ViewBox   d2scene.Box    `json:"view_box"`
	Width     float64        `json:"logical_width"`
	Height    float64        `json:"logical_height"`
	Paths     []pathSnapshot `json:"paths"`
}

func main() {
	if len(os.Args) != 2 {
		fmt.Fprintln(os.Stderr, "usage: d2-scene-probe input.d2")
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
	ctx := log.WithDefault(context.Background())
	diagram, _, err := d2lib.Compile(ctx, string(input), &d2lib.CompileOptions{
		Layout:         &layout,
		InputPath:      os.Args[1],
		Ruler:          ruler,
		LayoutResolver: layoutResolver,
	}, nil)
	if err != nil {
		fatal(err)
	}
	document, err := d2scenebuild.Build(ctx, diagram, d2scenebuild.Options{})
	if err != nil {
		fatal(err)
	}
	result := report{
		D2Commit:  "d5a51743b6d5c00f1ec6f8340003f5d5a6ba4eda",
		GoVersion: runtime.Version(),
		ViewBox:   document.ViewBox,
		Width:     document.LogicalWidth,
		Height:    document.LogicalHeight,
	}
	collect(document.Root, nil, nil, &result.Paths)
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(result); err != nil {
		fatal(err)
	}
}

func collect(node *d2scene.Node, parents, maskedBy []string, paths *[]pathSnapshot) {
	if node == nil {
		return
	}
	if path, ok := node.Primitive.(d2scene.Path); ok {
		commands := make([]command, len(path.Commands))
		for index, item := range path.Commands {
			commands[index] = command{
				Kind:    item.Kind,
				P1:      makePoint(item.P1),
				P2:      makePoint(item.P2),
				P3:      makePoint(item.P3),
				RadiusX: item.RadiusX,
				RadiusY: item.RadiusY,
			}
		}
		*paths = append(*paths, pathSnapshot{
			NodeID: node.ID, Parents: append([]string(nil), parents...),
			MaskedBy:  append([]string(nil), maskedBy...),
			Transform: node.Transform, Opacity: node.Opacity, HasMask: node.Mask != nil,
			Commands: commands, Stroke: path.Stroke, Fill: path.Fill,
		})
	}
	parents = append(parents, node.ID)
	if node.Mask != nil {
		maskedBy = append(maskedBy, node.ID)
	}
	for _, child := range node.Children {
		collect(child, parents, maskedBy, paths)
	}
}

func makePoint(value d2scene.Point) point {
	return point{
		X: value.X, XHex: fmt.Sprintf("%016x", math.Float64bits(value.X)),
		Y: value.Y, YHex: fmt.Sprintf("%016x", math.Float64bits(value.Y)),
	}
}

func fatal(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}
