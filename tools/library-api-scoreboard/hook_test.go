// SPDX-License-Identifier: MPL-2.0
// Observe original calls without replacing their assertions.
package d2lib

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/d2target"
)

var d2rsLibraryMu sync.Mutex
var d2rsLibraryCounts = map[string]int{}

func d2rsLibraryOptions(c *CompileOptions, r *d2svg.RenderOpts) any {
	if c == nil {
		c = &CompileOptions{}
	}
	if r == nil {
		r = &d2svg.RenderOpts{}
	}
	return map[string]any{"Layout": c.Layout, "FontFamily": c.FontFamily, "Render": r}
}
func d2rsLibrarySnapshot(v any) any {
	b, e := json.Marshal(v)
	if e != nil {
		panic(e)
	}
	var out any
	if e = json.Unmarshal(b, &out); e != nil {
		panic(e)
	}
	return out
}
func d2rsLibraryRecord(t *testing.T, request, got any) {
	d2rsLibraryMu.Lock()
	defer d2rsLibraryMu.Unlock()
	n := d2rsLibraryCounts[t.Name()]
	d2rsLibraryCounts[t.Name()] = n + 1
	name := fmt.Sprintf("%s#%d", t.Name(), n)
	b, e := json.Marshal(map[string]any{"name": name, "test": t.Name(), "request": request, "got": got})
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(filepath.Join(os.Getenv("D2RS_LIBRARY_DUMP"), fmt.Sprintf("%x.json", sha256.Sum256([]byte(name)))), b, 0600); e != nil {
		t.Fatal(e)
	}
}
func d2rsLibraryConfigs(t *testing.T, config *d2target.Config, c *CompileOptions, r *d2svg.RenderOpts) {
	req := d2rsLibrarySnapshot(map[string]any{"op": "configs", "config": config, "options": d2rsLibraryOptions(c, r), "has_compile_options": c != nil, "has_render_options": r != nil})
	applyConfigs(config, c, r)
	d2rsLibraryRecord(t, req, d2rsLibraryOptions(c, r))
}
func d2rsLibraryDefaults(t *testing.T, c *CompileOptions, r *d2svg.RenderOpts) {
	req := d2rsLibrarySnapshot(map[string]any{"op": "defaults", "options": d2rsLibraryOptions(c, r), "has_compile_options": c != nil, "has_render_options": r != nil})
	applyDefaults(c, r)
	d2rsLibraryRecord(t, req, d2rsLibraryOptions(c, r))
}
func d2rsLibraryCompile(t *testing.T, ctx context.Context, input string, c *CompileOptions, r *d2svg.RenderOpts) (*d2target.Diagram, *d2graph.Graph, error) {
	req := d2rsLibrarySnapshot(map[string]any{"op": "compile", "input": input, "options": d2rsLibraryOptions(c, r), "has_compile_options": c != nil, "has_render_options": r != nil})
	calls := 0
	if c != nil && c.LayoutResolver != nil {
		old := c.LayoutResolver
		c.LayoutResolver = func(name string) (d2graph.LayoutGraph, error) {
			f, e := old(name)
			if e != nil {
				return nil, e
			}
			return func(ctx context.Context, g *d2graph.Graph) error { calls++; return f(ctx, g) }, nil
		}
		defer func() { c.LayoutResolver = old }()
	}
	d, g, e := Compile(ctx, input, c, r)
	got := map[string]any{"error": "", "config": nil, "options": d2rsLibraryOptions(c, r), "layout_calls": calls}
	if e != nil {
		got["error"] = e.Error()
	}
	if d != nil {
		got["config"] = d.Config
	}
	if t.Name() == "TestDisabledIsometricRendersIdenticallyToDefault/default" || t.Name() == "TestDisabledIsometricRendersIdenticallyToDefault/explicit_false" || t.Name() == "TestDisabledIsometricRendersIdenticallyToDefault/source_false" || t.Name() == "TestDisabledIsometricRendersIdenticallyToDefault/disable_source_true" {
		if d != nil {
			out, err := d2svg.Render(d, r)
			if err != nil {
				t.Fatal(err)
			}
			got["svg"] = string(out)
		}
	}
	// Callback-dependent reuse tests remain explicit unsupported native operations.
	if t.Name() == "TestCompileLayoutReuseRequiresExplicitStableResolver" || t.Name() == "TestCompileLayoutReusePreservesPerBoardCancellation" || t.Name() == "TestCompileReusesStyleOnlyScenarioAndStepGeometry" || t.Name() == "TestCompileLayoutReuseRejectsGeometryChangesAndLayers" || t.Name() == "TestLayoutInputSignatureIgnoresOnlyGeometryNeutralStyles" {
		req.(map[string]any)["callback_contract"] = true
		req.(map[string]any)["layout_reuse"] = c.LayoutReuse
		if d != nil {
			got["diagram"] = d
		}
	}
	d2rsLibraryRecord(t, req, got)
	return d, g, e
}
