// SPDX-License-Identifier: MPL-2.0
// Public d2compiler oracle for the d2-rs-language fixture recipes
// (tools/fixtures/langlibelk-language-public.toml). Development oracle only.
//
// stdin: [{hex, path, utf16, files_hex, fs}]; stdout: one row per request.
// fs "memfs" serves files_hex through lib/memfs (the d2js/WASM file system)
// instead of testing/fstest.MapFS.
//
//	compile  d2compiler.Compile; {error, panic} or {config, error: null, graph, panic: false}
//	         with graph = d2graph.SerializeGraph.
//	inspect  as compile, plus "inspect": the legend's object/edge counts, each
//	         object's parent, tooltip and earliest declaration line, each edge's
//	         AbsID, reference scopes and earliest declaration line (the
//	         d2sequence getObjEarliestLineNum/getEdgeEarliestLineNum rules), and
//	         the d2exporter shapes' tooltips after giving every object a
//	         160x160 box.
//	measure  as compile, then SetDimensions(nil, textmeasure.NewRuler(), nil, nil); the
//	         graph before and after, the raw and measured label bytes and the
//	         measurement error/panic.
package main

import (
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io/fs"
	"math"
	"os"
	"strings"
	"testing/fstest"

	"github.com/d2lang/d2/d2compiler"
	"github.com/d2lang/d2/d2exporter"
	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/lib/geo"
	"github.com/d2lang/d2/lib/memfs"
	"github.com/d2lang/d2/lib/textmeasure"
)

type request struct {
	Hex      string            `json:"hex"`
	Path     string            `json:"path"`
	UTF16    bool              `json:"utf16"`
	FilesHex map[string]string `json:"files_hex"`
	FS       string            `json:"fs"`
}

func compile(r request) (*d2graph.Graph, any, error) {
	input, err := hex.DecodeString(r.Hex)
	if err != nil {
		panic(err)
	}
	files := fstest.MapFS{}
	contents := map[string]string{}
	for p, s := range r.FilesHex {
		b, e := hex.DecodeString(s)
		if e != nil {
			panic(e)
		}
		files[p] = &fstest.MapFile{Data: b}
		contents[p] = string(b)
	}
	var fileSystem fs.FS = files
	if r.FS == "memfs" {
		m, e := memfs.New(contents)
		if e != nil {
			panic(e)
		}
		fileSystem = m
	} else if r.FS != "" {
		panic("unknown fs " + r.FS)
	}
	g, config, err := d2compiler.Compile(r.Path, strings.NewReader(string(input)), &d2compiler.CompileOptions{UTF16Pos: r.UTF16, FS: fileSystem})
	return g, config, err
}

func serialize(g *d2graph.Graph) json.RawMessage {
	b, err := d2graph.SerializeGraph(g)
	if err != nil {
		panic(err)
	}
	return json.RawMessage(b)
}

func labels(g *d2graph.Graph) any {
	objects := []any{}
	edges := []any{}
	for _, o := range g.Objects {
		objects = append(objects, map[string]any{"id": o.AbsID(), "id_hex": hex.EncodeToString([]byte(o.AbsID())), "label_hex": hex.EncodeToString([]byte(o.Label.Value))})
	}
	for _, e := range g.Edges {
		edges = append(edges, map[string]any{"label_hex": hex.EncodeToString([]byte(e.Label.Value))})
	}
	return map[string]any{"objects": objects, "edges": edges}
}

func measure(g *d2graph.Graph, ruler *textmeasure.Ruler) (out map[string]any) {
	out = map[string]any{"error": nil, "panic": false}
	defer func() {
		if v := recover(); v != nil {
			out["panic"] = true
			out["panic_value"] = fmt.Sprint(v)
		}
	}()
	if e := g.SetDimensions(nil, ruler, nil, nil); e != nil {
		out["error"] = e.Error()
	}
	return
}

func earliestObjectLine(o *d2graph.Object) int {
	min := int(math.MaxInt32)
	for _, ref := range o.References {
		if ref.MapKey == nil || ref.Key.HasGlob() {
			continue
		}
		if ref.MapKey.Range.Start.Line < min {
			min = ref.MapKey.Range.Start.Line
		}
	}
	return min
}

func earliestEdgeLine(e *d2graph.Edge) int {
	min := int(math.MaxInt32)
	for _, ref := range e.References {
		if ref.MapKey == nil || ref.Edge.Src.HasGlob() || ref.Edge.Dst.HasGlob() {
			continue
		}
		if ref.MapKey.Range.Start.Line < min {
			min = ref.MapKey.Range.Start.Line
		}
	}
	return min
}

func inspect(g *d2graph.Graph) map[string]any {
	var legend any
	if g.Legend != nil {
		legend = map[string]any{"objects": len(g.Legend.Objects), "edges": len(g.Legend.Edges)}
	}
	objects := []any{}
	for _, o := range g.Objects {
		var tooltip, position any
		if o.Tooltip != nil {
			tooltip = o.Tooltip.Value
		}
		if o.TooltipPosition != nil {
			position = o.TooltipPosition.Value
		}
		parent := ""
		if o.Parent != nil {
			parent = o.Parent.AbsID()
		}
		objects = append(objects, map[string]any{"id": o.AbsID(), "parent": parent, "line": earliestObjectLine(o),
			"tooltip": tooltip, "tooltip_position": position})
	}
	edges := []any{}
	for _, e := range g.Edges {
		scopes := []string{}
		for _, ref := range e.References {
			scope := ""
			if ref.ScopeObj != nil {
				scope = ref.ScopeObj.AbsID()
			}
			scopes = append(scopes, scope)
		}
		edges = append(edges, map[string]any{"abs_id": e.AbsID(), "line": earliestEdgeLine(e), "scopes": scopes})
	}
	out := map[string]any{"legend": legend, "objects": objects, "edges": edges}
	for _, o := range g.Objects {
		o.Box = geo.NewBox(geo.NewPoint(0, 0), 160, 160)
	}
	diagram, err := d2exporter.Export(context.Background(), g, nil, nil)
	if err != nil {
		out["export_error"] = err.Error()
		return out
	}
	targets := []any{}
	for _, s := range diagram.Shapes {
		targets = append(targets, map[string]any{"id": s.ID, "tooltip": s.Tooltip, "position": s.TooltipPosition})
	}
	out["targets"] = targets
	return out
}

func evaluate(mode string, r request) (out map[string]any) {
	defer func() {
		if v := recover(); v != nil {
			out = map[string]any{"panic": true, "error": nil, "panic_value": fmt.Sprint(v)}
		}
	}()
	g, config, err := compile(r)
	if err != nil {
		return map[string]any{"error": err.Error(), "panic": false}
	}
	if mode == "compile" {
		return map[string]any{"config": config, "error": nil, "graph": serialize(g), "panic": false}
	}
	if mode == "inspect" {
		graph := serialize(g)
		return map[string]any{"config": config, "error": nil, "graph": graph, "panic": false, "inspect": inspect(g)}
	}
	before := serialize(g)
	raw := labels(g)
	ruler, e := textmeasure.NewRuler()
	if e != nil {
		panic(e)
	}
	measured := measure(g, ruler)
	return map[string]any{"error": nil, "panic": false, "config": config, "before": before, "after": serialize(g),
		"raw_labels": raw, "measured_labels": labels(g), "measurement": measured}
}

func main() {
	if len(os.Args) != 2 || (os.Args[1] != "compile" && os.Args[1] != "inspect" && os.Args[1] != "measure") {
		fmt.Fprintln(os.Stderr, "usage: public-compiler-oracle compile|inspect|measure < requests.json")
		os.Exit(2)
	}
	var requests []request
	if e := json.NewDecoder(os.Stdin).Decode(&requests); e != nil {
		panic(e)
	}
	rows := []any{}
	for _, r := range requests {
		rows = append(rows, evaluate(os.Args[1], r))
	}
	if e := json.NewEncoder(os.Stdout).Encode(rows); e != nil {
		panic(e)
	}
}
