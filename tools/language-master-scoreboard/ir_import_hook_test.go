// SPDX-License-Identifier: MPL-2.0
// Observers for original public import calls; original assertions stay intact.
package d2ir_test

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
	"github.com/d2lang/d2/d2ir"
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

func d2rsIRCompile(t *testing.T, ast *d2ast.Map, opts *d2ir.CompileOptions) (*d2ir.Map, []string, error) {
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return d2ir.Compile(ast, opts)
	}
	if opts == nil {
		opts = &d2ir.CompileOptions{}
	}
	policy := "background"
	fileSystem := opts.FS
	// Unwrap only the original cancel-on-open fixture. Snapshotting the backing
	// MapFS cannot advance cancellation or consume the original Open operation.
	if wrapped, ok := fileSystem.(cancelOnOpenFS); ok {
		fileSystem = wrapped.FS
		policy = "cancel_on_open"
	} else if opts.Context != nil && opts.Context != context.Background() {
		t.Fatalf("unsupported original context %T", opts.Context)
	}
	files := map[string]string{}
	if fileSystem != nil {
		if err := fs.WalkDir(fileSystem, ".", func(name string, entry fs.DirEntry, err error) error {
			if err != nil {
				return err
			}
			if entry.IsDir() {
				return nil
			}
			data, err := fs.ReadFile(fileSystem, name)
			if err != nil {
				return err
			}
			files[name] = string(data)
			return nil
		}); err != nil {
			t.Fatal(err)
		}
	}
	root, imports, err := d2ir.Compile(ast, opts)
	errText, errKind := "", ""
	if err != nil {
		errText = err.Error()
		if err == context.Canceled {
			errKind = "canceled"
		}
	}
	d2rsIRMu.Lock()
	defer d2rsIRMu.Unlock()
	original, ok := d2rsIRInputs[ast]
	if !ok {
		t.Fatal("missing original IR parse input")
	}
	request := map[string]interface{}{"path": original["path"], "input": original["input"], "has_fs": opts.FS != nil,
		"has_context": opts.Context != nil, "context_policy": map[string]interface{}{"kind": policy}, "files": files, "typed_error": true}
	call := d2rsIRCalls[t.Name()]
	d2rsIRCalls[t.Name()]++
	name := fmt.Sprintf("%s#%d", t.Name(), call)
	data, marshalErr := json.Marshal(map[string]interface{}{"name": name, "test": t.Name(), "request": request,
		"got": map[string]interface{}{"error": errText, "error_kind": errKind, "ir": root, "imports": imports}})
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"), id+".json"), data, 0644); err != nil {
		t.Fatal(err)
	}
	return root, imports, err
}
