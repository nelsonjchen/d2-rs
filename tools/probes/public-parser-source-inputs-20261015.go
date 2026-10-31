// SPDX-License-Identifier: MPL-2.0
// Static test-input discovery only; D2 parsing is performed by the pinned oracle.
package main

import (
	"encoding/hex"
	"encoding/json"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strconv"
	"strings"
)

type Entry struct {
	Source string `json:"source"`
	Line   int    `json:"line"`
	Column int    `json:"column"`
	Kind   string `json:"kind"`
	Hex    string `json:"hex"`
}
type Pending struct {
	Source string `json:"source"`
	Line   int    `json:"line"`
	Column int    `json:"column"`
	Kind   string `json:"kind"`
}

func constant(e ast.Expr) (string, bool) {
	switch x := e.(type) {
	case *ast.BasicLit:
		if x.Kind == token.STRING {
			s, err := strconv.Unquote(x.Value)
			return s, err == nil
		}
	case *ast.BinaryExpr:
		if x.Op == token.ADD {
			a, ok := constant(x.X)
			b, ok2 := constant(x.Y)
			return a + b, ok && ok2
		}
	case *ast.ParenExpr:
		return constant(x.X)
	}
	return "", false
}
func main() {
	root := os.Args[1]
	cmd := exec.Command("git", "ls-files", "-z")
	cmd.Dir = root
	raw, err := cmd.Output()
	if err != nil {
		panic(err)
	}
	files := strings.Split(strings.TrimSuffix(string(raw), "\x00"), "\x00")
	sort.Strings(files)
	entries := []Entry{}
	pending := []Pending{}
	scanned := []string{}
	for _, name := range files {
		if !strings.HasSuffix(name, "_test.go") {
			continue
		}
		set := token.NewFileSet()
		file, err := parser.ParseFile(set, filepath.Join(root, name), nil, 0)
		if err != nil {
			panic(err)
		}
		scanned = append(scanned, name)
		add := func(e ast.Expr, kind string) {
			p := set.Position(e.Pos())
			s, ok := constant(e)
			if ok {
				entries = append(entries, Entry{name, p.Line, p.Column, kind, hex.EncodeToString([]byte(s))})
			} else {
				pending = append(pending, Pending{name, p.Line, p.Column, kind})
			}
		}
		ast.Inspect(file, func(n ast.Node) bool {
			switch x := n.(type) {
			case *ast.KeyValueExpr:
				if key, ok := x.Key.(*ast.Ident); ok && (key.Name == "script" || key.Name == "text") {
					add(x.Value, "field-"+key.Name)
				} else if s, ok := constant(x.Key); ok && strings.HasSuffix(s, ".d2") {
					add(x.Value, "map-file")
				}
			case *ast.AssignStmt:
				for i, lhs := range x.Lhs {
					if key, ok := lhs.(*ast.Ident); ok && (key.Name == "script" || key.Name == "text") && i < len(x.Rhs) {
						add(x.Rhs[i], "assignment-"+key.Name)
					}
				}
			case *ast.ValueSpec:
				for i, key := range x.Names {
					if (key.Name == "script" || key.Name == "text") && i < len(x.Values) {
						add(x.Values[i], "declaration-"+key.Name)
					}
				}
			case *ast.CallExpr:
				if sel, ok := x.Fun.(*ast.SelectorExpr); ok && sel.Sel.Name == "NewReader" && len(x.Args) == 1 {
					if base, ok := sel.X.(*ast.Ident); ok && base.Name == "strings" {
						add(x.Args[0], "reader")
					}
				}
			}
			return true
		})
	}
	if err := json.NewEncoder(os.Stdout).Encode(map[string]any{"scanned_test_files": scanned, "entries": entries, "pending_nonconstant": pending}); err != nil {
		panic(err)
	}
}
