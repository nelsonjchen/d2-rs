// SPDX-License-Identifier: MPL-2.0
// Env-gated wrappers for original d2ir glob/variable expansion assertions.
package d2ir

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
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

var d2rsIRMu sync.Mutex
var d2rsIRInputs = map[*d2ast.Map]map[string]interface{}{}
var d2rsIRCalls = map[string]int{}

func d2rsIRParse(t *testing.T, path string, reader io.Reader, opts *d2parser.ParseOptions) (*d2ast.Map, error) {
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return d2parser.Parse(path, reader, opts)
	}
	input, err := io.ReadAll(reader)
	if err != nil {
		t.Fatal(err)
	}
	ast, err := d2parser.Parse(path, bytes.NewReader(input), opts)
	d2rsIRMu.Lock()
	d2rsIRInputs[ast] = map[string]interface{}{"path": path, "input": string(input)}
	d2rsIRMu.Unlock()
	return ast, err
}
func d2rsIRCompile(t *testing.T, ast *d2ast.Map, opts *CompileOptions) (*Map, []string, error) {
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return Compile(ast, opts)
	}
	if opts == nil {
		opts = &CompileOptions{}
	}
	// Describe the original context without calling Err (which can advance it).
	var policy interface{}
	if opts.Context == nil || opts.Context == context.Background() {
		policy = map[string]interface{}{"kind": "background"}
	} else if ctx, ok := opts.Context.(*cancelAfterErrContext); ok {
		policy = map[string]interface{}{"kind": "cancel_after_checks", "cancel_at": ctx.cancelAt, "initial_calls": ctx.calls}
	}
	root, imports, err := Compile(ast, opts)
	errText := ""
	if err != nil {
		errText = err.Error()
	}
	d2rsIRMu.Lock()
	defer d2rsIRMu.Unlock()
	original, ok := d2rsIRInputs[ast]
	if !ok {
		t.Fatal("missing original IR parse input")
	}
	files := map[string]string{}
	if opts.FS != nil {
		if err := fs.WalkDir(opts.FS, ".", func(name string, entry fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if entry.IsDir() {
				return nil
			}
			data, err := fs.ReadFile(opts.FS, name)
			if err != nil {
				return err
			}
			files[name] = string(data)
			return nil
		}); err != nil {
			t.Fatal(err)
		}
	}
	request := map[string]interface{}{"path": original["path"], "input": original["input"], "max_glob_expansion": opts.MaxGlobExpansion,
		"max_variable_expansion": opts.MaxVariableExpansion, "max_edge_expansion": opts.MaxEdgeExpansion,
		"max_edge_expansion_work": opts.MaxEdgeExpansionWork, "has_context": opts.Context != nil,
		"context_policy": policy, "has_fs": opts.FS != nil, "files": files}
	call := d2rsIRCalls[t.Name()]
	d2rsIRCalls[t.Name()]++
	name := fmt.Sprintf("%s#%d", t.Name(), call)
	data, marshalErr := json.Marshal(map[string]interface{}{"name": name, "test": t.Name(), "request": request,
		"got": map[string]interface{}{"error": errText, "ir": root, "imports": imports}})
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"), id+".json"), data, 0644); err != nil {
		t.Fatal(err)
	}
	return root, imports, err
}
