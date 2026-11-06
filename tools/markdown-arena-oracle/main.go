// SPDX-License-Identifier: MPL-2.0
package main

import (
	"encoding/json"
	"github.com/yuin/goldmark/ast"
	"os"
)

type operation struct {
	Kind   string
	Parent int
	Target *int
	Child  int
}

func evaluate(ops []operation) any {
	nodes := make([]ast.Node, 8)
	ids := map[ast.Node]int{}
	for i := range nodes {
		nodes[i] = ast.NewParagraph()
		ids[nodes[i]] = i
	}
	for _, i := range []int{2, 3, 4} {
		nodes[0].AppendChild(nodes[0], nodes[i])
	}
	for _, i := range []int{5, 6} {
		nodes[1].AppendChild(nodes[1], nodes[i])
	}
	id := func(n ast.Node) any {
		if n == nil {
			return nil
		}
		return ids[n]
	}
	snapshots := make([]any, 0)
	snapshot := func() {
		rows := make([]any, 0, len(nodes))
		for i, n := range nodes {
			rows = append(rows, map[string]any{"data": i, "parent": id(n.Parent()), "previous": id(n.PreviousSibling()), "next": id(n.NextSibling()), "first": id(n.FirstChild()), "last": id(n.LastChild()), "child_count": n.ChildCount()})
		}
		snapshots = append(snapshots, rows)
	}
	snapshot()
	for _, op := range ops {
		parent, child := nodes[op.Parent], nodes[op.Child]
		var target ast.Node
		if op.Target != nil {
			target = nodes[*op.Target]
		}
		switch op.Kind {
		case "append":
			parent.AppendChild(parent, child)
		case "remove":
			parent.RemoveChild(parent, child)
		case "clear":
			parent.RemoveChildren(parent)
		case "before":
			parent.InsertBefore(parent, target, child)
		case "after":
			parent.InsertAfter(parent, target, child)
		case "replace":
			parent.ReplaceChild(parent, target, child)
		default:
			panic("unknown operation")
		}
		snapshot()
	}
	return snapshots
}
func main() {
	var requests [][]operation
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
