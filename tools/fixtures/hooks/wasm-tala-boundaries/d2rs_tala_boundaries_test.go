// SPDX-License-Identifier: MPL-2.0
// Source-layout boundary observations for d2-rs-wasm (recipe
// wasm-tala-boundaries): d2lib.Compile with the in-process engines, ported
// from the v0.9.0-era .reference/tala-{seed-config,deferred-seeds}-native-20261009
// go-modules (whose layout-probe.go only exported layoutPlan) and
// tools/layout-admission-scoreboard/context.go. This is an internal test of
// package d2talalayout so it calls layoutPlan directly. Inert unless run by
// scripts/regen-fixtures.py.
//
// Inputs, in order: tala-seed-config.json, tala-deferred-seeds.json,
// tala-seed-nesting.json, layout-context.json. Outputs: the four fixtures.
package d2talalayout

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strconv"
	"strings"
	"testing"
	"time"

	"github.com/d2lang/d2/d2compiler"
	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/d2/d2layouts"
	"github.com/d2lang/d2/d2layouts/d2dagrelayout"
	"github.com/d2lang/d2/d2layouts/d2elklayout"
	"github.com/d2lang/d2/d2lib"
	"github.com/d2lang/d2/lib/geo"
	d2log "github.com/d2lang/d2/lib/log"
	"github.com/d2lang/d2/lib/textmeasure"
)

type d2rsSeedInput struct {
	Name   string  `json:"name"`
	Source string  `json:"source"`
	Engine string  `json:"engine"`
	Seeds  []int64 `json:"seeds,omitempty"`
	Cancel bool    `json:"cancel,omitempty"`
}

type d2rsPlan struct {
	Seeds   []int64 `json:"seeds"`
	Workers int     `json:"workers"`
	Error   string  `json:"error"`
}

// d2rsCompilePlans compiles source with the item's engine and records the
// seed plan of every TALA layout invocation (go-module/main.go).
func d2rsCompilePlans(t *testing.T, item d2rsSeedInput) (string, []d2rsPlan) {
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	plans := []d2rsPlan{}
	options := &Options{Seeds: item.Seeds, MaxConcurrency: 1}
	resolver := func(engine string) (d2graph.LayoutGraph, error) {
		switch engine {
		case "tala":
			return func(ctx context.Context, g *d2graph.Graph) error {
				seeds, workers, err := layoutPlan(g, options)
				message := ""
				if err != nil {
					message = err.Error()
				}
				plans = append(plans, d2rsPlan{seeds, workers, message})
				return Layout(ctx, g, options)
			}, nil
		case "dagre":
			return d2dagrelayout.DefaultLayout, nil
		default:
			return d2elklayout.DefaultLayout, nil
		}
	}
	ctx, cancel := context.WithCancel(d2log.WithDefault(context.Background()))
	if item.Cancel {
		cancel()
	}
	defer cancel()
	_, _, err = d2lib.Compile(ctx, item.Source, &d2lib.CompileOptions{Layout: &item.Engine, Ruler: ruler, LayoutResolver: resolver,
		RouterResolver: func(string) (d2graph.RouteEdges, error) { return RouteEdges, nil }}, nil)
	if err != nil {
		return err.Error(), plans
	}
	return "", plans
}

type d2rsContextInput struct {
	Name   string  `json:"name"`
	Source string  `json:"source"`
	Path   string  `json:"path"`
	Engine string  `json:"engine"`
	Seeds  []int64 `json:"seeds"`
	Cancel bool    `json:"cancel"`
	Reason string  `json:"reason,omitempty"`
}

// d2rsContextError is tools/layout-admission-scoreboard/context.go: an active
// compile whose engine and router callbacks use a canceled or expired context.
func d2rsContextError(t *testing.T, item d2rsContextInput) string {
	base := d2log.WithDefault(context.Background())
	var layoutContext context.Context
	var cancel context.CancelFunc
	switch item.Reason {
	case "deadline", "deadline-then-cancel":
		layoutContext, cancel = context.WithDeadline(base, time.Now().Add(-time.Second))
		if item.Reason == "deadline-then-cancel" {
			cancel()
		}
	case "cancel-then-deadline":
		parent, cancelParent := context.WithCancel(base)
		cancelParent()
		layoutContext, cancel = context.WithDeadline(parent, time.Now().Add(-time.Second))
	default:
		layoutContext, cancel = context.WithCancel(base)
		if item.Cancel || item.Reason == "cancel" {
			cancel()
		}
	}
	options := &Options{Seeds: item.Seeds, MaxConcurrency: 1}
	resolver := func(engine string) (d2graph.LayoutGraph, error) {
		return func(_ context.Context, g *d2graph.Graph) error {
			switch engine {
			case "tala":
				return Layout(layoutContext, g, options)
			case "dagre":
				return d2dagrelayout.DefaultLayout(layoutContext, g)
			default:
				return d2elklayout.DefaultLayout(layoutContext, g)
			}
		}, nil
	}
	router := func(engine string) (d2graph.RouteEdges, error) {
		return func(_ context.Context, g *d2graph.Graph, edges []*d2graph.Edge) error {
			if engine == "tala" {
				return RouteEdges(layoutContext, g, edges)
			}
			return d2layouts.DefaultRouter(layoutContext, g, edges)
		}, nil
	}
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	_, _, err = d2lib.Compile(base, item.Source, &d2lib.CompileOptions{Layout: &item.Engine, Ruler: ruler, LayoutResolver: resolver, RouterResolver: router}, nil)
	cancel()
	if err != nil {
		return err.Error()
	}
	return ""
}

// The committed grid: per group, runs of [base, source, path, seeds,
// "engine:variant ..."]; the variant fixes cancel/reason and the name suffix.
var d2rsVariants = map[string]map[string]struct {
	cancel bool
	reason string
}{
	"cancellation": {"control": {false, ""}, "cancel": {true, ""}},
	"deadlines": {"control": {false, "active"}, "deadline": {true, "deadline"}, "cancel": {false, "cancel"},
		"cancel-then-deadline": {false, "cancel-then-deadline"}, "deadline-then-cancel": {false, "deadline-then-cancel"}},
}

func d2rsContextInputs(t *testing.T, group string, runs [][]json.RawMessage) []d2rsContextInput {
	var out []d2rsContextInput
	for _, run := range runs {
		var base, source, path, calls string
		var seeds []int64
		for i, target := range []any{&base, &source, &path, &seeds, &calls} {
			if err := json.Unmarshal(run[i], target); err != nil {
				t.Fatal(err)
			}
		}
		for _, call := range strings.Fields(calls) {
			engine, variant, _ := strings.Cut(call, ":")
			v, ok := d2rsVariants[group][variant]
			if !ok {
				t.Fatalf("unknown %s variant %q", group, variant)
			}
			out = append(out, d2rsContextInput{base + "-" + engine + "-" + variant, source, path, engine, seeds, v.cancel, v.reason})
		}
	}
	return out
}

// d2rsRawLabelGraph builds the d2graph that a committed raw layout graph
// (d2-rs layoutD2Json input: ids, boxes, width/height attributes and edge
// label values, no label dimensions) denotes, without measuring any text.
func d2rsRawLabelGraph(t *testing.T, raw json.RawMessage) *d2graph.Graph {
	var graph struct {
		Objects []struct {
			ID  string `json:"id"`
			Box struct {
				Width, Height float64
			} `json:"box"`
			Attributes map[string]struct {
				Value string `json:"value"`
			} `json:"attributes"`
		} `json:"objects"`
		Edges []struct {
			Src, Dst   string
			Attributes map[string]struct {
				Value string `json:"value"`
			} `json:"attributes"`
		} `json:"edges"`
	}
	if err := json.Unmarshal(raw, &graph); err != nil {
		t.Fatal(err)
	}
	var source strings.Builder
	for _, o := range graph.Objects {
		source.WriteString(o.ID)
		if w, ok := o.Attributes["width"]; ok {
			source.WriteString(": {width: " + w.Value + "; height: " + o.Attributes["height"].Value + "}")
		}
		source.WriteString("\n")
	}
	for _, e := range graph.Edges {
		source.WriteString(e.Src + " -> " + e.Dst)
		if l, ok := e.Attributes["label"]; ok {
			source.WriteString(": " + l.Value)
		}
		source.WriteString("\n")
	}
	g, _, err := d2compiler.Compile("-", strings.NewReader(source.String()), nil)
	if err != nil {
		t.Fatal(err)
	}
	if len(g.Objects) != len(graph.Objects) || len(g.Edges) != len(graph.Edges) {
		t.Fatalf("raw graph rebuilt as %d objects/%d edges", len(g.Objects), len(g.Edges))
	}
	for i, o := range graph.Objects {
		if g.Objects[i].AbsID() != o.ID {
			t.Fatalf("object %d is %q, want %q", i, g.Objects[i].AbsID(), o.ID)
		}
		g.Objects[i].Box = geo.NewBox(nil, o.Box.Width, o.Box.Height)
	}
	return g
}

func d2rsRead(t *testing.T, path string, into any) {
	b, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(b, into); err != nil {
		t.Fatal(err)
	}
}

func d2rsWrite(t *testing.T, path string, value any) {
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

func TestD2RSCaptureTalaBoundaries(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	inputs := strings.Split(os.Getenv("D2RS_RECIPE_INPUTS"), string(filepath.ListSeparator))
	outputs := strings.Split(os.Getenv("D2RS_RECIPE_OUTPUTS"), string(filepath.ListSeparator))
	if len(inputs) != 5 || len(outputs) != 5 {
		t.Fatalf("want 5 inputs and 5 outputs, got %d and %d", len(inputs), len(outputs))
	}
	var manifest struct {
		D2 struct {
			Commit string `json:"commit"`
		} `json:"d2"`
	}
	d2rsRead(t, filepath.Join(os.Getenv("D2RS_REPO_ROOT"), "reference-manifest.json"), &manifest)
	commit := manifest.D2.Commit

	type seedConfigCase struct {
		Input json.RawMessage `json:"input"`
		Error string          `json:"error"`
		Seeds []int64         `json:"seeds"`
	}
	type deferredCase struct {
		Input json.RawMessage `json:"input"`
		Error string          `json:"error"`
		Calls int             `json:"source_core_calls"`
	}
	var raw []json.RawMessage
	d2rsRead(t, inputs[0], &raw)
	seedConfig := []seedConfigCase{}
	for _, r := range raw {
		var item d2rsSeedInput
		if err := json.Unmarshal(r, &item); err != nil {
			t.Fatal(err)
		}
		message, plans := d2rsCompilePlans(t, item)
		var seeds []int64
		if len(plans) > 0 {
			seeds = plans[0].Seeds
		}
		seedConfig = append(seedConfig, seedConfigCase{r, message, seeds})
	}
	d2rsWrite(t, outputs[0], map[string]any{"source_commit": commit, "scope": "d2lib.Compile error/acceptance and the seed plan of the first TALA invocation (nil when none ran).", "cases": seedConfig})

	raw = nil
	d2rsRead(t, inputs[1], &raw)
	deferred := []deferredCase{}
	for _, r := range raw {
		var item d2rsSeedInput
		if err := json.Unmarshal(r, &item); err != nil {
			t.Fatal(err)
		}
		message, plans := d2rsCompilePlans(t, item)
		deferred = append(deferred, deferredCase{r, message, len(plans)})
	}
	d2rsWrite(t, outputs[1], map[string]any{"source_commit": commit, "scope": "d2lib.Compile error/acceptance and the number of TALA layout invocations across source diagram scopes; graph geometry is not recorded.", "cases": deferred})

	var nestingInputs []struct {
		Name   string `json:"name"`
		Source string `json:"source"`
	}
	d2rsRead(t, inputs[2], &nestingInputs)
	nesting := []any{}
	for _, c := range nestingInputs {
		calls := []int{}
		for _, seed := range []int64{1, 7} {
			message, plans := d2rsCompilePlans(t, d2rsSeedInput{Name: c.Name, Source: c.Source, Engine: "tala", Seeds: []int64{seed}})
			if message != "" {
				t.Fatalf("%s: %s", c.Name, message)
			}
			calls = append(calls, len(plans))
		}
		nesting = append(nesting, map[string]any{"name": c.Name, "source": c.Source, "source_layout_calls": calls})
	}
	d2rsWrite(t, outputs[2], map[string]any{"source_commit": commit, "cases": nesting})

	var grid map[string][][]json.RawMessage
	d2rsRead(t, inputs[3], &grid)
	type contextCase struct {
		Input d2rsContextInput `json:"input"`
		Error string           `json:"error"`
	}
	groups := map[string][]contextCase{}
	for _, group := range []string{"cancellation", "deadlines"} {
		cases := []contextCase{}
		for _, item := range d2rsContextInputs(t, group, grid[group]) {
			cases = append(cases, contextCase{item, d2rsContextError(t, item)})
		}
		groups[group] = cases
	}
	d2rsWrite(t, outputs[3], map[string]any{
		"source_commit": commit,
		"projection":    "Errors at the post-compilation engine/router boundary for canceled or expired layout contexts (tools/layout-admission-scoreboard/context.go); compilation itself uses an active context.",
		"inputs":        "tools/d2js-capture/inputs/layout-context.json",
		"cancellation":  groups["cancellation"],
		"deadlines":     groups["deadlines"],
	})

	// Raw labelled graphs straight into Layout: every seed set and worker bound.
	var graphs []json.RawMessage
	d2rsRead(t, inputs[4], &graphs)
	rawCases := []any{}
	for _, raw := range graphs {
		for _, seeds := range [][]int64{{1}, {1, 2, 3}} {
			for concurrency := 0; concurrency <= 2; concurrency++ {
				err := Layout(context.Background(), d2rsRawLabelGraph(t, raw), &Options{Seeds: seeds, MaxConcurrency: concurrency})
				message := ""
				if err != nil {
					message = err.Error()
				}
				rawCases = append(rawCases, map[string]any{"name": "raw-label-" + strconv.Itoa(len(rawCases)), "graph": raw, "seeds": seeds, "concurrency": concurrency, "error": message})
			}
		}
	}
	d2rsWrite(t, outputs[4], map[string]any{"source_commit": commit, "scope": "Raw labelled layout graphs (no label dimensions) passed to d2talalayout.Layout directly.", "cases": rawCases})
}
