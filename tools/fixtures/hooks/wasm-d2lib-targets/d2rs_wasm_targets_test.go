// SPDX-License-Identifier: MPL-2.0
// d2lib.Compile observations for d2-rs-wasm (recipe wasm-d2lib-targets),
// ported from the v0.9.0-era Go modules in
// .reference/compiled-edge-index-native-20261009/oracle-go and
// .reference/wasm-sketch-precedence-native-20261009/go-module:
//   - compiled edge identities (Edge.AbsID/Index/arrows) with the exported
//     target connections, for each committed source;
//   - per-board object/label measurements under every source/caller sketch
//     combination (generated here, as the original did);
//   - root SVG attributes and rough shape paths of d2svg.Render for the
//     committed render-option sources.
// Inert unless run by scripts/regen-fixtures.py.
package d2lib_test

import (
	"bytes"
	"context"
	"encoding/json"
	"encoding/xml"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2compiler"
	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts/d2dagrelayout"
	"github.com/d2lang/d2/d2lib"
	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/log"
	"github.com/d2lang/d2/lib/textmeasure"
)

func d2rsWasmCommit(t *testing.T) string {
	b, err := os.ReadFile(filepath.Join(os.Getenv("D2RS_REPO_ROOT"), "reference-manifest.json"))
	if err != nil {
		t.Fatal(err)
	}
	var manifest struct {
		D2 struct {
			Commit string `json:"commit"`
		} `json:"d2"`
	}
	if err := json.Unmarshal(b, &manifest); err != nil {
		t.Fatal(err)
	}
	return manifest.D2.Commit
}

func d2rsWasmCompile(source string, opts *d2svg.RenderOpts) (*d2target.Diagram, *d2graph.Graph, error) {
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		return nil, nil, err
	}
	return d2lib.Compile(log.WithDefault(context.Background()), source, &d2lib.CompileOptions{
		Ruler: ruler,
		LayoutResolver: func(engine string) (d2graph.LayoutGraph, error) {
			if engine != "dagre" {
				return nil, fmt.Errorf("unexpected engine %q", engine)
			}
			return d2dagrelayout.DefaultLayout, nil
		},
	}, opts)
}

func d2rsWasmWrite(t *testing.T, path string, value any) {
	var b bytes.Buffer
	encoder := json.NewEncoder(&b)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(value); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, b.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

func d2rsWasmRead(t *testing.T, path string, into any) {
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(b, into); err != nil {
		t.Fatal(err)
	}
}

func d2rsEdgeIdentity(t *testing.T, input string) []any {
	var inputs []map[string]any
	d2rsWasmRead(t, input, &inputs)
	rows := []any{}
	for _, in := range inputs {
		row := map[string]any{"input": in}
		func() {
			defer func() {
				if p := recover(); p != nil {
					row["panic"] = fmt.Sprint(p)
				}
			}()
			// Edge identities come from the compiled graph, as the original
			// d2compiler.Compile probe recorded them (layout adds no edges
			// there); connections are the exported target's.
			g, _, err := d2compiler.Compile("-", strings.NewReader(in["source"].(string)), nil)
			if err != nil {
				row["error"] = err.Error()
				return
			}
			diagram, _, err := d2rsWasmCompile(in["source"].(string), &d2svg.RenderOpts{})
			if err != nil {
				row["error"] = err.Error()
				return
			}
			edges := []any{}
			for _, e := range g.Edges {
				edges = append(edges, map[string]any{"src": e.Src.AbsID(), "dst": e.Dst.AbsID(), "src_arrow": e.SrcArrow, "dst_arrow": e.DstArrow, "index": e.Index, "abs_id": e.AbsID()})
			}
			row["error"] = ""
			row["edges"] = edges
			row["connections"] = diagram.Connections
		}()
		rows = append(rows, row)
	}
	return rows
}

func d2rsSketchPrecedence(t *testing.T) []any {
	body := "regular: Frame sampling jitter\nbold_text: {label: Frame sampling jitter; style.bold: true}\nitalic_text: {label: Frame sampling jitter; style.italic: true}\nmono: {label: Frame sampling jitter; style.font: mono}\nregular -> bold_text: sample interval\n"
	cases := []any{}
	for _, config := range []string{"absent", "false", "true"} {
		for _, caller := range []string{"absent", "false", "true"} {
			input := body + "layers: {alternate: {" + body + "}}\nscenarios: {recolor: {regular.style.fill: red}}\nsteps: {fade: {regular.style.opacity: 0.5}}\n"
			if config != "absent" {
				input = "vars: {d2-config: {sketch: " + config + "}}\n" + input
			}
			opts := &d2svg.RenderOpts{}
			if caller != "absent" {
				value := caller == "true"
				opts.Sketch = &value
			}
			_, graph, err := d2rsWasmCompile(input, opts)
			if err != nil {
				t.Fatal(err)
			}
			for i, board := range []*d2graph.Graph{graph, graph.Layers[0], graph.Scenarios[0], graph.Steps[0]} {
				target := []string{"", "layers.alternate", "scenarios.recolor", "steps.fade"}[i]
				options := map[string]any{"engine": "dagre", "target": target}
				if caller != "absent" {
					options["sketch"] = caller == "true"
				}
				nodes := []any{}
				for _, obj := range board.Objects {
					nodes = append(nodes, map[string]any{"id": obj.AbsID(), "width": obj.Width, "height": obj.Height, "label_width": obj.LabelDimensions.Width, "label_height": obj.LabelDimensions.Height})
				}
				cases = append(cases, map[string]any{"name": config + "/" + caller + "/" + target, "source": input, "options": options, "effective_sketch": *opts.Sketch, "nodes": nodes})
			}
		}
	}
	return cases
}

// d2rsSVGProjection returns the root <svg> width/height/viewBox/
// preserveAspectRatio attributes that are present, and the d of every <path>
// whose class list contains "shape", in document order.
func d2rsSVGProjection(t *testing.T, svg []byte) (map[string]string, []string) {
	decoder := xml.NewDecoder(bytes.NewReader(svg))
	var root map[string]string
	paths := []string{}
	for {
		token, err := decoder.Token()
		if err == io.EOF {
			break
		}
		if err != nil {
			t.Fatal(err)
		}
		start, ok := token.(xml.StartElement)
		if !ok {
			continue
		}
		if root == nil {
			root = map[string]string{}
			for _, a := range start.Attr {
				switch a.Name.Local {
				case "width", "height", "viewBox", "preserveAspectRatio":
					root[a.Name.Local] = a.Value
				}
			}
			continue
		}
		if start.Name.Local != "path" {
			continue
		}
		var class, d string
		for _, a := range start.Attr {
			switch a.Name.Local {
			case "class":
				class = a.Value
			case "d":
				d = a.Value
			}
		}
		for _, c := range strings.Fields(class) {
			if c == "shape" {
				paths = append(paths, d)
				break
			}
		}
	}
	return root, paths
}

func d2rsRenderOptions(t *testing.T, input string) []any {
	var inputs []struct {
		Name    string `json:"name"`
		Source  string `json:"source"`
		Options struct {
			Sketch *bool    `json:"sketch,omitempty"`
			Pad    *int64   `json:"pad,omitempty"`
			Scale  *float64 `json:"scale,omitempty"`
		} `json:"options"`
	}
	d2rsWasmRead(t, input, &inputs)
	cases := []any{}
	for _, item := range inputs {
		opts := &d2svg.RenderOpts{Sketch: item.Options.Sketch, Pad: item.Options.Pad, Scale: item.Options.Scale}
		diagram, _, err := d2rsWasmCompile(item.Source, opts)
		if err != nil {
			t.Fatal(err)
		}
		svg, err := d2svg.Render(diagram, opts)
		if err != nil {
			t.Fatal(err)
		}
		attributes, paths := d2rsSVGProjection(t, svg)
		cases = append(cases, map[string]any{"name": item.Name, "source": item.Source, "options": item.Options, "svg_attributes": attributes, "shape_paths": paths})
	}
	return cases
}

func TestD2RSCaptureWasmTargets(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	inputs := strings.Split(os.Getenv("D2RS_RECIPE_INPUTS"), string(filepath.ListSeparator))
	outputs := strings.Split(os.Getenv("D2RS_RECIPE_OUTPUTS"), string(filepath.ListSeparator))
	if len(inputs) != 2 || len(outputs) != 3 {
		t.Fatalf("want 2 inputs and 3 outputs, got %d and %d", len(inputs), len(outputs))
	}
	commit := d2rsWasmCommit(t)
	d2rsWasmWrite(t, outputs[0], map[string]any{"source_commit": commit, "oracle": "d2lib.Compile (dagre); d2graph.Edge.Index/AbsID and exported target connections", "cases": d2rsEdgeIdentity(t, inputs[0])})
	d2rsWasmWrite(t, outputs[1], map[string]any{"source_commit": commit, "cases": d2rsSketchPrecedence(t)})
	d2rsWasmWrite(t, outputs[2], map[string]any{"source_commit": commit, "cases": d2rsRenderOptions(t, inputs[1])})
}
