// SPDX-License-Identifier: MPL-2.0
package main

import (
	"encoding/json"
	"github.com/yuin/goldmark/ast"
	east "github.com/yuin/goldmark/extension/ast"
	"github.com/yuin/goldmark/parser"
	"github.com/yuin/goldmark/text"
	"os"
	"reflect"
)

type builtinProcessor struct {
	char     byte
	register func(ast.Node)
}

func (p builtinProcessor) IsDelimiter(c byte) bool {
	if p.char == '~' {
		return c == '~'
	}
	return c == '*' || c == '_'
}
func (p builtinProcessor) CanOpenCloser(a, b *parser.Delimiter) bool { return a.Char == b.Char }
func (p builtinProcessor) OnMatch(n int) ast.Node {
	var node ast.Node
	if p.char == '~' {
		node = east.NewStrikethrough()
	} else {
		node = ast.NewEmphasis(n)
	}
	p.register(node)
	return node
}

type nodeSpec struct {
	Kind           string       `json:"kind"`
	Segment        text.Segment `json:"segment"`
	Raw            bool         `json:"raw"`
	Soft           bool         `json:"soft_line_break"`
	Hard           bool         `json:"hard_line_break"`
	CanOpen        bool         `json:"can_open"`
	CanClose       bool         `json:"can_close"`
	Length         int          `json:"length"`
	OriginalLength int          `json:"original_length"`
	Char           byte         `json:"char"`
}
type operation struct {
	Kind     string
	ID       int
	Bottom   *int
	TypedNil bool
}
type request struct {
	Nodes      []nodeSpec
	Parents    []*int
	Operations []operation
}

func evaluate(q request) any {
	nodes := make([]ast.Node, 0)
	ids := map[ast.Node]int{}
	register := func(n ast.Node) { ids[n] = len(nodes); nodes = append(nodes, n) }
	pc := parser.NewContext()
	for _, s := range q.Nodes {
		var n ast.Node
		switch s.Kind {
		case "Container":
			n = ast.NewParagraph()
		case "Text":
			t := ast.NewTextSegment(s.Segment)
			t.SetRaw(s.Raw)
			t.SetSoftLineBreak(s.Soft)
			t.SetHardLineBreak(s.Hard)
			n = t
		case "Delimiter":
			d := parser.NewDelimiter(s.CanOpen, s.CanClose, s.Length, s.Char, builtinProcessor{s.Char, register})
			d.OriginalLength = s.OriginalLength
			d.Segment = s.Segment
			n = d
		default:
			panic("unknown node")
		}
		register(n)
	}
	for i, n := range nodes {
		if q.Parents[i] != nil {
			p := nodes[*q.Parents[i]]
			p.AppendChild(p, n)
		}
		if d, ok := n.(*parser.Delimiter); ok {
			pc.PushDelimiter(d)
		}
	}
	ast.OracleNewTextHook = register
	defer func() { ast.OracleNewTextHook = nil }()
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
	snapshot := func() {
		rows := make([]any, 0, len(nodes))
		for _, n := range nodes {
			var data any
			switch n := n.(type) {
			case *ast.Text:
				r := map[string]any{"kind": "Text", "segment": n.Segment, "raw": n.IsRaw()}
				if n.SoftLineBreak() {
					r["soft_line_break"] = true
				}
				if n.HardLineBreak() {
					r["hard_line_break"] = true
				}
				data = r
			case *ast.Emphasis:
				data = map[string]any{"kind": "Emphasis", "level": n.Level, "position": n.Pos()}
			case *east.Strikethrough:
				data = map[string]any{"kind": "Strikethrough", "position": n.Pos()}
			case *parser.Delimiter:
				data = map[string]any{"kind": "Delimiter", "segment": n.Segment, "can_open": n.CanOpen, "can_close": n.CanClose, "length": n.Length, "original_length": n.OriginalLength, "char": n.Char, "previous_delimiter": id(n.PreviousDelimiter), "next_delimiter": id(n.NextDelimiter)}
			default:
				data = map[string]any{"kind": "Container"}
			}
			rows = append(rows, map[string]any{"data": data, "parent": id(n.Parent()), "previous": id(n.PreviousSibling()), "next": id(n.NextSibling()), "first": id(n.FirstChild()), "last": id(n.LastChild()), "child_count": n.ChildCount()})
		}
		snapshots = append(snapshots, map[string]any{"nodes": rows, "first_delimiter": id(pc.FirstDelimiter()), "last_delimiter": id(pc.LastDelimiter())})
	}
	snapshot()
	for _, op := range q.Operations {
		switch op.Kind {
		case "process":
			var bottom ast.Node
			if op.Bottom != nil {
				bottom = nodes[*op.Bottom]
			}
			if op.TypedNil {
				bottom = (*parser.Delimiter)(nil)
			}
			parser.ProcessDelimiters(bottom, pc)
		case "remove":
			pc.RemoveDelimiter(nodes[op.ID].(*parser.Delimiter))
		case "clear":
			var bottom ast.Node
			if op.Bottom != nil {
				bottom = nodes[*op.Bottom]
			}
			pc.ClearDelimiters(bottom)
		default:
			panic("unknown operation")
		}
		snapshot()
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
