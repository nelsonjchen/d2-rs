// SPDX-License-Identifier: MPL-2.0
package d2ir

import (
	"github.com/d2lang/d2/d2ast"
	"github.com/d2lang/d2/d2parser"
	"strings"
)

type SelectionRequest struct {
	Input  string `json:"input"`
	Import string `json:"import"`
}

func OracleSelection(r SelectionRequest) any {
	a, e := d2parser.Parse("source.d2", strings.NewReader(r.Input), nil)
	if e != nil {
		panic(e)
	}
	c := &compiler{err: &d2parser.ParseError{}}
	root := &Map{}
	root.initRoot()
	root.parent.(*Field).References[0].Context_.Scope = a
	root.parent.(*Field).References[0].Context_.ScopeAST = a
	c.compileMap(root, a, a)
	c.compileSubstitutions(root, nil)
	c.overlayClasses(root)
	root.removeSuspendedFields()
	if len(c.err.Errors) > 0 {
		panic(c.err)
	}
	impAST, e := d2parser.Parse("import.d2", strings.NewReader("x: "+r.Import), nil)
	if e != nil {
		panic(e)
	}
	imp := impAST.Nodes[0].MapKey.Value.Import
	snapshot := func() []any {
		out := []any{}
		var visit func(Node)
		visit = func(n Node) {
			scopes := []bool{}
			children := []Node{}
			kind := ""
			switch n := n.(type) {
			case *Map:
				kind = "map"
				for _, f := range n.Fields {
					children = append(children, f)
				}
				for _, e := range n.Edges {
					children = append(children, e)
				}
			case *Field:
				kind = "field"
				for _, r := range n.References {
					scopes = append(scopes, r.Context_.ScopeMap == nil)
				}
				if n.Primary() != nil {
					children = append(children, n.Primary())
				}
				if n.Composite != nil {
					children = append(children, n.Composite)
				}
			case *Edge:
				kind = "edge"
				for _, r := range n.References {
					scopes = append(scopes, r.Context_.ScopeMap == nil)
				}
				if n.Primary() != nil {
					children = append(children, n.Primary())
				}
				if n.Map() != nil {
					children = append(children, n.Map())
				}
			case *Array:
				kind = "array"
				for _, v := range n.Values {
					children = append(children, v)
				}
			case *Scalar:
				kind = "scalar"
			}
			out = append(out, map[string]any{"kind": kind, "nil_scopes": scopes, "safe": importTemplateSafeAt(n)})
			for _, ch := range children {
				visit(ch)
			}
		}
		visit(root)
		return out
	}
	before := snapshot()
	selected, ok := c.selectImport(root, imp)
	var selectedAST d2ast.Node
	if ok {
		selectedAST = selected.AST()
	}
	errors := []d2ast.Error{}
	errors = append(errors, c.err.Errors...)
	return map[string]any{"before": before, "after": snapshot(), "selected": selectedAST, "errors": errors, "ok": ok}
}
