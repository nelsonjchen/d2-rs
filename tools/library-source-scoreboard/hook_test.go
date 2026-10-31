// SPDX-License-Identifier: MPL-2.0
// Observe original library calls; retain their return values and assertions.
package d2lib

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
	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/d2target"
)

var d2rsSourceMu sync.Mutex
var d2rsSourceCounts = map[string]int{}

func d2rsSourceRequest(op string, ctx context.Context, input string, opts *CompileOptions) map[string]any {
	if opts == nil {
		opts = &CompileOptions{}
	}
	return map[string]any{"op": op, "input": input, "path": opts.InputPath,
		"utf16": opts.UTF16Pos, "canceled": errors.Is(ctx.Err(), context.Canceled),
		"max_variable_expansion": opts.MaxVariableExpansion, "max_glob_expansion": opts.MaxGlobExpansion,
		"max_edge_expansion": opts.MaxEdgeExpansion, "max_edge_expansion_work": opts.MaxEdgeExpansionWork,
		"has_fs": opts.FS != nil,
		// The previous adapter can classify these new operations as unsupported.
		"options": map[string]any{"Layout": nil, "FontFamily": nil, "Render": &d2svg.RenderOpts{}}}
}

func d2rsSourceRecord(t *testing.T, request, got any) {
	d2rsSourceMu.Lock()
	defer d2rsSourceMu.Unlock()
	n := d2rsSourceCounts[t.Name()]
	d2rsSourceCounts[t.Name()] = n + 1
	name := fmt.Sprintf("%s#%d", t.Name(), n)
	b, err := json.Marshal(map[string]any{"name": name, "test": t.Name(), "request": request, "got": got})
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(filepath.Join(os.Getenv("D2RS_LIBRARY_DUMP"), fmt.Sprintf("%x.json", sha256.Sum256([]byte(name)))), b, 0600); err != nil {
		t.Fatal(err)
	}
}

func d2rsSourceParse(t *testing.T, ctx context.Context, input string, opts *CompileOptions) (*d2ast.Map, error) {
	if os.Getenv("D2RS_LIBRARY_DUMP") == "" {
		return Parse(ctx, input, opts)
	}
	request := d2rsSourceRequest("source-parse", ctx, input, opts)
	a, err := Parse(ctx, input, opts)
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsSourceRecord(t, request, map[string]any{"error": message, "canceled": errors.Is(err, context.Canceled), "ast_nil": a == nil, "ast": a})
	return a, err
}

func d2rsSourceCompile(t *testing.T, ctx context.Context, input string, opts *CompileOptions, render *d2svg.RenderOpts) (*d2target.Diagram, *d2graph.Graph, error) {
	if os.Getenv("D2RS_LIBRARY_DUMP") == "" {
		return Compile(ctx, input, opts, render)
	}
	request := d2rsSourceRequest("source-compile", ctx, input, opts)
	d, g, err := Compile(ctx, input, opts, render)
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsSourceRecord(t, request, map[string]any{"error": message, "canceled": errors.Is(err, context.Canceled), "diagram_nil": d == nil, "graph_nil": g == nil})
	return d, g, err
}
