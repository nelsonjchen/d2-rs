// SPDX-License-Identifier: MPL-2.0
// Oracle hook for the d2lib-compiled crates/d2-rs-render fixtures
// (docs/FIXTURES.md; generator scripts/render_probe_fixtures.py). For each
// case of the D2RS_RECIPE_INPUT job list it records the d2lib.Compile target
// (TALA, seed 1, one worker, or the case's layout), and on request the
// d2svg.Render SVG, graph predicates (sequence groups/notes/fills/children),
// compiler references, the Markdown scene background/clip, or the root canvas
// scene backgrounds and viewBox. Overlaid into d2lib as an external test
// package; inert unless D2RS_RECIPE_INPUT is set. Harvested from the v0.9.0-era
// ../d2-v0.9.0-trace/{markdown_background,root_canvas_style,
// sequence_reference_groups,sequence_note_references}_probe.go overlays.
package d2lib_test

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"testing"
	"testing/fstest"

	"github.com/d2lang/d2/d2compiler"
	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts/d2dagrelayout"
	"github.com/d2lang/d2/d2layouts/d2elklayout"
	"github.com/d2lang/d2/d2layouts/d2talalayout"
	"github.com/d2lang/d2/d2lib"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/log"
	"github.com/d2lang/d2/lib/textmeasure"
)

type d2rsProbeCase struct {
	Name      string            `json:"name"`
	Source    string            `json:"source"`
	Layout    string            `json:"layout"`
	InputPath string            `json:"inputPath"`
	Files     map[string]string `json:"files"`
	// Theme is passed to Compile's RenderOpts and to the SVG/scene builders.
	Theme *int64 `json:"theme"`
	// RenderPad is the d2svg.Render pad (nil: renderer default).
	RenderPad *int64 `json:"renderPad"`
	// Layer selects d.Layers[0] before the root-canvas scene.
	Layer  bool     `json:"layer"`
	Extras []string `json:"extras"`
}

func d2rsProbeResolver(name string) func(string) (d2graph.LayoutGraph, error) {
	return func(string) (d2graph.LayoutGraph, error) {
		switch name {
		case "dagre":
			return func(ctx context.Context, g *d2graph.Graph) error { return d2dagrelayout.Layout(ctx, g, nil) }, nil
		case "elk":
			return func(ctx context.Context, g *d2graph.Graph) error { return d2elklayout.Layout(ctx, g, nil) }, nil
		case "elk-spore":
			// ELK with the SPoRE overlap-removal algorithm (elk-spore probe).
			return func(ctx context.Context, g *d2graph.Graph) error {
				opts := d2elklayout.DefaultOpts
				opts.Algorithm = "sporeOverlap"
				return d2elklayout.Layout(ctx, g, &opts)
			}, nil
		}
		return func(ctx context.Context, g *d2graph.Graph) error {
			return d2talalayout.Layout(ctx, g, &d2talalayout.Options{Seeds: []int64{1}, MaxConcurrency: 1})
		}, nil
	}
}

func d2rsPredicates(g *d2graph.Graph) []map[string]any {
	rows := []map[string]any{}
	for _, o := range g.Objects {
		children := []map[string]any{}
		for _, c := range o.ChildrenArray {
			children = append(children, map[string]any{"id": c.AbsID(), "containsEdge": c.ContainsAnyEdge(g.Edges)})
		}
		scope := ""
		if sd := o.OuterSequenceDiagram(); sd != nil {
			scope = sd.AbsID()
		}
		rows = append(rows, map[string]any{"id": o.AbsID(), "group": o.IsSequenceDiagramGroup(), "note": o.IsSequenceDiagramNote(), "fill": o.GetFill(), "outerSequence": scope, "containsEdge": o.ContainsAnyEdge(g.Edges), "children": children})
	}
	return rows
}

func d2rsReferences(t *testing.T, c d2rsProbeCase) map[string]any {
	g, _, err := d2compiler.Compile(c.Name, bytes.NewReader([]byte(c.Source)), nil)
	if err != nil {
		return map[string]any{"error": err.Error()}
	}
	objects := []map[string]any{}
	for _, o := range g.Objects {
		hasEdgeRef := false
		refs := []map[string]any{}
		for _, r := range o.References {
			edgeCount := 0
			if r.MapKey != nil {
				edgeCount = len(r.MapKey.Edges)
			}
			if edgeCount > 0 {
				hasEdgeRef = true
			}
			scope := ""
			if r.ScopeObj != nil {
				scope = r.ScopeObj.AbsID()
			}
			refs = append(refs, map[string]any{"edgeCount": edgeCount, "scope": scope})
		}
		objects = append(objects, map[string]any{"id": o.AbsID(), "hasEdgeRef": hasEdgeRef, "note": o.IsSequenceDiagramNote(), "group": o.IsSequenceDiagramGroup(), "containsObjects": o.ContainsAnyObject(g.Objects), "containsEdges": o.ContainsAnyEdge(g.Edges), "references": refs})
	}
	edges := []map[string]any{}
	for _, e := range g.Edges {
		scopes := []string{}
		for _, r := range e.References {
			scopes = append(scopes, r.ScopeObj.AbsID())
		}
		edges = append(edges, map[string]any{"source": e.Src.AbsID(), "target": e.Dst.AbsID(), "scopes": scopes})
	}
	return map[string]any{"objects": objects, "edges": edges}
}

// Appendix target bounds (appendix_bounds_controls overlay): each input row
// gains the d2target.BoundingBox of its raw target and the appendix max width.
func TestD2RSAppendixBounds(t *testing.T) {
	input := os.Getenv("D2RS_RECIPE_INPUT")
	output := os.Getenv("D2RS_RECIPE_OUTPUT")
	if input == "" || output == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(input)
	if err != nil {
		t.Fatal(err)
	}
	var rows []map[string]json.RawMessage
	if err := json.Unmarshal(data, &rows); err != nil {
		t.Fatal(err)
	}
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	dims := d2graph.GetTextDimensions(nil, ruler, &d2target.MText{Text: "details", FontSize: 16}, nil)
	for _, row := range rows {
		var target d2target.Diagram
		if err := json.Unmarshal(row["target"], &target); err != nil {
			t.Fatal(err)
		}
		tl, br := target.BoundingBox()
		row["bounds"], err = json.Marshal(map[string]any{"left": tl.X, "top": tl.Y, "right": br.X, "bottom": br.Y, "appendix_max_width": dims.Width + 48})
		if err != nil {
			t.Fatal(err)
		}
	}
	out, err := json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(output, append(out, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestD2RSCompileProbe(t *testing.T) {
	input := os.Getenv("D2RS_RECIPE_INPUT")
	output := os.Getenv("D2RS_RECIPE_OUTPUT")
	if input == "" || output == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(input)
	if err != nil {
		t.Fatal(err)
	}
	var cases []d2rsProbeCase
	if err := json.Unmarshal(data, &cases); err != nil {
		t.Fatal(err)
	}
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	ctx := log.WithDefault(context.Background())
	rows := []map[string]any{}
	for _, c := range cases {
		row := map[string]any{"name": c.Name, "source": c.Source}
		extras := map[string]bool{}
		for _, e := range c.Extras {
			extras[e] = true
		}
		if extras["references"] {
			row["references"] = d2rsReferences(t, c)
		}
		if extras["compile"] {
			layout := c.Layout
			if layout == "" {
				layout = "tala"
			}
			opts := &d2lib.CompileOptions{Layout: &layout, Ruler: ruler, LayoutResolver: d2rsProbeResolver(layout), InputPath: c.InputPath}
			if c.Files != nil {
				fsys := fstest.MapFS{}
				for name, text := range c.Files {
					fsys[name] = &fstest.MapFile{Data: []byte(text)}
				}
				opts.FS = fsys
			}
			var renderOpts *d2svg.RenderOpts
			if c.Theme != nil {
				renderOpts = &d2svg.RenderOpts{ThemeID: c.Theme}
			}
			d, g, err := d2lib.Compile(ctx, c.Source, opts, renderOpts)
			if err != nil {
				row["compile_error"] = err.Error()
				rows = append(rows, row)
				continue
			}
			row["diagram"] = d
			if extras["bytes"] {
				data, err := d.Bytes()
				if err != nil {
					t.Fatal(err)
				}
				row["bytes"] = data
			}
			if extras["raw_routes"] {
				routes := []map[string]any{}
				for _, e := range g.Edges {
					routes = append(routes, map[string]any{"id": e.AbsID(), "src": e.Src.AbsID(), "dst": e.Dst.AbsID(), "route": e.Route})
				}
				row["raw_routes"] = routes
			}
			if extras["predicates"] {
				row["predicates"] = d2rsPredicates(g)
			}
			if extras["svg"] {
				svg, err := d2svg.Render(d, &d2svg.RenderOpts{Pad: c.RenderPad, ThemeID: c.Theme})
				if err != nil {
					row["svg_error"] = err.Error()
				} else {
					row["svg"] = string(svg)
				}
			}
			if extras["markdown"] {
				scene, err := d2scenebuild.Build(ctx, d, d2scenebuild.Options{ThemeID: c.Theme})
				if err != nil {
					t.Fatal(err)
				}
				var md *d2scene.Node
				var visit func(*d2scene.Node)
				visit = func(n *d2scene.Node) {
					if n == nil {
						return
					}
					if n.ID == "a:markdown" {
						md = n
					}
					for _, child := range n.Children {
						visit(child)
					}
				}
				visit(scene.Root)
				if md == nil {
					t.Fatalf("%s: missing a:markdown", c.Name)
				}
				var bg *d2scene.Node
				for _, n := range md.Children {
					if n.ID == "a:markdown:background" {
						bg = n
					}
				}
				row["shape"] = d.Shapes[0]
				row["markdown_transform"] = md.Transform
				row["clip"] = md.Clip
				row["background"] = bg
			}
			if extras["root"] {
				if c.Layer {
					d = d.Layers[0]
				}
				pad := int64(20)
				row["root"] = d.Root
				scene, err := d2scenebuild.Build(ctx, d, d2scenebuild.Options{Pad: &pad})
				if err != nil {
					row["scene_error"] = err.Error()
				} else {
					prims := []any{}
					for _, n := range scene.Root.Children {
						if n.ID == "root:background" || n.ID == "root:double-border:outer" {
							prims = append(prims, map[string]any{"id": n.ID, "primitive": n.Primitive})
						}
					}
					row["viewbox"] = scene.ViewBox
					row["backgrounds"] = prims
				}
			}
		}
		rows = append(rows, row)
	}
	out, err := json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(output, out, 0o644); err != nil {
		t.Fatal(err)
	}
}
