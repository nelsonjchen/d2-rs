// SPDX-License-Identifier: MPL-2.0
package main

import (
	"encoding/hex"
	"encoding/json"
	"github.com/yuin/goldmark/ast"
	"github.com/yuin/goldmark/text"
	"os"
)

type node struct {
	Kind     string       `json:"kind"`
	Segment  text.Segment `json:"segment"`
	Raw      bool         `json:"raw"`
	Soft     bool         `json:"soft_line_break"`
	Hard     bool         `json:"hard_line_break"`
	Children []node       `json:"children"`
}

func makeNode(n node) ast.Node {
	if n.Kind == "Text" {
		t := ast.NewTextSegment(n.Segment)
		t.SetRaw(n.Raw)
		t.SetSoftLineBreak(n.Soft)
		t.SetHardLineBreak(n.Hard)
		return t
	}
	c := ast.NewCodeSpan()
	for _, child := range n.Children {
		c.AppendChild(c, makeNode(child))
	}
	return c
}
func observe(n ast.Node) any {
	if t, ok := n.(*ast.Text); ok {
		r := map[string]any{"kind": "Text", "segment": t.Segment, "raw": t.IsRaw()}
		if t.SoftLineBreak() {
			r["soft_line_break"] = true
		}
		if t.HardLineBreak() {
			r["hard_line_break"] = true
		}
		return r
	}
	children := make([]any, 0)
	for c := n.FirstChild(); c != nil; c = c.NextSibling() {
		children = append(children, observe(c))
	}
	return map[string]any{"kind": "CodeSpan", "children": children}
}

type request struct {
	Source   string
	Children []node
	Action   string
	Index    int
	Right    node
	Segment  text.Segment
}

func evaluate(q request) any {
	source, err := hex.DecodeString(q.Source)
	if err != nil {
		panic(err)
	}
	parent := ast.NewParagraph()
	for _, n := range q.Children {
		parent.AppendChild(parent, makeNode(n))
	}
	var selected ast.Node
	for n, i := parent.FirstChild(), 0; n != nil; n, i = n.NextSibling(), i+1 {
		if i == q.Index {
			selected = n
			break
		}
	}
	var merged any
	switch q.Action {
	case "merge":
		merged = selected.(*ast.Text).Merge(makeNode(q.Right), source)
	case "append":
		ast.MergeOrAppendTextSegment(parent, q.Segment)
	case "replace":
		ast.MergeOrReplaceTextSegment(parent, selected, q.Segment)
	default:
		panic("unknown action")
	}
	children := make([]any, 0)
	for n := parent.FirstChild(); n != nil; n = n.NextSibling() {
		children = append(children, observe(n))
	}
	return map[string]any{"merged": merged, "children": children}
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
