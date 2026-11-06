// SPDX-License-Identifier: MPL-2.0
// Env-gated observers; original parser assertions and returned values are intact.
package d2parser_test

import (
	"bytes"
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/d2ast"
	"github.com/d2lang/d2/d2parser"
)

var d2rsParserMu sync.Mutex
var d2rsParserCalls = map[string]int{}

func d2rsMasterParse(t *testing.T, path string, reader io.Reader, opts *d2parser.ParseOptions) (*d2ast.Map, error) {
	return d2rsMasterParseContext(t, context.Background(), path, reader, opts)
}

func d2rsMasterParseContext(t *testing.T, ctx context.Context, path string, reader io.Reader, opts *d2parser.ParseOptions) (*d2ast.Map, error) {
	if os.Getenv("D2RS_MASTER_LANGUAGE_DUMP") == "" {
		return d2parser.ParseContext(ctx, path, reader, opts)
	}
	input, readErr := io.ReadAll(reader)
	if readErr != nil {
		t.Fatal(readErr)
	}
	// These original tests use only Background or a pre-canceled cancelCtx.
	policy := "background"
	if ctx.Err() == context.Canceled {
		policy = "canceled"
	}
	ast, err := d2parser.ParseContext(ctx, path, bytes.NewReader(input), opts)
	errText, kind := "", ""
	var structured *d2parser.ParseError
	if err != nil {
		errText = err.Error()
		if err == context.Canceled {
			kind = "canceled"
		} else {
			kind = "parse"
			var ok bool
			structured, ok = err.(*d2parser.ParseError)
			if !ok {
				t.Fatalf("unsupported parser error type %T", err)
			}
		}
	}
	d2rsParserMu.Lock()
	defer d2rsParserMu.Unlock()
	call := d2rsParserCalls[t.Name()]
	d2rsParserCalls[t.Name()]++
	name := fmt.Sprintf("%s#%d", t.Name(), call)
	data, marshalErr := json.Marshal(map[string]interface{}{
		"name": name, "test": t.Name(),
		"request": map[string]interface{}{"path": path, "input": string(input), "context_policy": policy, "utf16": opts != nil && opts.UTF16Pos},
		"got":     map[string]interface{}{"map": ast, "error": errText, "error_kind": kind, "structured": structured},
	})
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MASTER_LANGUAGE_DUMP"), id+".json"), data, 0644); err != nil {
		t.Fatal(err)
	}
	return ast, err
}
