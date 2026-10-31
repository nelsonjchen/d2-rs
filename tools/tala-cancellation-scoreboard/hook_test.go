// Original PreserveCompoundRoutes calls; trace checkout only.
package engine

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/graphjson"
	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
)

var d2rsInteriorCounts = map[string]int{}

func d2rsInteriorView(g *layoutgraph.Graph) any {
	if g == nil {
		return nil
	}
	nodes := []any{}
	edges := []any{}
	for _, n := range g.Nodes {
		var label, icon any
		if n.Label != nil {
			label = map[string]any{"text": n.Label.Text, "position": n.Label.Position.String(), "width": n.Label.Width, "height": n.Label.Height, "fixed": n.Label.PositionFixed()}
		}
		if n.Icon != nil {
			icon = map[string]any{"position": n.Icon.Position.String(), "fixed": n.Icon.PositionFixed()}
		}
		var parent any
		if n.Container != nil {
			parent = n.Container.ID
		}
		nodes = append(nodes, map[string]any{"id": n.ID, "parent": parent, "point": n.TopLeft, "width": n.Width, "height": n.Height, "label": label, "icon": icon})
	}
	for _, e := range g.Edges {
		var label, source, target any
		if e.Label != nil {
			label = map[string]any{"text": e.Label.Text, "position": e.Label.Position.String(), "width": e.Label.Width, "height": e.Label.Height, "fixed": e.Label.PositionFixed()}
		}
		arrow := func(l *layoutgraph.Label) any {
			if l == nil {
				return nil
			}
			return map[string]any{"text": l.Text, "width": l.Width, "height": l.Height}
		}
		source, target = arrow(e.SourceArrowheadLabel), arrow(e.TargetArrowheadLabel)
		edges = append(edges, map[string]any{"id": e.ID, "from": e.From.ID, "to": e.To.ID, "points": e.Points, "curve": e.IsCurve, "percentage": e.LabelPercentage, "label": label, "source_label": source, "target_label": target})
	}
	return map[string]any{"nodes": nodes, "edges": edges}
}

func d2rsInteriorInput(t *testing.T, g *layoutgraph.Graph) any {
	t.Helper()
	wire, err := graphjson.Serialize(context.Background(), g)
	if err != nil {
		t.Fatal(err)
	}
	return map[string]any{"wire": wire, "view": d2rsInteriorView(g)}
}

// Observe calls made by the unchanged original cancellation tests. The normal
// probe remains captured separately from the four cancellation calls.
func d2rsCompoundOriginal(t *testing.T, ctx context.Context, graph *layoutgraph.Graph) (*layoutgraph.Graph, error) {
	t.Helper()
	directory := os.Getenv("D2RS_TALA_CANCELLATION_CAPTURE")
	if directory == "" {
		return compoundWithPreservedRoutes(ctx, graph)
	}
	index := d2rsInteriorCounts[t.Name()]
	d2rsInteriorCounts[t.Name()]++
	schedule := map[string]any{"kind": "pre"}
	sourceSchedule := map[string]any{}
	if counted, ok := ctx.(*compoundCancelAfterChecks); ok {
		sourceSchedule["limit"] = counted.limit
		sourceSchedule["checks_before"] = counted.checks
		if counted.limit == 0 {
			schedule["kind"] = "probe"
		} else {
			schedule["kind"] = "fraction"
			schedule["fraction_index"] = index - 1
		}
	}
	request := map[string]any{"graph": d2rsInteriorInput(t, graph), "cancel": schedule}
	candidate, err := compoundWithPreservedRoutes(ctx, graph)
	if counted, ok := ctx.(*compoundCancelAfterChecks); ok {
		sourceSchedule["checks_after"] = counted.checks
	}
	diagnostic := ""
	if err != nil {
		diagnostic = err.Error()
	}
	cause := diagnostic
	if errors.Is(err, context.Canceled) {
		cause = context.Canceled.Error()
	}
	result := map[string]any{"graph": d2rsInteriorView(candidate), "cause": cause, "input_after": d2rsInteriorView(graph)}
	path := filepath.Join(directory, t.Name())
	if err := os.MkdirAll(path, 0755); err != nil {
		t.Fatal(err)
	}
	data, marshalErr := json.Marshal(map[string]any{"test": t.Name(), "call": index, "input": request, "source_schedule": sourceSchedule, "diagnostic": diagnostic, "result": result})
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	if err := os.WriteFile(filepath.Join(path, fmt.Sprintf("%03d.json", index)), data, 0644); err != nil {
		t.Fatal(err)
	}
	return candidate, err
}
