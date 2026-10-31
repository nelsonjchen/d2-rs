// SPDX-License-Identifier: MPL-2.0
// Overlay hook for d2lib (tools/fixtures/mathtext-latex-wasm.toml): a native
// mirror of the d2js d2wasm.Compile path (d2js/d2wasm/functions.go Compile and
// api.go wrapWASMCall), which only builds for js/wasm. For each generated
// case {name, source, ...} it compiles fs {"index": source} with layout dagre
// exactly as Compile configures d2lib, and records either the compiled
// diagram (expected_diagram) or the WASM error {message, code}, including the
// "panic recovered" message wrapWASMCall produces for a recovered panic.
package d2lib_test

import (
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"runtime/debug"
	"testing"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts/d2dagrelayout"
	"github.com/d2lang/d2/d2lib"
	"github.com/d2lang/d2/d2parser"
	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/lib/log"
	"github.com/d2lang/d2/lib/memfs"
	"github.com/d2lang/d2/lib/textmeasure"
	"github.com/d2lang/util-go/go2"
)

type d2rsWASMError struct {
	Message string `json:"message"`
	Code    int    `json:"code"`
}

func d2rsCompile(source string) (diagram json.RawMessage, failure *d2rsWASMError) {
	defer func() {
		if r := recover(); r != nil {
			diagram, failure = nil, &d2rsWASMError{fmt.Sprintf("panic recovered: %v\n%s", r, debug.Stack()), 500}
		}
	}()
	const inputPath = "index"
	files := map[string]string{inputPath: source}
	compileOpts := &d2lib.CompileOptions{UTF16Pos: true, LayoutReuse: true, InputPath: inputPath}
	compileOpts.LayoutResolver = func(engine string) (d2graph.LayoutGraph, error) {
		if engine != "dagre" {
			return nil, fmt.Errorf("layout option '%s' not recognized", engine)
		}
		return d2dagrelayout.DefaultLayout, nil
	}
	compileOpts.RouterResolver = func(string) (d2graph.RouteEdges, error) { return nil, nil }
	var err error
	if compileOpts.FS, err = memfs.New(files); err != nil {
		return nil, &d2rsWASMError{"invalid fs input: " + err.Error(), 400}
	}
	if compileOpts.Ruler, err = textmeasure.NewRuler(); err != nil {
		return nil, &d2rsWASMError{"text ruler cannot be initialized: " + err.Error(), 500}
	}
	compileOpts.Layout = go2.Pointer("dagre")
	ctx := log.WithDefault(context.Background())
	compiled, _, err := d2lib.Compile(ctx, source, compileOpts, &d2svg.RenderOpts{})
	if err != nil {
		if pe, ok := err.(*d2parser.ParseError); ok {
			errs, _ := json.Marshal(pe.Errors)
			return nil, &d2rsWASMError{string(errs), 400}
		}
		return nil, &d2rsWASMError{err.Error(), 500}
	}
	raw, err := json.Marshal(*compiled)
	if err != nil {
		panic(err)
	}
	return raw, nil
}

func TestD2RSCaptureNilChild(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var cases []json.RawMessage
	if err := json.Unmarshal(data, &cases); err != nil {
		t.Fatal(err)
	}
	manifest, err := os.ReadFile(filepath.Join(os.Getenv("D2RS_REPO_ROOT"), "reference-manifest.json"))
	if err != nil {
		t.Fatal(err)
	}
	var pin struct {
		D2 struct {
			Commit string `json:"commit"`
		} `json:"d2"`
	}
	if err := json.Unmarshal(manifest, &pin); err != nil {
		t.Fatal(err)
	}
	var body bytes.Buffer
	faults, successes := 0, 0
	for i, raw := range cases {
		var request struct {
			Source string `json:"source"`
		}
		if err := json.Unmarshal(raw, &request); err != nil {
			t.Fatal(err)
		}
		diagram, failure := d2rsCompile(request.Source)
		errorJSON, err := json.Marshal(failure)
		if err != nil {
			t.Fatal(err)
		}
		if failure != nil {
			faults++
			diagram = json.RawMessage("null")
		} else {
			successes++
		}
		// Keep the request's own field order, then append the outcome.
		trimmed := bytes.TrimRight(bytes.TrimSpace(raw), "}")
		if i > 0 {
			body.WriteString(",")
		}
		fmt.Fprintf(&body, `%s,"source_error":%s,"expected_diagram":%s}`, trimmed, errorJSON, diagram)
	}
	header, err := json.Marshal(struct {
		SourceCommit string `json:"source_commit"`
		Scope        string `json:"scope"`
	}{pin.D2.Commit, fmt.Sprintf("%d native d2wasm.Compile mirrors: %d faults and %d complete successful diagrams (dagre).", len(cases), faults, successes)})
	if err != nil {
		t.Fatal(err)
	}
	doc := append(bytes.TrimRight(header, "}"), []byte(`,"cases":[`+body.String()+`]}`)...)
	var pretty bytes.Buffer
	if err := json.Indent(&pretty, doc, "", "  "); err != nil {
		t.Fatal(err)
	}
	pretty.WriteByte('\n')
	if err := os.WriteFile(out, pretty.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}
