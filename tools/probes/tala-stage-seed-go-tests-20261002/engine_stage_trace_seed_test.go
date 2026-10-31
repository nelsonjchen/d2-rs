// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
package engine

import (
	"encoding/json"
	"io"
	"os"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/layoutgraph"
)

func TestDiagnosticStageSeedMetadata(t *testing.T) {
	t.Setenv("TALA_TRACE_JSONL", "1")
	for _, seed := range []int64{-17, 0, 1, 17} {
		p := newPipeline(layoutgraph.NewGraph(), seed, false)
		event := captureDiagnosticStageSeedEvent(t, func() { p.emitNormalizedTrace("Probe") })
		got, ok := event["seed"].(float64)
		if !ok || int64(got) != seed {
			t.Fatalf("known seed %d event=%v", seed, event)
		}
		if _, ok := event["scope"]; ok {
			t.Fatalf("known seed has unknown scope: %v", event)
		}
	}
	unknown := &pipeline{graph: layoutgraph.NewGraph()}
	event := captureDiagnosticStageSeedEvent(t, func() { unknown.emitNormalizedTrace("UnboundProbe") })
	if seed, ok := event["seed"]; !ok || seed != nil || event["scope"] != "seed-unavailable" {
		t.Fatalf("unknown seed must be null with explicit scope: %v", event)
	}
	selectedSeed := int64(-17)
	selected := captureDiagnosticStageSeedEvent(t, func() { TraceSelectedGraph(layoutgraph.NewGraph(), &selectedSeed) })
	if selected["seed"] != float64(-17) || selected["stage"] != "SelectedResult" {
		t.Fatalf("selected=%v", selected)
	}
	selectedUnknown := captureDiagnosticStageSeedEvent(t, func() { TraceSelectedGraph(layoutgraph.NewGraph(), nil) })
	if selectedUnknown["seed"] != nil || selectedUnknown["scope"] != "seed-unavailable" {
		t.Fatalf("unknown selected=%v", selectedUnknown)
	}
}

func captureDiagnosticStageSeedEvent(t *testing.T, emit func()) map[string]any {
	t.Helper()
	read, write, err := os.Pipe()
	if err != nil {
		t.Fatal(err)
	}
	prior := os.Stderr
	os.Stderr = write
	emit()
	os.Stderr = prior
	if err := write.Close(); err != nil {
		t.Fatal(err)
	}
	data, err := io.ReadAll(read)
	if err != nil {
		t.Fatal(err)
	}
	if err := read.Close(); err != nil {
		t.Fatal(err)
	}
	var event map[string]any
	if err := json.Unmarshal(data, &event); err != nil {
		t.Fatalf("event=%q error=%v", data, err)
	}
	return event
}
