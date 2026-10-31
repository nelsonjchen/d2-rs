// SPDX-License-Identifier: MPL-2.0
// d2compiler.Compile -> d2oracle.Move -> d2format.Format over the move
// controls of tools/fixtures/inputs/mathtext-txt/editor-move.json (recipes
// mathtext-txt-editor-move-*), adapted from tools/probes/editor-move-v090.
//
// $D2RS_EDITOR_MOVE names the seed section. Each control runs with
// include_descendants false and true ("-descFalse"/"-descTrue" names). The
// entry "upstream TestMove" expands to every TestMove table case of this
// package's edit_test.go with its own includeDescendants, and "upstream
// TestMove, flipped descendants" to the same cases with it negated (names
// suffixed "-flipped-desc"). A Move panic is recorded as {"input", "panic"}.
package d2oracle_test

import (
	"encoding/json"
	"fmt"
	"go/ast"
	"go/parser"
	"go/token"
	"os"
	"strconv"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2compiler"
	"github.com/d2lang/d2/d2format"
	"github.com/d2lang/d2/d2oracle"
	"github.com/d2lang/util-go/mapfs"
)

type d2rsMoveInput struct {
	Name          string            `json:"name"`
	Source        string            `json:"source"`
	Old           string            `json:"old_key"`
	New           string            `json:"new_key"`
	Desc          bool              `json:"include_descendants"`
	Path          string            `json:"path,omitempty"`
	Board         []string          `json:"board_path,omitempty"`
	Files         map[string]string `json:"files,omitempty"`
	Expected      string            `json:"fixture_expected_source"`
	ExpectedError string            `json:"fixture_expected_error"`
	Skip          bool              `json:"upstream_skip"`
}

func d2rsMoveString(e ast.Expr) string {
	if x, ok := e.(*ast.BasicLit); ok && x.Kind == token.STRING {
		s, err := strconv.Unquote(x.Value)
		if err != nil {
			panic(err)
		}
		return s
	}
	if x, ok := e.(*ast.BinaryExpr); ok && x.Op == token.ADD {
		return d2rsMoveString(x.X) + d2rsMoveString(x.Y)
	}
	panic(fmt.Sprintf("nonliteral string %T", e))
}

// d2rsUpstreamMoves extracts the TestMove table of edit_test.go.
func d2rsUpstreamMoves(t *testing.T) []d2rsMoveInput {
	file, err := parser.ParseFile(token.NewFileSet(), "edit_test.go", nil, 0)
	if err != nil {
		t.Fatal(err)
	}
	rows := []d2rsMoveInput{}
	for _, d := range file.Decls {
		fn, ok := d.(*ast.FuncDecl)
		if !ok || fn.Name.Name != "TestMove" {
			continue
		}
		ast.Inspect(fn.Body, func(n ast.Node) bool {
			assign, ok := n.(*ast.AssignStmt)
			if !ok || len(assign.Lhs) != 1 {
				return true
			}
			id, ok := assign.Lhs[0].(*ast.Ident)
			if !ok || id.Name != "testCases" {
				return true
			}
			for _, e := range assign.Rhs[0].(*ast.CompositeLit).Elts {
				in := d2rsMoveInput{}
				for _, f := range e.(*ast.CompositeLit).Elts {
					field := f.(*ast.KeyValueExpr)
					switch field.Key.(*ast.Ident).Name {
					case "name":
						in.Name = d2rsMoveString(field.Value)
					case "text":
						in.Source = d2rsMoveString(field.Value)
					case "key":
						in.Old = d2rsMoveString(field.Value)
					case "newKey":
						in.New = d2rsMoveString(field.Value)
					case "includeDescendants":
						in.Desc = field.Value.(*ast.Ident).Name == "true"
					case "skip":
						in.Skip = field.Value.(*ast.Ident).Name == "true"
					case "exp":
						in.Expected = d2rsMoveString(field.Value)
					case "expErr":
						in.ExpectedError = d2rsMoveString(field.Value)
					case "boardPath":
						for _, v := range field.Value.(*ast.CompositeLit).Elts {
							in.Board = append(in.Board, d2rsMoveString(v))
						}
					case "fsTexts":
						in.Files = map[string]string{}
						for _, v := range field.Value.(*ast.CompositeLit).Elts {
							kv := v.(*ast.KeyValueExpr)
							in.Files[d2rsMoveString(kv.Key)] = d2rsMoveString(kv.Value)
						}
					}
				}
				in.Name = "upstream/" + in.Name
				if in.Path == "" {
					in.Path = "-"
				}
				rows = append(rows, in)
			}
			return false
		})
	}
	if len(rows) == 0 {
		t.Fatal("no TestMove testCases found in edit_test.go")
	}
	return rows
}

func d2rsRunMove(in d2rsMoveInput) (row map[string]any) {
	defer func() {
		if p := recover(); p != nil {
			row = map[string]any{"input": in, "panic": fmt.Sprint(p)}
		}
	}()
	files := map[string]string{}
	for k, v := range in.Files {
		files[k] = v
	}
	entry := in.Path
	if entry == "" {
		entry = "input.d2"
	}
	files[entry] = in.Source
	fs, err := mapfs.New(files)
	if err != nil {
		panic(err)
	}
	defer fs.Close()
	g, _, err := d2compiler.Compile(entry, strings.NewReader(in.Source), &d2compiler.CompileOptions{FS: fs})
	out := ""
	if err == nil {
		g, err = d2oracle.Move(g, in.Board, in.Old, in.New, in.Desc)
		if err == nil {
			out = d2format.Format(g.AST)
		}
	}
	message := ""
	if err != nil {
		message = err.Error()
	}
	return map[string]any{"input": in, "source": out, "error": message}
}

func TestD2RSEditorMove(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var seeds map[string][]json.RawMessage
	if err := json.Unmarshal(data, &seeds); err != nil {
		t.Fatal(err)
	}
	section, ok := seeds[os.Getenv("D2RS_EDITOR_MOVE")]
	if !ok {
		t.Fatalf("no seed section D2RS_EDITOR_MOVE=%q", os.Getenv("D2RS_EDITOR_MOVE"))
	}
	inputs := []d2rsMoveInput{}
	for _, raw := range section {
		var marker string
		if json.Unmarshal(raw, &marker) == nil {
			flip := marker == "upstream TestMove, flipped descendants"
			if !flip && marker != "upstream TestMove" {
				t.Fatalf("unknown seed entry %q", marker)
			}
			for _, in := range d2rsUpstreamMoves(t) {
				if flip {
					in.Desc = !in.Desc
					in.Name += "-flipped-desc"
				}
				inputs = append(inputs, in)
			}
			continue
		}
		var control d2rsMoveInput
		if err := json.Unmarshal(raw, &control); err != nil {
			t.Fatal(err)
		}
		for _, desc := range []bool{false, true} {
			in := control
			in.Desc = desc
			in.Name = fmt.Sprintf("%s-desc%s", control.Name, map[bool]string{false: "False", true: "True"}[desc])
			inputs = append(inputs, in)
		}
	}
	rows := []any{}
	for _, in := range inputs {
		rows = append(rows, d2rsRunMove(in))
	}
	encoded, err := json.MarshalIndent(map[string]any{"source": "d2compiler.Compile -> d2oracle.Move -> d2format.Format", "cases": rows}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(encoded, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
