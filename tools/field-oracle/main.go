// SPDX-License-Identifier: MPL-2.0
package main

import (
	"encoding/hex"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/d2lang/d2/d2ast"
	"github.com/d2lang/d2/d2ir"
	"github.com/d2lang/d2/d2parser"
	goast "go/ast"
	"go/parser"
	"go/token"
	"io"
	"os"
	"strconv"
)

func main() {
	if len(os.Args) > 2 && os.Args[1] == "import-corpus" {
		extractImportCorpus(os.Args[2])
		return
	}

	if len(os.Args) > 1 && os.Args[1] == "compile-imports" {
		var requests []struct {
			Input    string            `json:"input"`
			Path     string            `json:"path"`
			Files    map[string]string `json:"files"`
			FilesHex map[string]string `json:"files_hex"`
			UTF16    bool              `json:"utf16"`
		}
		if e := json.NewDecoder(os.Stdin).Decode(&requests); e != nil {
			panic(e)
		}
		out := []any{}
		for _, r := range requests {
			if r.Files == nil {
				r.Files = map[string]string{}
			}
			for path, encoded := range r.FilesHex {
				b, e := hex.DecodeString(encoded)
				if e != nil {
					panic(e)
				}
				r.Files[path] = string(b)
			}
			out = append(out, d2ir.OracleCompileImports(r.Input, r.Path, r.Files, r.UTF16))
		}
		json.NewEncoder(os.Stdout).Encode(out)
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "stream-parse" {
		var requests []streamRequest
		if e := json.NewDecoder(os.Stdin).Decode(&requests); e != nil {
			panic(e)
		}
		out := []any{}
		for _, r := range requests {
			out = append(out, streamParse(r))
		}
		json.NewEncoder(os.Stdout).Encode(out)
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "import-loading" {
		var r []d2ir.OracleImportRequest
		if e := json.NewDecoder(os.Stdin).Decode(&r); e != nil {
			panic(e)
		}
		out := []any{}
		for _, request := range r {
			out = append(out, d2ir.OracleImportLoading(request))
		}
		json.NewEncoder(os.Stdout).Encode(out)
		return
	}
	if len(os.Args) > 1 && os.Args[1] == "array-capacities" {
		var requests [][2]int
		if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
			panic(err)
		}
		out := []int{}
		for _, r := range requests {
			values := make([]any, r[0])
			values = append(values, make([]any, r[1]-r[0])...)
			out = append(out, cap(values))
		}
		json.NewEncoder(os.Stdout).Encode(out)
		return
	}

	if len(os.Args) > 2 && os.Args[1] == "literals" {
		fset := token.NewFileSet()
		file, err := parser.ParseFile(fset, os.Args[2], nil, 0)
		if err != nil {
			panic(err)
		}
		rows := []any{}
		goast.Inspect(file, func(n goast.Node) bool {
			call, ok := n.(*goast.CallExpr)
			if !ok || len(call.Args) < 2 {
				return true
			}
			fn, ok := call.Fun.(*goast.Ident)
			callName := "compile"
			if len(os.Args) > 3 {
				callName = os.Args[3]
			}
			if !ok || fn.Name != callName {
				return true
			}
			lit, ok := call.Args[1].(*goast.BasicLit)
			if !ok || lit.Kind != token.STRING {
				return true
			}
			input, err := strconv.Unquote(lit.Value)
			if err != nil {
				panic(err)
			}
			rows = append(rows, map[string]any{"input": input, "origin": fmt.Sprintf("%s:%d", os.Args[2], fset.Position(call.Pos()).Line)})
			return true
		})
		json.NewEncoder(os.Stdout).Encode(rows)
		return
	}

	if len(os.Args) > 1 && os.Args[1] == "compile" {
		var requests []struct {
			Input string
			Path  string
		}
		if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
			panic(err)
		}
		out := []any{}
		for _, r := range requests {
			out = append(out, compileObserved(r.Input, r.Path))
		}
		json.NewEncoder(os.Stdout).Encode(out)
		return
	}

	var requests [][]d2ir.OracleFieldOperation
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	out := []any{}
	for _, ops := range requests {
		out = append(out, d2ir.OracleFieldScenario(ops))
	}
	if err := json.NewEncoder(os.Stdout).Encode(out); err != nil {
		panic(err)
	}
}

// Compiler panics are classified separately from successful/error IR results.
// This recovery exists only in the development oracle.
func compileObserved(input, path string) (out any) {
	defer func() {
		if recover() != nil {
			out = map[string]any{"panic": true}
		}
	}()
	return d2ir.OracleCompileFields(input, path)
}

type streamRequest struct {
	Load      bool   `json:"load"`
	Hex       string `json:"hex"`
	Chunk     int    `json:"chunk"`
	ErrorAt   *int   `json:"error_at"`
	Combined  bool   `json:"combined"`
	ZeroReads int    `json:"zero_reads"`
	UTF16     bool   `json:"utf16"`
	Seed      bool   `json:"seed"`
}
type streamFaultReader struct {
	r     streamRequest
	data  []byte
	pos   int
	ended bool
}

func (f *streamFaultReader) Read(dst []byte) (int, error) {
	if f.r.ZeroReads > 0 {
		f.r.ZeroReads--
		return 0, nil
	}
	if f.ended {
		return 0, io.EOF
	}
	if f.r.ErrorAt != nil && f.pos == *f.r.ErrorAt {
		f.ended = true
		return 0, errors.New("read fault")
	}
	if f.pos == len(f.data) {
		f.ended = true
		return 0, io.EOF
	}
	n := len(f.data) - f.pos
	if n > len(dst) {
		n = len(dst)
	}
	if f.r.Chunk > 0 && n > f.r.Chunk {
		n = f.r.Chunk
	}
	if f.r.ErrorAt != nil && f.pos+n > *f.r.ErrorAt {
		n = *f.r.ErrorAt - f.pos
	}
	copy(dst, f.data[f.pos:f.pos+n])
	f.pos += n
	if f.r.Combined {
		if f.r.ErrorAt != nil && f.pos == *f.r.ErrorAt {
			f.ended = true
			return n, errors.New("read fault")
		}
		if f.pos == len(f.data) {
			f.ended = true
			return n, io.EOF
		}
	}
	return n, nil
}
func streamParse(r streamRequest) any {
	data, e := hex.DecodeString(r.Hex)
	if e != nil {
		panic(e)
	}
	if r.Load {
		return d2ir.OracleLoadStream(&streamFaultReader{r: r, data: data}, r.Seed, r.UTF16)
	}
	accumulator := &d2parser.ParseError{}
	if r.Seed {
		accumulator.Errors = append(accumulator.Errors, d2ast.Error{Message: "existing error"})
	}
	parsed, e := d2parser.Parse("stream.d2", &streamFaultReader{r: r, data: data}, &d2parser.ParseOptions{UTF16Pos: r.UTF16, ParseError: accumulator})
	var structured any
	display := ""
	if e != nil {
		structured = e
		display = e.Error()
	}
	return map[string]any{"map": parsed, "error": display, "structured": structured, "accumulator": accumulator}
}

func extractImportCorpus(path string) {
	fset := token.NewFileSet()
	f, e := parser.ParseFile(fset, path, nil, 0)
	if e != nil {
		panic(e)
	}
	text := func(expr goast.Expr) string {
		literal, ok := expr.(*goast.BasicLit)
		if !ok || literal.Kind != token.STRING {
			panic("expected literal")
		}
		s, e := strconv.Unquote(literal.Value)
		if e != nil {
			panic(e)
		}
		return s
	}
	rows := []any{}
	goast.Inspect(f, func(n goast.Node) bool {
		if call, ok := n.(*goast.CallExpr); ok {
			name, ok := call.Fun.(*goast.Ident)
			if !ok || name.Name != "compileFS" {
				return true
			}
			path := text(call.Args[1])
			files := map[string]string{}
			for _, elt := range call.Args[2].(*goast.CompositeLit).Elts {
				kv := elt.(*goast.KeyValueExpr)
				files[text(kv.Key)] = text(kv.Value)
			}
			rows = append(rows, map[string]any{"path": path, "input": files[path], "files": files, "origin": fset.Position(call.Pos()).String()})
			return true
		}
		if composite, ok := n.(*goast.CompositeLit); ok {
			values := map[string]string{}
			for _, elt := range composite.Elts {
				kv, ok := elt.(*goast.KeyValueExpr)
				if !ok {
					continue
				}
				name, ok := kv.Key.(*goast.Ident)
				if !ok {
					continue
				}
				if name.Name == "library" || name.Name == "source" {
					values[name.Name] = text(kv.Value)
				}
			}
			if len(values) == 2 {
				rows = append(rows, map[string]any{"path": "index.d2", "input": values["source"], "files": map[string]string{"library.d2": values["library"]}, "origin": fset.Position(composite.Pos()).String()})
			}
		}
		return true
	})
	json.NewEncoder(os.Stdout).Encode(rows)
}
