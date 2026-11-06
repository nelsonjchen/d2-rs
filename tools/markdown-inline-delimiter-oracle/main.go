// SPDX-License-Identifier: MPL-2.0
package main

import (
	"encoding/hex"
	"encoding/json"
	"github.com/yuin/goldmark/ast"
	"github.com/yuin/goldmark/extension"
	"github.com/yuin/goldmark/parser"
	"github.com/yuin/goldmark/text"
	"os"
	"reflect"
)

type call struct {
	Kind    string
	Advance int
}
type request struct {
	Source   string
	Segments []text.Segment
	Calls    []call
}

func evaluate(q request) any {
	source, err := hex.DecodeString(q.Source)
	if err != nil {
		panic(err)
	}
	segments := text.NewSegments()
	segments.AppendAll(q.Segments)
	reader := text.NewBlockReader(source, segments)
	pc := parser.NewContext()
	root := ast.NewParagraph()
	nodes := []ast.Node{root}
	ids := map[ast.Node]int{root: 0}
	id := func(n ast.Node) any {
		if n == nil || reflect.ValueOf(n).IsNil() {
			return nil
		}
		v, ok := ids[n]
		if !ok {
			panic("unregistered node")
		}
		return v
	}
	snapshots := make([]any, 0)
	snapshot := func(returned ast.Node) {
		lineBytes, _ := reader.PeekLine()
		var lineValue any
		if lineBytes != nil {
			lineValue = hex.EncodeToString(lineBytes)
		}
		line, pos := reader.Position()
		rows := make([]any, 0, len(nodes))
		for _, n := range nodes {
			var data any = map[string]any{"kind": "Container"}
			if d, ok := n.(*parser.Delimiter); ok {
				data = map[string]any{"kind": "Delimiter", "segment": d.Segment, "can_open": d.CanOpen, "can_close": d.CanClose, "length": d.Length, "original_length": d.OriginalLength, "char": d.Char, "previous_delimiter": id(d.PreviousDelimiter), "next_delimiter": id(d.NextDelimiter)}
			}
			rows = append(rows, map[string]any{"data": data, "parent": id(n.Parent()), "previous": id(n.PreviousSibling()), "next": id(n.NextSibling()), "first": id(n.FirstChild()), "last": id(n.LastChild()), "child_count": n.ChildCount()})
		}
		snapshots = append(snapshots, map[string]any{"nodes": rows, "first_delimiter": id(pc.FirstDelimiter()), "last_delimiter": id(pc.LastDelimiter()), "returned": id(returned), "line": line, "position": pos, "text": lineValue, "source": hex.EncodeToString(source)})
	}
	snapshot(nil)
	for _, c := range q.Calls {
		reader.Advance(c.Advance)
		var p parser.InlineParser = parser.NewEmphasisParser()
		if c.Kind == "strike" {
			p = extension.NewStrikethroughParser()
		}
		n := p.Parse(root, reader, pc)
		if n != nil {
			ids[n] = len(nodes)
			nodes = append(nodes, n)
			root.AppendChild(root, n)
		}
		snapshot(n)
	}
	return snapshots
}
func main() {
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	results := make([]any, 0, len(requests))
	for _, q := range requests {
		results = append(results, evaluate(q))
	}
	if err := json.NewEncoder(os.Stdout).Encode(results); err != nil {
		panic(err)
	}
}
