// SPDX-License-Identifier: MPL-2.0
// Capture the original internal tests without replacing any assertions.
package d2ir

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/d2ast"
	"github.com/d2lang/d2/d2parser"
)

var d2rsInternalMu sync.Mutex
var d2rsInternalInputs = map[*d2ast.Map]map[string]interface{}{}
var d2rsInternalKeys = map[*d2ast.Key]string{}
var d2rsInternalBases = map[string]string{}
var d2rsInternalEdges = map[string]*d2rsInternalSession{}
var d2rsInternalCalls = map[string]int{}

type d2rsInternalSession struct {
	request      map[string]interface{}
	observations []interface{}
	observed     bool
}

func d2rsInternalWrite(t *testing.T, request, got, observations interface{}) {
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return
	}
	call := d2rsInternalCalls[t.Name()]
	d2rsInternalCalls[t.Name()]++
	name := fmt.Sprintf("%s#%d", t.Name(), call)
	data, err := json.Marshal(map[string]interface{}{"name": name, "test": t.Name(), "request": request, "got": got, "observations": observations})
	if err != nil {
		t.Fatal(err)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"), id+".json"), data, 0644); err != nil {
		t.Fatal(err)
	}
}

type d2rsInternalBudget struct {
	variable                   *variableExpansionBudget
	glob                       *globExpansionBudget
	edge                       *edgeExpansionBudget
	work                       *edgeExpansionWorkBudget
	limit, used, createdFields int64
	ops                        []map[string]interface{}
	results                    []string
}

func d2rsInternalBudgetNew(t *testing.T, kind string, limit int64) (*d2rsInternalBudget, error) {
	b := &d2rsInternalBudget{ops: []map[string]interface{}{}, results: []string{}}
	var err error
	switch kind {
	case "variable":
		b.variable, err = newVariableExpansionBudget(limit)
	case "glob":
		b.glob, err = newGlobExpansionBudget(limit)
	case "edge":
		b.edge, err = newEdgeExpansionBudget(limit)
	case "work":
		b.work, err = newEdgeExpansionWorkBudget(limit)
	default:
		t.Fatal("unknown budget")
	}
	text := ""
	if err != nil {
		text = err.Error()
	} else {
		b.sync()
	}
	t.Cleanup(func() {
		d2rsInternalMu.Lock()
		defer d2rsInternalMu.Unlock()
		var state interface{}
		if err == nil {
			state = map[string]interface{}{"limit": b.limit, "used": b.used, "created_fields": b.createdFields}
		}
		d2rsInternalWrite(t, map[string]interface{}{"op": "budget", "kind": kind, "limit": limit, "operations": b.ops}, map[string]interface{}{"error": text, "results": b.results, "state": state}, nil)
	})
	if err != nil {
		return nil, err
	}
	return b, nil
}
func (b *d2rsInternalBudget) sync() {
	switch {
	case b.variable != nil:
		b.limit, b.used = b.variable.limit, b.variable.used
	case b.glob != nil:
		b.limit, b.used, b.createdFields = b.glob.limit, b.glob.used, b.glob.createdFields
	case b.edge != nil:
		b.limit, b.used = b.edge.limit, b.edge.used
	case b.work != nil:
		b.limit, b.used = b.work.limit, b.work.used
	}
}
func (b *d2rsInternalBudget) record(op string, units int64, err error) error {
	// Lossless run encoding of identical requests; keep EVERY returned error.
	if len(b.ops) > 0 && b.ops[len(b.ops)-1]["op"] == op && b.ops[len(b.ops)-1]["units"] == units {
		last := b.ops[len(b.ops)-1]
		last["count"] = last["count"].(int) + 1
	} else {
		b.ops = append(b.ops, map[string]interface{}{"op": op, "units": units, "count": 1})
	}
	text := ""
	if err != nil {
		text = err.Error()
	}
	b.results = append(b.results, text)
	b.sync()
	return err
}
func (b *d2rsInternalBudget) reserve(units ...int64) error {
	u := int64(1)
	if len(units) > 0 {
		u = units[0]
	}
	var err error
	switch {
	case b.variable != nil:
		err = b.variable.reserve(u)
	case b.glob != nil:
		err = b.glob.reserve(u)
	case b.edge != nil:
		err = b.edge.reserve()
	case b.work != nil:
		err = b.work.reserve()
	}
	return b.record("reserve", u, err)
}
func (b *d2rsInternalBudget) reserveField() error { return b.record("field", 1, b.glob.reserveField()) }

func d2rsInternalParse(t *testing.T, path string, r io.Reader, opts *d2parser.ParseOptions) (*d2ast.Map, error) {
	data, err := io.ReadAll(r)
	if err != nil {
		return nil, err
	}
	ast, err := d2parser.Parse(path, bytes.NewReader(data), opts)
	d2rsInternalMu.Lock()
	defer d2rsInternalMu.Unlock()
	d2rsInternalInputs[ast] = map[string]interface{}{"path": path, "input": string(data)}
	return ast, err
}
func d2rsInternalArray(t *testing.T, c *compiler, dst *Array, array *d2ast.Array, scope *d2ast.Map) {
	ctx := c.ctx.(*cancelAfterErrContext)
	initial := ctx.calls
	c.compileArray(dst, array, scope)
	d2rsInternalMu.Lock()
	defer d2rsInternalMu.Unlock()
	input, ok := d2rsInternalInputs[scope]
	if !ok {
		t.Fatal("uncaptured array source")
	}
	request := map[string]interface{}{"op": "array_cancel", "path": input["path"], "input": input["input"], "cancel_at": ctx.cancelAt, "initial_calls": initial}
	d2rsInternalWrite(t, request, map[string]interface{}{"canceled": errors.Is(c.contextErr, context.Canceled), "incomplete": len(dst.Values) < len(array.Nodes)}, map[string]interface{}{"calls": ctx.calls, "array": dst, "source_values": len(array.Nodes), "used": c.variableExpansion.used})
}
func d2rsInternalBase(t *testing.T, source string) *Map {
	root := compileIndexTestSource(t, source)
	d2rsInternalMu.Lock()
	defer d2rsInternalMu.Unlock()
	d2rsInternalBases[t.Name()] = source
	return root
}
func d2rsInternalKey(t *testing.T, text string) (*d2ast.Key, error) {
	key, err := d2parser.ParseMapKey(text)
	d2rsInternalMu.Lock()
	defer d2rsInternalMu.Unlock()
	d2rsInternalKeys[key] = text
	return key, err
}
func d2rsInternalRange(t *testing.T, start, end int) {
	d2rsInternalMu.Lock()
	defer d2rsInternalMu.Unlock()
	s := &d2rsInternalSession{request: map[string]interface{}{"op": "edge_cancel", "input": d2rsInternalBases[t.Name()], "cancel_start": start, "cancel_end": end}, observations: []interface{}{}}
	d2rsInternalEdges[t.Name()] = s
	t.Cleanup(func() {
		d2rsInternalMu.Lock()
		defer d2rsInternalMu.Unlock()
		d2rsInternalWrite(t, s.request, map[string]interface{}{"observed_during_expansion": s.observed}, s.observations)
	})
}
func d2rsInternalObserveEdge(t *testing.T, root *Map, refctx *RefContext, c *compiler, selector bool) {
	d2rsInternalMu.Lock()
	defer d2rsInternalMu.Unlock()
	s := d2rsInternalEdges[t.Name()]
	if s == nil {
		t.Fatal("uncaptured original cancellation range")
	}
	ctx := c.ctx.(*cancelAfterErrContext)
	s.request["key"] = d2rsInternalKeys[refctx.Key]
	s.request["selector"] = selector
	used := c.edgeExpansion.used
	canceled := errors.Is(c.contextErr, context.Canceled)
	s.observed = s.observed || (canceled && used > 0 && used < 9)
	s.observations = append(s.observations, map[string]interface{}{"cancel_at": ctx.cancelAt, "calls": ctx.calls, "canceled": canceled, "used": used, "ir": root})
}
func d2rsInternalCreate(t *testing.T, root *Map, id *EdgeID, refctx *RefContext, c *compiler) ([]*Edge, error) {
	edges, err := root.CreateEdge(id, refctx, c)
	d2rsInternalObserveEdge(t, root, refctx, c, false)
	return edges, err
}
func d2rsInternalSelect(t *testing.T, root *Map, id *EdgeID, refctx *RefContext, c *compiler) []*Edge {
	edges := root.getEdgesForCompile(id, refctx, c)
	d2rsInternalObserveEdge(t, root, refctx, c, true)
	return edges
}
func d2rsInternalNilReserve(t *testing.T, ctx context.Context, root *Map, copy bool) error {
	if root != nil {
		t.Fatal("unexpected non-nil reservation")
	}
	var err error
	if copy {
		err = ReserveVariableExpansionCopy(ctx, root)
	} else {
		err = ReserveVariableExpansionAliases(ctx, root)
	}
	d2rsInternalMu.Lock()
	defer d2rsInternalMu.Unlock()
	text := ""
	if err != nil {
		text = err.Error()
	}
	d2rsInternalWrite(t, map[string]interface{}{"op": "nil_reserve", "copy": copy}, map[string]interface{}{"error": text}, nil)
	return err
}
