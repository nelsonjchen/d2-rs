// SPDX-License-Identifier: MPL-2.0

// Command tala-public-api-oracle replays serialized-graph requests through
// upstream D2's public TALA API for the d2-rs-lib oracle fixtures
// (tools/fixtures/langlibelk-lib.toml, scripts/check-lib-tala-public-api.py).
//
// stdin is a JSON array of requests
//
//	{name, graph | raw_graph | raw_bytes, mode, cancel, seeds, requested}
//
// Each request runs d2graph.DeserializeGraph and then, unless that failed,
// d2talalayout.Layout (mode "layout") or d2talalayout.RouteEdges on the
// requested edge indices (any other mode), with a pre-cancelled context when
// cancel is set. stdout is a JSON array of rows {input, error, graph?,
// effective_font_sizes?} or {input, panic}. -fonts records each object's
// effective label font size after deserialization; -raw echoes the
// raw_graph/raw_bytes request fields.
//
// With -key-values, stdin is a JSON array of object IDs and stdout their
// d2format key spellings, [{value, key}].
package main

import (
	"context"
	"encoding/json"
	"flag"
	"fmt"
	"os"

	"github.com/d2lang/d2/d2ast"
	"github.com/d2lang/d2/d2format"
	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts/d2talalayout"
	"github.com/d2lang/d2/d2target"
)

type request struct {
	Name      string          `json:"name"`
	Graph     json.RawMessage `json:"graph"`
	RawGraph  string          `json:"raw_graph"`
	RawBytes  []byte          `json:"raw_bytes,omitempty"`
	Mode      string          `json:"mode"`
	Cancel    bool            `json:"cancel"`
	Seeds     []int64         `json:"seeds"`
	Requested []int           `json:"requested"`
}

// graphRequest is the echo of a request that carries only a decoded graph.
type graphRequest struct {
	Name      string          `json:"name"`
	Graph     json.RawMessage `json:"graph"`
	Mode      string          `json:"mode"`
	Cancel    bool            `json:"cancel"`
	Seeds     []int64         `json:"seeds"`
	Requested []int           `json:"requested"`
}

func main() {
	fonts := flag.Bool("fonts", false, "record effective_font_sizes")
	raw := flag.Bool("raw", false, "echo raw_graph and raw_bytes")
	keyValues := flag.Bool("key-values", false, "format object IDs as d2 keys")
	flag.Parse()
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", "  ")
	if *keyValues {
		var values []string
		check(json.NewDecoder(os.Stdin).Decode(&values))
		out := []map[string]string{}
		for _, v := range values {
			out = append(out, map[string]string{"value": v, "key": d2format.KeyPath(d2ast.MakeKeyPath([]string{v}))[0]})
		}
		check(enc.Encode(out))
		return
	}
	var requests []request
	check(json.NewDecoder(os.Stdin).Decode(&requests))
	rows := []any{}
	for _, item := range requests {
		var echo any = item
		if !*raw {
			echo = graphRequest{item.Name, item.Graph, item.Mode, item.Cancel, item.Seeds, item.Requested}
		}
		rows = append(rows, replay(item, echo, *fonts))
	}
	check(enc.Encode(rows))
}

func replay(item request, echo any, fonts bool) (row map[string]any) {
	defer func() {
		if recovered := recover(); recovered != nil {
			row = map[string]any{"input": echo, "panic": fmt.Sprint(recovered)}
		}
	}()
	g := d2graph.NewGraph()
	input := []byte(item.Graph)
	if item.RawGraph != "" {
		input = []byte(item.RawGraph)
	}
	if item.RawBytes != nil {
		input = item.RawBytes
	}
	err := d2graph.DeserializeGraph(input, g)
	// Only with -fonts: AbsID walks Parent links, which never terminates on
	// the parent cycles some graph-identity requests build.
	sizes := map[string]int{}
	if err == nil && fonts {
		for _, obj := range g.Objects {
			size := obj.Text().FontSize
			if obj.Class != nil || obj.SQLTable != nil {
				size -= d2target.HeaderFontAdd
			}
			sizes[obj.AbsID()] = size
		}
	}
	ctx, cancel := context.WithCancel(context.Background())
	if item.Cancel {
		cancel()
	}
	if err != nil {
		// DeserializeGraph failures are public results, too.
	} else if item.Mode == "layout" {
		err = d2talalayout.Layout(ctx, g, &d2talalayout.Options{Seeds: item.Seeds, MaxConcurrency: 1})
	} else {
		selected := []*d2graph.Edge{}
		for _, index := range item.Requested {
			selected = append(selected, g.Edges[index])
		}
		err = d2talalayout.RouteEdges(ctx, g, selected)
	}
	cancel()
	message := ""
	if err != nil {
		message = err.Error()
	}
	row = map[string]any{"input": echo, "error": message}
	if fonts {
		row["effective_font_sizes"] = sizes
	}
	if err == nil {
		data, err := d2graph.SerializeGraph(g)
		check(err)
		row["graph"] = json.RawMessage(data)
	}
	return row
}

func check(err error) {
	if err != nil {
		panic(err)
	}
}
