// SPDX-License-Identifier: MPL-2.0
// Original tests keep their bodies and assertions; only resolver calls are observed.
package d2cli

import (
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/d2graph"
	"github.com/d2lang/util-go/xmain"
)

var d2rsResolverMu sync.Mutex
var d2rsResolverEvents []map[string]any
var d2rsResolverNext int

func d2rsResolverRecord(t *testing.T, event map[string]any) {
	t.Helper()
	d2rsResolverMu.Lock()
	defer d2rsResolverMu.Unlock()
	event["test"] = t.Name()
	d2rsResolverEvents = append(d2rsResolverEvents, event)
	data, err := json.Marshal(d2rsResolverEvents)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_RESOLVER_CAPTURE"), "calls.json"), data, 0600); err != nil {
		t.Fatal(err)
	}
}

func d2rsResolverFlags(t *testing.T, ms *xmain.State) map[string]any {
	t.Helper()
	flags := map[string]any{}
	if ms == nil {
		return flags
	}
	for _, name := range []string{"dagre-nodesep", "dagre-edgesep", "elk-nodeNodeBetweenLayers", "elk-edgeNodeBetweenLayers", "elk-edgeEdgeBetweenLayers", "elk-nodeSelfLoop"} {
		if hasLayoutFlag(ms, name) {
			v, err := ms.Opts.Flags.GetInt64(name)
			if err != nil {
				t.Fatal(err)
			}
			flags[name] = v
		}
	}
	for _, name := range []string{"elk-algorithm", "elk-padding"} {
		if hasLayoutFlag(ms, name) {
			v, err := ms.Opts.Flags.GetString(name)
			if err != nil {
				t.Fatal(err)
			}
			flags[name] = v
		}
	}
	if hasLayoutFlag(ms, "tala-seeds") {
		v, err := ms.Opts.Flags.GetInt64Slice("tala-seeds")
		if err != nil {
			t.Fatal(err)
		}
		flags["tala-seeds"] = v
	}
	return flags
}

func d2rsLayoutResolver(t *testing.T, ctx context.Context, ms *xmain.State) func(string) (d2graph.LayoutGraph, error) {
	t.Helper()
	d2rsResolverMu.Lock()
	id := d2rsResolverNext
	d2rsResolverNext++
	d2rsResolverMu.Unlock()
	d2rsResolverRecord(t, map[string]any{"op": "create", "resolver": id, "flags": d2rsResolverFlags(t, ms)})
	resolve := LayoutResolver(ctx, ms)
	return func(name string) (d2graph.LayoutGraph, error) {
		layout, err := resolve(name)
		message := ""
		if err != nil {
			message = err.Error()
		}
		d2rsResolverRecord(t, map[string]any{"op": "resolve", "resolver": id, "name": name, "flags": d2rsResolverFlags(t, ms), "error": message})
		if err != nil {
			return nil, err
		}
		return func(ctx context.Context, g *d2graph.Graph) error {
			before, e := d2graph.SerializeGraph(g)
			if e != nil {
				t.Fatal(e)
			}
			err := layout(ctx, g)
			after, e := d2graph.SerializeGraph(g)
			if e != nil {
				t.Fatal(e)
			}
			message := ""
			if err != nil {
				message = err.Error()
			}
			d2rsResolverRecord(t, map[string]any{"op": "layout", "resolver": id, "name": name, "input": json.RawMessage(before), "result": map[string]any{"error": message, "graph": json.RawMessage(after)}})
			return err
		}, nil
	}
}
