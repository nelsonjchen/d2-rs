// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Development reference only; never shipped as part of d2-rs.
//
// Whole-D2 layout oracle for the layout-oracle recipes
// (tools/fixtures/layout-oracle.toml, tools/fixtures/tala-oracle.toml).
// Reads a JSON list of {name, source, layout, seeds} on stdin, compiles
// each with d2lib.Compile and the named in-process engine (dagre, elk or
// tala with the given seeds), and writes per case:
//
//   - shapes, connections: the d2target diagram;
//   - objects: every graph object's box (decimal and float64 bits) and
//     label dimensions;
//   - reference_edges: for each non-lifeline connection, its graph edge's
//     reference scopes (ScopeObj AbsIDs, "" for the root);
//   - lines: the earliest non-glob source line of every object and of each
//     non-lifeline connection, as d2sequence orders actors and messages
//     (math.MaxInt32 when none);
//   - error: the compile/layout error text;
//   - positions: when layout fails d2layouts' object-position validation,
//     every positioned object of the failing (possibly nested) graph, as
//     formatted with %f by the error message.
//
// The binary is built with an -overlay (scripts/layout_oracle_fixtures.py)
// that renames d2layouts.validateObjectPositions and adds d2rs_positions.go,
// an observer around it plus an exported entry point for direct validation.
// A request with `validate` instead of `source` runs that validation on
// one 1x1 object per entry (coordinates as Go float strings).
package main

import (
	"context"
	"encoding/json"
	"math"
	"os"
	"fmt"
	"strconv"
	"strings"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts"
	"github.com/d2lang/d2/d2layouts/d2dagrelayout"
	"github.com/d2lang/d2/d2layouts/d2elklayout"
	"github.com/d2lang/d2/d2layouts/d2talalayout"
	"github.com/d2lang/d2/d2lib"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/geo"
	"github.com/d2lang/d2/lib/log"
	"github.com/d2lang/d2/lib/textmeasure"
)

type request struct {
	Name     string     `json:"name"`
	Source   string     `json:"source"`
	Layout   string     `json:"layout"`
	Seeds    []int64    `json:"seeds,omitempty"`
	Validate []position `json:"validate,omitempty"`
}

type position struct {
	ID     string `json:"id"`
	X      string `json:"x"`
	Y      string `json:"y"`
	Width  string `json:"width,omitempty"`
	Height string `json:"height,omitempty"`
}

type object struct {
	ID          string    `json:"id"`
	X           float64   `json:"x"`
	Y           float64   `json:"y"`
	Width       float64   `json:"width"`
	Height      float64   `json:"height"`
	Bits        [4]uint64 `json:"bits"`
	LabelWidth  int       `json:"label_width"`
	LabelHeight int       `json:"label_height"`
}

type referenceEdge struct {
	Scopes []string `json:"scopes"`
	Source string   `json:"source"`
	Target string   `json:"target"`
}

type lineEdge struct {
	Line   int    `json:"line"`
	Source string `json:"source"`
	Target string `json:"target"`
}

type lines struct {
	Objects map[string]int `json:"objects"`
	Edges   []lineEdge     `json:"edges"`
}

type result struct {
	Name           string                `json:"name"`
	Error          string                `json:"error"`
	Shapes         []d2target.Shape      `json:"shapes"`
	Connections    []d2target.Connection `json:"connections"`
	Objects        []object              `json:"objects"`
	ReferenceEdges []referenceEdge       `json:"reference_edges"`
	Lines          *lines                `json:"lines"`
	Positions      []position            `json:"positions"`
}

// Same rules as d2sequence getObjEarliestLineNum / getEdgeEarliestLineNum.
func objectLine(o *d2graph.Object) int {
	line := int(math.MaxInt32)
	for _, ref := range o.References {
		if ref.MapKey == nil || ref.Key.HasGlob() {
			continue
		}
		line = min(line, ref.MapKey.Range.Start.Line)
	}
	return line
}

func edgeLine(e *d2graph.Edge) int {
	line := int(math.MaxInt32)
	for _, ref := range e.References {
		if ref.MapKey == nil || ref.Edge.Src.HasGlob() || ref.Edge.Dst.HasGlob() {
			continue
		}
		line = min(line, ref.MapKey.Range.Start.Line)
	}
	return line
}

func format(v float64) string { return fmt.Sprintf("%f", v) }

func number(s string) float64 {
	v, err := strconv.ParseFloat(s, 64)
	if err != nil {
		panic(err)
	}
	return v
}

func validate(item request) result {
	g := d2graph.NewGraph()
	var objects []*d2graph.Object
	for _, p := range item.Validate {
		obj := &d2graph.Object{Graph: g, Parent: g.Root, ID: p.ID, IDVal: p.ID,
			Box: geo.NewBox(geo.NewPoint(number(p.X), number(p.Y)), 1, 1), Children: map[string]*d2graph.Object{}}
		objects = append(objects, obj)
	}
	out := result{Name: item.Name}
	if err := d2layouts.D2RSValidateObjectPositions(objects); err != nil {
		out.Error = err.Error()
	}
	return out
}

func compile(ruler *textmeasure.Ruler, item request) result {
	out := result{Name: item.Name}
	d2layouts.D2RSPositionObserver = func(objects []*d2graph.Object, err error) {
		if err == nil || out.Positions != nil {
			return
		}
		out.Positions = []position{}
		for _, obj := range objects {
			if obj.TopLeft == nil {
				continue
			}
			out.Positions = append(out.Positions, position{obj.AbsID(), format(obj.TopLeft.X), format(obj.TopLeft.Y),
				format(obj.Width), format(obj.Height)})
		}
	}
	defer func() { d2layouts.D2RSPositionObserver = nil }()
	resolver := func(engine string) (d2graph.LayoutGraph, error) {
		switch item.Layout {
		case "elk":
			return d2elklayout.DefaultLayout, nil
		case "tala":
			return func(ctx context.Context, g *d2graph.Graph) error {
				return d2talalayout.Layout(ctx, g, &d2talalayout.Options{Seeds: item.Seeds, MaxConcurrency: 1})
			}, nil
		default:
			return d2dagrelayout.DefaultLayout, nil
		}
	}
	layout := item.Layout
	diagram, g, err := d2lib.Compile(log.WithDefault(context.Background()), item.Source, &d2lib.CompileOptions{
		Layout: &layout, Ruler: ruler, LayoutResolver: resolver,
	}, nil)
	if err != nil {
		out.Error = err.Error()
	}
	if diagram != nil {
		out.Shapes, out.Connections = diagram.Shapes, diagram.Connections
	}
	if g == nil {
		return out
	}
	out.Lines = &lines{Objects: map[string]int{}, Edges: []lineEdge{}}
	out.ReferenceEdges = []referenceEdge{}
	for _, obj := range g.Objects {
		row := object{ID: obj.AbsID(), Width: obj.Width, Height: obj.Height,
			LabelWidth: obj.LabelDimensions.Width, LabelHeight: obj.LabelDimensions.Height}
		if obj.TopLeft != nil {
			row.X, row.Y = obj.TopLeft.X, obj.TopLeft.Y
		}
		row.Bits = [4]uint64{math.Float64bits(row.X), math.Float64bits(row.Y),
			math.Float64bits(row.Width), math.Float64bits(row.Height)}
		// A failed layout may leave non-finite boxes, which JSON cannot carry;
		// its positions are reported as text instead.
		if err == nil {
			out.Objects = append(out.Objects, row)
		}
		out.Lines.Objects[obj.AbsID()] = objectLine(obj)
	}
	edges := map[string]*d2graph.Edge{}
	for _, e := range g.Edges {
		edges[e.AbsID()] = e
	}
	for _, connection := range out.Connections {
		if strings.Contains(connection.Dst, "-lifeline-end-") {
			continue
		}
		e := edges[connection.ID]
		if e == nil {
			continue
		}
		scopes := []string{}
		for _, ref := range e.References {
			scope := ""
			if ref.ScopeObj != nil {
				scope = ref.ScopeObj.AbsID()
			}
			scopes = append(scopes, scope)
		}
		out.ReferenceEdges = append(out.ReferenceEdges, referenceEdge{scopes, e.Src.AbsID(), e.Dst.AbsID()})
		out.Lines.Edges = append(out.Lines.Edges, lineEdge{edgeLine(e), e.Src.AbsID(), e.Dst.AbsID()})
	}
	return out
}

func main() {
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		panic(err)
	}
	results := make([]result, 0, len(requests))
	for _, item := range requests {
		if item.Validate != nil {
			results = append(results, validate(item))
		} else {
			results = append(results, compile(ruler, item))
		}
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", " ")
	if err := enc.Encode(results); err != nil {
		panic(err)
	}
}
