// SPDX-License-Identifier: MPL-2.0

// Command cli-layout-probes captures pinned upstream layout and sketch
// results for the d2-rs-cli, d2-rs-lib, d2-rs-dagre and d2-rs-sketch oracle
// fixtures (tools/fixtures/langlibelk-cli.toml). It writes one JSON (or TSV)
// document to stdout:
//
//	label-positions  TALA (seed 1) label positions of every object, for the
//	                 authored label.near grid over four shapes and three layouts
//	raw-labeled      TALA Layout errors for a serialized graph with opposing
//	                 edge labels that carry no measured label geometry
//	dagre-flat       Dagre boxes and routes for flat graphs with one long label
//	solid-paths      rough-go solid-filled rectangle/polygon fill and outline paths
package main

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"log/slog"
	"os"
	"strings"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts/d2dagrelayout"
	"github.com/d2lang/d2/d2layouts/d2talalayout"
	"github.com/d2lang/d2/d2lib"
	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/lib/log"
	"github.com/d2lang/d2/lib/textmeasure"
	rough "github.com/d2lang/rough-go"
)

// probeContext routes D2 logging to stderr; stdout carries only the document.
func probeContext() context.Context {
	return log.With(context.Background(), slog.New(slog.NewTextHandler(os.Stderr, nil)))
}

func main() {
	if len(os.Args) != 2 {
		fail(fmt.Errorf("usage: cli-layout-probes label-positions|raw-labeled|dagre-flat|solid-paths"))
	}
	switch os.Args[1] {
	case "label-positions":
		emit(labelPositions())
	case "raw-labeled":
		emit(rawLabeled())
	case "dagre-flat":
		emit(dagreFlat())
	case "solid-paths":
		os.Stdout.WriteString(solidPaths())
	default:
		fail(fmt.Errorf("unknown probe %q", os.Args[1]))
	}
}

func fail(err error) {
	fmt.Fprintln(os.Stderr, err)
	os.Exit(1)
}

func emit(value any) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", "  ")
	if err := enc.Encode(value); err != nil {
		fail(err)
	}
	os.Stdout.Write(buf.Bytes())
}

func compile(source string, layout d2graph.LayoutGraph, sketch bool) *d2graph.Graph {
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		fail(err)
	}
	opts := &d2lib.CompileOptions{
		Ruler: ruler,
		LayoutResolver: func(string) (d2graph.LayoutGraph, error) {
			return layout, nil
		},
	}
	_, g, err := d2lib.Compile(probeContext(), source, opts, &d2svg.RenderOpts{Sketch: &sketch})
	if err != nil {
		fail(fmt.Errorf("%q: %w", source, err))
	}
	return g
}

// labelPositions: the authored label.near grid (.reference/glob-dimensions-20261006).
func labelPositions() any {
	positions := []string{"top-left", "top-center", "top-right", "center-left", "center-center", "center-right",
		"bottom-left", "bottom-center", "bottom-right"}
	for _, side := range []string{"outside", "border"} {
		positions = append(positions, side+"-top-left", side+"-top-center", side+"-top-right",
			side+"-left-top", side+"-left-center", side+"-left-bottom",
			side+"-right-top", side+"-right-center", side+"-right-bottom",
			side+"-bottom-left", side+"-bottom-center", side+"-bottom-right")
	}
	positions = append(positions, "unset", "null")
	shapes := []struct{ name, attrs string }{
		{"circle", "shape: circle; width: 10; height: 10"},
		{"rectangle", "width: 10; height: 10"},
		{"person", "shape: person"},
		{"cylinder-container", "shape: cylinder; kid"},
	}
	type row struct {
		Name      string             `json:"name"`
		Positions map[string]*string `json:"positions"`
		Source    string             `json:"source"`
	}
	tala := func(ctx context.Context, g *d2graph.Graph) error {
		return d2talalayout.Layout(ctx, g, &d2talalayout.Options{Seeds: []int64{1}, MaxConcurrency: 1})
	}
	observe := func(name, source string) row {
		g := compile(source, tala, false)
		out := map[string]*string{}
		for _, obj := range g.Objects {
			out[obj.AbsID()] = obj.LabelPosition
		}
		return row{name, out, source}
	}
	rows := []row{}
	for _, shape := range shapes {
		for _, layout := range []string{"flat", "nested", "grid"} {
			for _, position := range positions {
				probe := shape.attrs
				if position != "unset" {
					probe += "; label.near: " + position
				}
				probe = "a: Probe { " + probe + " }"
				var source string
				switch layout {
				case "flat":
					source = probe + "; b: Other; a -> b"
				case "nested":
					source = "p: { " + probe + "; b: Other; a -> b }"
				default:
					source = "grid-rows: 1; " + probe + "; b: Other"
				}
				rows = append(rows, observe(shape.name+"/"+layout+"/"+position, source))
			}
		}
	}
	rows = append(rows, observe("class-centered-undersized",
		"classes.c: { shape: circle; width: 10; height: 10; label.near: center-center }\na: Probe { class: c }; b; a -> b"))
	return rows
}

// rawLabeled: crates/d2-rs-lib/tests/fixtures/simple.json with opposing
// labeled edges (and, for raw-labeled-explicit.json, an explicit 20x20 b),
// as assert_raw_labeled_opposing_source_failure builds it.
func rawLabeled() any {
	simple, err := os.ReadFile(os.Getenv("D2RS_SIMPLE_GRAPH"))
	if err != nil {
		fail(err)
	}
	type row struct {
		Concurrency int     `json:"concurrency"`
		Error       string  `json:"error"`
		Input       string  `json:"input"`
		Seeds       []int64 `json:"seeds"`
	}
	rows := []row{}
	for _, input := range []string{"raw-labeled.json", "raw-labeled-explicit.json"} {
		var document map[string]any
		if err := json.Unmarshal(simple, &document); err != nil {
			fail(err)
		}
		edges := document["edges"].([]any)
		edges[0].(map[string]any)["attributes"] = map[string]any{"label": map[string]any{"value": "forward"}}
		document["edges"] = append(edges, map[string]any{"Src": "b", "Dst": "a", "index": 0, "isCurve": false,
			"attributes": map[string]any{"label": map[string]any{"value": "reverse"}}})
		if input == "raw-labeled-explicit.json" {
			b := document["objects"].([]any)[1].(map[string]any)
			attributes, _ := b["attributes"].(map[string]any)
			if attributes == nil {
				attributes = map[string]any{}
				b["attributes"] = attributes
			}
			attributes["width"] = map[string]any{"value": "20"}
			attributes["height"] = map[string]any{"value": "20"}
			box := b["box"].(map[string]any)
			box["Width"] = 20
			box["Height"] = 20
		}
		data, err := json.Marshal(document)
		if err != nil {
			fail(err)
		}
		for _, seeds := range [][]int64{{1}, {1, 2, 3}} {
			for concurrency := 0; concurrency <= 2; concurrency++ {
				g := d2graph.NewGraph()
				if err := d2graph.DeserializeGraph(data, g); err != nil {
					fail(err)
				}
				err := d2talalayout.Layout(probeContext(), g, &d2talalayout.Options{Seeds: seeds, MaxConcurrency: concurrency})
				if err == nil {
					fail(fmt.Errorf("%s seeds %v: layout unexpectedly succeeded", input, seeds))
				}
				rows = append(rows, row{concurrency, err.Error(), input, seeds})
			}
		}
	}
	return rows
}

type point struct {
	X float64 `json:"x"`
	Y float64 `json:"y"`
}

type size struct {
	Height float64 `json:"height"`
	Width  float64 `json:"width"`
}

// dagreFlat: .reference/dagre-curved-20261018/capture-controls.py inputs,
// observed right after the Dagre layout returns.
func dagreFlat() any {
	type node struct {
		ID        string `json:"id"`
		Size      size   `json:"size"`
		LabelSize size   `json:"label_size"`
	}
	type edge struct {
		Source    string `json:"source"`
		Target    string `json:"target"`
		Label     string `json:"label"`
		LabelSize size   `json:"label_size"`
	}
	type box struct {
		ID     string `json:"id"`
		Origin point  `json:"origin"`
		Size   size   `json:"size"`
	}
	type row struct {
		Name           string    `json:"name"`
		Direction      string    `json:"direction"`
		Nodes          []node    `json:"nodes"`
		Edges          []edge    `json:"edges"`
		ExpectedBoxes  []box     `json:"expected_boxes"`
		ExpectedRoutes [][]point `json:"expected_routes"`
	}
	rows := []row{}
	for _, direction := range []string{"down", "up", "right", "left"} {
		for _, count := range []int{3, 4} {
			names := strings.Split("abcd"[:count], "")
			source := "direction: " + direction + "\n" + strings.Join(names, " -> ") + "\n" +
				names[0] + " -> " + names[count-1] + ": route {style.border-radius: 20}\n"
			for _, sketch := range []bool{false, true} {
				name := fmt.Sprintf("%s-%d-ordinary", direction, count)
				if sketch {
					name = fmt.Sprintf("%s-%d-sketch", direction, count)
				}
				var r row
				layout := func(ctx context.Context, g *d2graph.Graph) error {
					if err := d2dagrelayout.DefaultLayout(ctx, g); err != nil {
						return err
					}
					r = row{Name: name, Direction: direction}
					for _, obj := range g.Objects {
						r.Nodes = append(r.Nodes, node{obj.ID, size{obj.Height, obj.Width},
							size{float64(obj.LabelDimensions.Height), float64(obj.LabelDimensions.Width)}})
						r.ExpectedBoxes = append(r.ExpectedBoxes, box{obj.ID, point{obj.TopLeft.X, obj.TopLeft.Y},
							size{obj.Height, obj.Width}})
					}
					for _, e := range g.Edges {
						r.Edges = append(r.Edges, edge{e.Src.ID, e.Dst.ID, e.Label.Value,
							size{float64(e.LabelDimensions.Height), float64(e.LabelDimensions.Width)}})
						route := []point{}
						for _, p := range e.Route {
							route = append(route, point{p.X, p.Y})
						}
						r.ExpectedRoutes = append(r.ExpectedRoutes, route)
					}
					return nil
				}
				compile(source, layout, sketch)
				rows = append(rows, r)
			}
		}
	}
	return rows
}

// solidPaths: rough-go solid fills for the d2-rs-sketch SketchOptions grid.
func solidPaths() string {
	var out strings.Builder
	shapes := []string{"rect", "rect-long", "triangle", "line", "point", "empty"}
	for _, seed := range []int64{1, 42, 2147483647} {
		for _, mode := range []string{"default", "preserve", "single", "zero"} {
			for _, shape := range shapes {
				roughness := 1.0
				if mode == "zero" {
					roughness = 0
				}
				opts := &rough.Options{
					Fill:               rough.String("#000"),
					FillStyle:          rough.String("solid"),
					Seed:               rough.Float64(float64(seed)),
					Bowing:             rough.Float64(2),
					Roughness:          rough.Float64(roughness),
					PreserveVertices:   rough.Bool(mode == "preserve"),
					DisableMultiStroke: rough.Bool(mode == "single"),
				}
				g := rough.NewGenerator(nil)
				var d rough.Drawable
				switch shape {
				case "rect":
					d = g.Rectangle(2.25, -3.5, 140, 80, opts)
				case "rect-long":
					d = g.Rectangle(2.25, -3.5, 350, 700, opts)
				default:
					points := map[string][]rough.Point{
						"triangle": {{0, 0}, {100.5, 10.25}, {33, 77}},
						"line":     {{0, 0}, {100.5, 10.25}},
						"point":    {{3, 5}},
						"empty":    {},
					}[shape]
					d = g.Polygon(points, opts)
				}
				fill, outline := "", ""
				for _, p := range g.ToPaths(d) {
					if p.Fill != "none" && p.Fill != "" {
						fill = p.D
					} else {
						outline = p.D
					}
				}
				fmt.Fprintf(&out, "%d\t%s\t%s\t%s\t%s\n", seed, mode, shape, fill, outline)
			}
		}
	}
	return out.String()
}
