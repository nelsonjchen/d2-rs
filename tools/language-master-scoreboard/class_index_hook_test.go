// SPDX-License-Identifier: MPL-2.0
// Env-gated observation of the original class lookup tests; assertions stay active.
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
	"github.com/d2lang/d2/d2ir"
)

type d2rsClassSession struct {
	source  string
	queries []string
	results []*d2ir.Map
	policy  interface{}
	c       *compiler
}

var d2rsClassMu sync.Mutex
var d2rsClassInputs = map[*d2ir.Map]string{}
var d2rsClassSessions = map[string]*d2rsClassSession{}

func d2rsClassIR(t *testing.T, source string, limit int64) (*d2ast.Map, *d2ir.Map) {
	ast, ir := compileExpansionIR(t, source, limit)
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") != "" {
		d2rsClassMu.Lock()
		d2rsClassInputs[ir] = source
		d2rsClassMu.Unlock()
	}
	return ast, ir
}
func d2rsClassLookup(t *testing.T, c *compiler, ir *d2ir.Map, name string) *d2ir.Map {
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return c.getClassMap(ir, name)
	}
	d2rsClassMu.Lock()
	session := d2rsClassSessions[t.Name()]
	if session == nil {
		source, ok := d2rsClassInputs[ir]
		if !ok {
			t.Fatal("missing original class IR input")
		}
		var policy interface{}
		if c.ctx == nil || c.ctx == context.Background() {
			policy = map[string]interface{}{"kind": "background"}
		}
		if ctx, ok := c.ctx.(*graphCancelAfterErrContext); ok {
			policy = map[string]interface{}{"kind": "cancel_after_checks", "cancel_at": ctx.cancelAt, "initial_calls": ctx.calls}
		}
		session = &d2rsClassSession{source: source, policy: policy, c: c, queries: []string{}, results: []*d2ir.Map{}}
		d2rsClassSessions[t.Name()] = session
		t.Cleanup(func() { d2rsClassWrite(t, session) })
	}
	d2rsClassMu.Unlock()
	got := c.getClassMap(ir, name)
	session.queries = append(session.queries, name)
	session.results = append(session.results, got)
	return got
}
func d2rsClassWrite(t *testing.T, s *d2rsClassSession) {
	text, kind := "", ""
	if s.c.fatalErr != nil {
		text = s.c.fatalErr.Error()
	}
	if errors.Is(s.c.fatalErr, context.Canceled) {
		kind = "canceled"
	}
	if errors.Is(s.c.fatalErr, context.DeadlineExceeded) {
		kind = "deadline"
	}
	var checks interface{}
	if ctx, ok := s.c.ctx.(*graphCancelAfterErrContext); ok {
		checks = ctx.calls
	}
	name := t.Name() + "#0"
	value := map[string]interface{}{"name": name, "test": t.Name(), "request": map[string]interface{}{"path": "expansion-internal.d2", "input": s.source, "queries": s.queries, "context_policy": s.policy}, "got": map[string]interface{}{"results": s.results, "error": text, "error_kind": kind, "indexed_roots": len(s.c.classMapsByRoot), "context_checks": checks}}
	data, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"), id+".json"), data, 0644); err != nil {
		t.Fatal(err)
	}
}
