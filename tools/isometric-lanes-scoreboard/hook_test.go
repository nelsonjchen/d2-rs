// SPDX-License-Identifier: MPL-2.0
package d2isometricimg

import (
	"context"
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2isometric"
	"math"
	"os"
	"path/filepath"
	"testing"
)

var d2rsGeometryRows []any

func d2rsGeometryStart(t *testing.T) {
	d2rsGeometryRows = []any{}
	t.Cleanup(func() {
		data, err := json.Marshal(map[string]any{"name": t.Name(), "operations": d2rsGeometryRows})
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), t.Name()+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	})
}
func d2rsGeometryRecord(request, got any) {
	// Freeze each observation now: original ownership tests later mutate returns.
	data, err := json.Marshal(map[string]any{"request": request, "got": got})
	if err != nil {
		panic(err)
	}
	d2rsGeometryRows = append(d2rsGeometryRows, json.RawMessage(data))
}
func d2rsGeometryError(err error) string {
	if err != nil {
		return err.Error()
	}
	return ""
}
func d2rsGeometryNumber(v float64) any {
	if math.IsNaN(v) {
		return "NaN"
	}
	if math.IsInf(v, 1) {
		return "+Inf"
	}
	if math.IsInf(v, -1) {
		return "-Inf"
	}
	return v
}
func d2rsGeometrySurface(s labelSurface) any {
	return map[string]any{"center": s.center, "width": s.width, "depth": s.depth, "angle": s.angle, "align": s.align}
}
func d2rsGeometryHeader(surface labelSurface, board d2isometric.Board, owner d2isometric.Node, nodes []d2isometric.Node, scale float64) labelSurface {
	got, _ := d2rsGeometryPlace(surface, board, owner, nodes, scale, false)
	return got
}
func d2rsGeometryPlace(surface labelSurface, board d2isometric.Board, owner d2isometric.Node, nodes []d2isometric.Node, scale float64, placement bool) (labelSurface, bool) {
	got, fits := hierarchyBoardHeaderPlacement(surface, board, owner, nodes, scale)
	request := map[string]any{"op": "header", "surface": d2rsGeometrySurface(surface), "board": board, "owner": owner, "nodes": nodes, "scale": scale, "placement": placement}
	var fit any
	if placement {
		fit = fits
	}
	d2rsGeometryRecord(request, map[string]any{"surface": d2rsGeometrySurface(got), "fits": fit, "board": board, "owner": owner, "nodes": nodes})
	return got, fits
}
func d2rsGeometryPlacement(surface labelSurface, board d2isometric.Board, owner d2isometric.Node, nodes []d2isometric.Node, scale float64) (labelSurface, bool) {
	return d2rsGeometryPlace(surface, board, owner, nodes, scale, true)
}
func d2rsGeometryEdges(edges []d2isometric.Edge) []any {
	out := []any{}
	for _, e := range edges {
		points := []any{}
		for _, p := range e.Points {
			points = append(points, []any{d2rsGeometryNumber(p.X), d2rsGeometryNumber(p.Y), d2rsGeometryNumber(p.Z)})
		}
		safe := e
		safe.Points = nil
		data, err := json.Marshal(safe)
		if err != nil {
			panic(err)
		}
		var record map[string]any
		if err := json.Unmarshal(data, &record); err != nil {
			panic(err)
		}
		record["points"] = points
		out = append(out, record)
	}
	return out
}
func d2rsGeometryBridge(ctx context.Context, edges []d2isometric.Edge) ([][]Vec, error) {
	return d2rsGeometryBridgeLimit(ctx, edges, nativeBridgeMaxWork)
}
func d2rsGeometryBridgeLimit(ctx context.Context, edges []d2isometric.Edge, limit int) ([][]Vec, error) {
	kind := "live"
	if ctx == nil {
		kind = "nil"
	} else if ctx.Err() != nil {
		kind = "canceled"
	}
	request := map[string]any{"op": "bridge", "context": kind, "edges": d2rsGeometryEdges(edges), "limit": limit}
	got, err := nativeResolveBridgeRoutes(ctx, edges, limit)
	d2rsGeometryRecord(request, map[string]any{"paths": got, "error": d2rsGeometryError(err), "source": d2rsGeometryEdges(edges)})
	return got, err
}

func d2rsGeometryLanes(ctx context.Context, edges []d2isometric.Edge, nodes []d2isometric.Node, boards []d2isometric.Board) ([][]Vec, error) {
	return d2rsGeometryLanesLimit(ctx, edges, nodes, boards, nativeLaneMaxWork)
}
func d2rsGeometryLanesLimit(ctx context.Context, edges []d2isometric.Edge, nodes []d2isometric.Node, boards []d2isometric.Board, limit int) ([][]Vec, error) {
	kind := "live"
	if ctx == nil {
		kind = "nil"
	} else if ctx.Err() != nil {
		kind = "canceled"
	}
	request := map[string]any{"op": "lane", "context": kind, "edges": d2rsGeometryEdges(edges), "nodes": nodes, "boards": boards, "limit": limit}
	got, err := nativeResolveLaneRoutes(ctx, edges, nodes, boards, limit)
	d2rsGeometryRecord(request, map[string]any{"paths": got, "error": d2rsGeometryError(err), "source": d2rsGeometryEdges(edges), "nodes": nodes, "boards": boards})
	return got, err
}
func d2rsGeometryPackets(b *meshBuilder, edges []d2isometric.Edge, placer *routeCaptionPlacer) []packetRoute {
	request := map[string]any{"op": "pipeline", "edges": d2rsGeometryEdges(edges)}
	packets := b.edges(edges, placer)
	paths, lengths := [][]Vec{}, [][]float64{}
	for _, p := range packets {
		paths = append(paths, p.points)
		lengths = append(lengths, p.lengths)
	}
	d2rsGeometryRecord(request, map[string]any{"paths": paths, "lengths": lengths, "error": d2rsGeometryError(b.err), "source": d2rsGeometryEdges(edges)})
	return packets
}
