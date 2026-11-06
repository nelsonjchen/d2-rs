// SPDX-License-Identifier: MPL-2.0
// Observe original IR copy assertions without changing their inputs or results.
package d2ir

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/d2ast"
	"github.com/d2lang/d2/d2parser"
)

var d2rsCopyMu sync.Mutex
var d2rsCopyInputs = map[*d2ast.Map]map[string]interface{}{}
var d2rsCopySessions = map[*Map]*d2rsCopySession{}
var d2rsCopyCalls = map[string]int{}

type d2rsCopySession struct {
	request map[string]interface{}
	results []interface{}
}

func d2rsCopyWrite(t *testing.T, request interface{}, got interface{}) {
	t.Helper()
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return
	}
	call := d2rsCopyCalls[t.Name()]
	d2rsCopyCalls[t.Name()]++
	name := fmt.Sprintf("%s#%d", t.Name(), call)
	data, err := json.Marshal(map[string]interface{}{"name": name, "test": t.Name(), "request": request, "got": got})
	if err != nil {
		t.Fatal(err)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"), id+".json"), data, 0644); err != nil {
		t.Fatal(err)
	}
}
func d2rsCopyParse(t *testing.T, path string, r io.Reader, opts *d2parser.ParseOptions) (*d2ast.Map, error) {
	data, err := io.ReadAll(r)
	if err != nil {
		return nil, err
	}
	ast, err := d2parser.Parse(path, bytes.NewReader(data), opts)
	d2rsCopyMu.Lock()
	defer d2rsCopyMu.Unlock()
	d2rsCopyInputs[ast] = map[string]interface{}{"path": path, "input": string(data)}
	return ast, err
}
func d2rsCopyResult(root *Map, imports []string, err error) interface{} {
	text, kind := "", ""
	if err != nil {
		text = err.Error()
	}
	if errors.Is(err, context.Canceled) {
		kind = "canceled"
	}
	if errors.Is(err, context.DeadlineExceeded) {
		kind = "deadline_exceeded"
	}
	var limit, used interface{}
	if root != nil {
		b := variableExpansionBudgetFor(root)
		if b != nil {
			limit = b.limit
			used = b.used
		}
	}
	return map[string]interface{}{"ir": root, "imports": imports, "error": text, "error_kind": kind, "limit": limit, "used": used}
}
func d2rsCopyCompile(t *testing.T, ast *d2ast.Map, opts *CompileOptions) (*Map, []string, error) {
	root, imports, err := Compile(ast, opts)
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return root, imports, err
	}
	d2rsCopyMu.Lock()
	defer d2rsCopyMu.Unlock()
	input, ok := d2rsCopyInputs[ast]
	if !ok {
		t.Fatal("missing original parse")
	}
	files := map[string]string{}
	limit := int64(0)
	if opts != nil {
		limit = opts.MaxVariableExpansion
		if opts.FS != nil {
			if err := fs.WalkDir(opts.FS, ".", func(p string, e fs.DirEntry, err error) error {
				if err != nil {
					return err
				}
				if e.IsDir() {
					return nil
				}
				data, err := fs.ReadFile(opts.FS, p)
				if err == nil {
					files[p] = string(data)
				}
				return err
			}); err != nil {
				t.Fatal(err)
			}
		}
	}
	s := &d2rsCopySession{request: map[string]interface{}{"op": "compile", "path": input["path"], "input": input["input"], "max_variable_expansion": limit, "files": files, "has_fs": opts != nil && opts.FS != nil, "copies": []interface{}{}}, results: []interface{}{d2rsCopyResult(root, imports, err)}}
	d2rsCopySessions[root] = s
	t.Cleanup(func() {
		d2rsCopyMu.Lock()
		defer d2rsCopyMu.Unlock()
		d2rsCopyWrite(t, s.request, map[string]interface{}{"results": s.results})
	})
	return root, imports, err
}
func d2rsCopyReserve(t *testing.T, ctx context.Context, root *Map) error {
	err := ReserveVariableExpansionCopy(ctx, root)
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return err
	}
	d2rsCopyMu.Lock()
	defer d2rsCopyMu.Unlock()
	s, ok := d2rsCopySessions[root]
	if !ok {
		t.Fatal("copy without captured compile")
	}
	if ctx != context.Background() {
		t.Fatal("uncaptured copy context")
	}
	s.request["copies"] = append(s.request["copies"].([]interface{}), map[string]interface{}{"context": "background"})
	s.results = append(s.results, d2rsCopyResult(root, nil, err))
	return err
}
func d2rsCopyUnits(t *testing.T, n Node) int64 {
	units := variableExpansionNodeUnits(n)
	request := map[string]interface{}{"op": "node_units", "node": n}
	// Keep nil slice members and reference slots. Go's default node JSON omits
	// empty references, so those allocated slices are described independently.
	switch n := n.(type) {
	case *Field:
		request["references"] = len(n.References)
	case *Edge:
		request["references"] = len(n.References)
	}
	d2rsCopyMu.Lock()
	defer d2rsCopyMu.Unlock()
	d2rsCopyWrite(t, request, map[string]interface{}{"units": units})
	return units
}
