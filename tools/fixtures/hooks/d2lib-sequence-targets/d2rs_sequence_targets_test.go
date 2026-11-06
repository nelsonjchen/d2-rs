// SPDX-License-Identifier: MPL-2.0
// d2lib.Compile observations for the sequence target fixtures (recipe
// d2lib-sequence-targets), ported from the v0.9.0-era
// .reference/sequence-actor-label-policy-20261018/go-module/main.go. Each
// committed source compiles through the original d2lib.Compile with dagre and
// UTF-16 positions; the record keeps the exported target, the raw graph edge
// routes, the compile error, or a recovered panic. Inert unless run by
// scripts/regen-fixtures.py.
package d2lib_test

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"regexp"
	"runtime"
	"runtime/debug"
	"strings"
	"testing"
	"testing/fstest"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts/d2dagrelayout"
	"github.com/d2lang/d2/d2lib"
	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/lib/log"
	"github.com/d2lang/d2/lib/textmeasure"
)

// A case names its source inline, or as a path in the upstream tree
// (upstream_path, read at the pin; the record then repeats the source).
type d2rsSequenceCase struct {
	Name         string `json:"name"`
	Source       string `json:"source"`
	UpstreamPath string `json:"upstream_path"`
}

var (
	d2rsGoroutine = regexp.MustCompile(`goroutine \d+ \[`)
	d2rsHexWord   = regexp.MustCompile(`0x[0-9a-f]+\??`)
)

// d2rsCanonicalStack keeps a recovered panic's frames and line numbers but
// drops what changes between runs and hosts: goroutine numbers, argument and
// pointer words, and the absolute upstream-tree and GOROOT prefixes. Readers
// compare only panic presence and the panic value, never the stack.
func d2rsCanonicalStack(stack string) string {
	if wd, err := os.Getwd(); err == nil {
		stack = strings.ReplaceAll(stack, filepath.Dir(wd)+string(filepath.Separator), "$D2/")
	}
	stack = strings.ReplaceAll(stack, runtime.GOROOT()+string(filepath.Separator), "$GOROOT/")
	stack = d2rsGoroutine.ReplaceAllString(stack, "goroutine N [")
	return d2rsHexWord.ReplaceAllString(stack, "0x?")
}

func d2rsEvaluateSequence(source string) (out map[string]any) {
	defer func() {
		if v := recover(); v != nil {
			out = map[string]any{"panic": true, "error": nil, "panic_value": fmt.Sprint(v), "stack": d2rsCanonicalStack(string(debug.Stack()))}
		}
	}()
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		panic(err)
	}
	engine := "dagre"
	diagram, g, err := d2lib.Compile(log.WithDefault(context.Background()), source, &d2lib.CompileOptions{
		InputPath: "index.d2",
		UTF16Pos:  true,
		FS:        fstest.MapFS{},
		Ruler:     ruler,
		Layout:    &engine,
		LayoutResolver: func(string) (d2graph.LayoutGraph, error) {
			return d2dagrelayout.DefaultLayout, nil
		},
	}, &d2svg.RenderOpts{})
	if err != nil {
		return map[string]any{"error": err.Error(), "panic": false}
	}
	rawEdges := []any{}
	for _, edge := range g.Edges {
		rawEdges = append(rawEdges, map[string]any{"src": edge.Src.AbsID(), "dst": edge.Dst.AbsID(), "route": edge.Route})
	}
	return map[string]any{"error": nil, "panic": false, "diagram": diagram, "raw_connections": rawEdges}
}

func TestD2RSCaptureSequenceTargets(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	inputs := strings.Split(os.Getenv("D2RS_RECIPE_INPUTS"), string(filepath.ListSeparator))
	outputs := strings.Split(os.Getenv("D2RS_RECIPE_OUTPUTS"), string(filepath.ListSeparator))
	if len(inputs) != len(outputs) {
		t.Fatalf("%d inputs for %d outputs", len(inputs), len(outputs))
	}
	for i, input := range inputs {
		b, err := os.ReadFile(input)
		if err != nil {
			t.Fatal(err)
		}
		var cases []d2rsSequenceCase
		if err := json.Unmarshal(b, &cases); err != nil {
			t.Fatal(err)
		}
		rows := make([]any, 0, len(cases))
		for _, c := range cases {
			if c.UpstreamPath == "" {
				rows = append(rows, d2rsEvaluateSequence(c.Source))
				continue
			}
			wd, err := os.Getwd()
			if err != nil {
				t.Fatal(err)
			}
			source, err := os.ReadFile(filepath.Join(filepath.Dir(wd), filepath.FromSlash(c.UpstreamPath)))
			if err != nil {
				t.Fatal(err)
			}
			row := d2rsEvaluateSequence(string(source))
			row["source"] = string(source)
			rows = append(rows, row)
		}
		b, err = json.Marshal(rows)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(outputs[i], append(b, '\n'), 0o644); err != nil {
			t.Fatal(err)
		}
	}
}
