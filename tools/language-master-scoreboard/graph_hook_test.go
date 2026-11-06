// SPDX-License-Identifier: MPL-2.0
// Observe original graph materialization calls without changing their assertions.
package d2compiler

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/d2ast"
	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2ir"
)

type d2rsGraphInput struct {
	source string
	limit  int64
}

var d2rsGraphMu sync.Mutex
var d2rsGraphInputs = map[*d2ir.Map]d2rsGraphInput{}
var d2rsGraphCalls = map[string]int{}

func d2rsGraphIR(t *testing.T, source string, limit int64) (*d2ast.Map, *d2ir.Map) {
	ast, ir := compileExpansionIR(t, source, limit)
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") != "" {
		d2rsGraphMu.Lock()
		d2rsGraphInputs[ir] = d2rsGraphInput{source, limit}
		d2rsGraphMu.Unlock()
	}
	return ast, ir
}

// Describe shared composite identities by their first source field path. This
// captures the actual post-Compile mutation, rather than rebuilding the test.
func d2rsGraphAliases(root *d2ir.Map) []map[string]interface{} {
	aliases := []map[string]interface{}{}
	seen := map[d2ir.Composite][]string{root: {}}
	var walk func(*d2ir.Map, []string)
	walk = func(m *d2ir.Map, parent []string) {
		for _, f := range m.Fields {
			if f.Composite == nil {
				continue
			}
			path := append(append([]string{}, parent...), f.Name.ScalarString())
			if prior, ok := seen[f.Composite]; ok {
				aliases = append(aliases, map[string]interface{}{"field": path, "target": prior})
				continue
			}
			seen[f.Composite] = path
			if child := f.Map(); child != nil {
				walk(child, path)
			}
		}
	}
	walk(root, nil)
	return aliases
}

func d2rsGraphCompile(t *testing.T, ctx context.Context, ast *d2ast.Map, ir *d2ir.Map) (*d2graph.Graph, error) {
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return compileIRContext(ctx, ast, ir)
	}
	d2rsGraphMu.Lock()
	input, ok := d2rsGraphInputs[ir]
	call := d2rsGraphCalls[t.Name()]
	d2rsGraphCalls[t.Name()]++
	d2rsGraphMu.Unlock()
	if !ok {
		t.Fatal("missing original graph IR input")
	}
	policy := map[string]interface{}{"kind": "background"}
	if c, ok := ctx.(*graphCancelAfterErrContext); ok {
		policy = map[string]interface{}{"kind": "cancel_after_checks", "cancel_at": c.cancelAt, "initial_calls": c.calls}
	}
	aliases := d2rsGraphAliases(ir)
	graph, err := compileIRContext(ctx, ast, ir)
	text, kind := "", ""
	if err != nil {
		text = err.Error()
	}
	if errors.Is(err, context.Canceled) {
		kind = "canceled"
	}
	if errors.Is(err, context.DeadlineExceeded) {
		kind = "deadline"
	}
	var checks interface{}
	if c, ok := ctx.(*graphCancelAfterErrContext); ok {
		checks = c.calls
	}
	name := fmt.Sprintf("%s#%d", t.Name(), call)
	value := map[string]interface{}{"name": name, "test": t.Name(), "request": map[string]interface{}{"path": "expansion-internal.d2", "input": input.source, "max_variable_expansion": input.limit, "composite_aliases": aliases, "context_policy": policy}, "got": map[string]interface{}{"graph": graph, "error": text, "error_kind": kind, "context_checks": checks}}
	data, marshalErr := json.Marshal(value)
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"), id+".json"), data, 0644); err != nil {
		t.Fatal(err)
	}
	return graph, err
}
