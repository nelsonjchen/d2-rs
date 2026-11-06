// SPDX-License-Identifier: MPL-2.0
// Original-test observers installed only in the pinned trace checkout.
package d2isometricimg

import (
	"context"
	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2target"
)

func init() { rasterInputs = map[*Raster]map[string]any{} }
func compositionEdges(b *meshBuilder, n d2isometric.Node, ts []Triangle) {
	req := map[string]any{"op": "ink", "state": shapeState(b, true), "node": n, "mesh": solidMesh(ts, true)}
	b.classicInkEdges(n, ts)
	solidRecord(req, map[string]any{"state": shapeState(b, false), "node": n})
}
func compositionSegmentsValue(ss []classicInkSegment) any {
	out := []any{}
	for _, s := range ss {
		fs := []any{}
		for _, f := range s.support {
			fs = append(fs, map[string]any{"points": f.points, "bias": f.bias, "keys": f.keys, "normal": f.normal})
		}
		out = append(out, map[string]any{"a": s.a, "c": s.c, "support": fs})
	}
	return out
}
func compositionSegments(ctx context.Context, n d2isometric.Node, ts []Triangle, radius ...float64) ([]classicInkSegment, error) {
	req := map[string]any{"op": "segments", "node": n, "mesh": solidMesh(ts, true), "radius": radius, "canceled": ctx.Err() != nil}
	ss, e := classicInkSegments(ctx, n, ts, radius...)
	solidRecord(req, map[string]any{"segments": compositionSegmentsValue(ss), "error": rasterError(e)})
	return ss, e
}
func compositionJunctions(b *meshBuilder, ss, ends []classicInkSegment, r float64, m *Material) {
	req := map[string]any{"op": "junctions", "state": shapeState(b, true), "segments": compositionSegmentsValue(ss), "ends": compositionSegmentsValue(ends), "radius": r, "material": solidMesh([]Triangle{{Material: m}}, true)}
	b.classicInkJunctions(ss, ends, r, m)
	solidRecord(req, shapeState(b, false))
}
func compositionLinks(b *meshBuilder) any {
	out := []any{}
	for _, l := range b.links {
		out = append(out, map[string]any{"points": l.points, "region": map[string]any{"url": l.region.URL, "tooltip": l.region.Tooltip, "target": l.region.Target}})
	}
	return out
}
func compositionState(b *meshBuilder, raw bool) any {
	links := b.links
	b.links = nil
	state := hierState(b, raw)
	b.links = links
	return map[string]any{"builder": state, "links": compositionLinks(b), "linkBytes": b.linkBytes}
}
func compositionNode(b *meshBuilder, op string, n d2isometric.Node, tint string) {
	links := b.links
	b.links = nil
	req := hierRequest(b, op)
	b.links = links
	if len(links) != 0 {
		panic("nonempty starting link list")
	}
	req["node"], req["tint"], req["linkBudget"] = n, tint, b.options.links
	if op == "node" {
		b.node(n, tint)
	} else {
		b.hierarchyNode(n, tint)
	}
	solidRecord(req, map[string]any{"state": compositionState(b, false), "node": n})
}
func compositionContent(b *meshBuilder, n d2isometric.Node, s d2target.Shape, fill string, top float64) {
	links := b.links
	b.links = nil
	req := hierRequest(b, "content")
	b.links = links
	if len(links) != 0 {
		panic("nonempty starting link list")
	}
	req["node"], req["shape"], req["fill"], req["top"], req["linkBudget"], req["tint"] = n, s, fill, top, b.options.links, ""
	b.canonicalNodeContent(n, s, fill, top)
	solidRecord(req, map[string]any{"state": compositionState(b, false), "node": n})
}
