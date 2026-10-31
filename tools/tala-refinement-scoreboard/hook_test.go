// Original-call observer for pinned master layout.go:308–345.
// Installed only in the manifest's trace checkout. Original assertions remain.
package d2talalayout

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/graphjson"
)

var d2rsRefinementMu sync.Mutex
var d2rsRefinementCount = map[string]int{}

func d2rsRefinementValue(t *testing.T, value seedResult) any {
	t.Helper()
	var graph any
	if value.graph != nil {
		var err error
		graph, err = graphjson.Serialize(context.Background(), value.graph)
		if err != nil {
			t.Fatal(err)
		}
	}
	// These original calls have no translation bindings; never silently omit them.
	if value.bindings.objectIDs != nil || value.bindings.edgeIDs != nil || value.bindings.edgeDestinations != nil {
		t.Fatal("nonempty translation requires an extended observer")
	}
	return map[string]any{"graph": graph, "penalty": value.score.penalty,
		"area": value.score.area, "sequence_edges": value.sequenceEdges,
		"bindings": []any{nil, nil, nil}}
}

func d2rsRefinement(t *testing.T, ctx context.Context, incumbent seedResult, build func() (seedResult, error), ordinary bool) (seedResult, error) {
	t.Helper()
	if os.Getenv("D2RS_TALA_REFINEMENT_CAPTURE") == "" {
		if ordinary {
			return considerSeedCandidate(ctx, incumbent, build)
		}
		return refineSeedResult(ctx, incumbent, build)
	}
	request := map[string]any{"ordinary": ordinary, "incumbent": d2rsRefinementValue(t, incumbent),
		"candidate": nil, "build_error": "", "panic": false, "context_before": fmt.Sprint(ctx.Err())}
	wrapped := func() (seedResult, error) {
		defer func() {
			if recovered := recover(); recovered != nil {
				request["panic"] = true
				panic(recovered)
			}
		}()
		candidate, err := build()
		request["candidate"] = d2rsRefinementValue(t, candidate)
		if err != nil {
			request["build_error"] = err.Error()
		}
		return candidate, err
	}
	var selected seedResult
	var err error
	if ordinary {
		selected, err = considerSeedCandidate(ctx, incumbent, wrapped)
	} else {
		selected, err = refineSeedResult(ctx, incumbent, wrapped)
	}
	request["context_after"] = fmt.Sprint(ctx.Err())
	var published any
	text := ""
	if err == nil {
		published = d2rsRefinementValue(t, selected)
	} else {
		text = err.Error()
	}
	result := map[string]any{"selected": published, "error": text,
		"incumbent_after": d2rsRefinementValue(t, incumbent)}
	d2rsRefinementMu.Lock()
	defer d2rsRefinementMu.Unlock()
	index := d2rsRefinementCount[t.Name()]
	d2rsRefinementCount[t.Name()]++
	directory := filepath.Join(os.Getenv("D2RS_TALA_REFINEMENT_CAPTURE"), t.Name())
	if err := os.MkdirAll(directory, 0755); err != nil {
		t.Fatal(err)
	}
	data, marshalErr := json.Marshal(map[string]any{"test": t.Name(), "call": index, "input": request, "result": result})
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	if err := os.WriteFile(filepath.Join(directory, fmt.Sprintf("%03d.json", index)), data, 0644); err != nil {
		t.Fatal(err)
	}
	return selected, err
}

func d2rsRefineOriginal(t *testing.T, ctx context.Context, incumbent seedResult, build func() (seedResult, error)) (seedResult, error) {
	return d2rsRefinement(t, ctx, incumbent, build, false)
}

func d2rsConsiderOriginal(t *testing.T, ctx context.Context, incumbent seedResult, build func() (seedResult, error)) (seedResult, error) {
	return d2rsRefinement(t, ctx, incumbent, build, true)
}
