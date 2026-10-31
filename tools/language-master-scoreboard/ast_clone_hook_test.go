// SPDX-License-Identifier: MPL-2.0
// Observe original master cached-AST assertions without changing their bodies.
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

var d2rsCloneMu sync.Mutex
var d2rsCloneInputs = map[*d2ast.Map]map[string]interface{}{}
var d2rsCloneCalls = map[string]int{}

func d2rsCloneParse(t *testing.T, path string, reader io.Reader, opts *d2parser.ParseOptions) (*d2ast.Map, error) {
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return d2parser.Parse(path, reader, opts)
	}
	input, err := io.ReadAll(reader)
	if err != nil {
		t.Fatal(err)
	}
	ast, err := d2parser.Parse(path, bytes.NewReader(input), opts)
	d2rsCloneMu.Lock()
	d2rsCloneInputs[ast] = map[string]interface{}{"path": path, "input": string(input)}
	d2rsCloneMu.Unlock()
	return ast, err
}

func d2rsCloneASTMapContext(t *testing.T, ctx context.Context, ast *d2ast.Map) (*d2ast.Map, error) {
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return cloneASTMapContext(ctx, ast)
	}
	originalContext, ok := ctx.(*cancelAfterErrContext)
	if !ok {
		t.Fatal("unrecognized original clone context")
	}
	// Do not call Err here: the original operation must consume the context.
	policy := map[string]interface{}{"cancel_at": originalContext.cancelAt, "initial_calls": originalContext.calls}
	cloned, err := cloneASTMapContext(ctx, ast)
	errorText, errorKind := "", ""
	if err != nil {
		errorText = err.Error()
	}
	if errors.Is(err, context.Canceled) {
		errorKind = "canceled"
	}
	d2rsCloneMu.Lock()
	defer d2rsCloneMu.Unlock()
	input, ok := d2rsCloneInputs[ast]
	if !ok {
		t.Fatal("missing original AST clone input")
	}
	request := map[string]interface{}{"path": input["path"], "input": input["input"], "context_policy": policy}
	call := d2rsCloneCalls[t.Name()]
	d2rsCloneCalls[t.Name()]++
	name := fmt.Sprintf("%s#%d", t.Name(), call)
	data, marshalErr := json.Marshal(map[string]interface{}{"name": name, "test": t.Name(), "request": request,
		"got": map[string]interface{}{"map": cloned, "error": errorText, "error_kind": errorKind}})
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"), id+".json"), data, 0644); err != nil {
		t.Fatal(err)
	}
	return cloned, err
}
