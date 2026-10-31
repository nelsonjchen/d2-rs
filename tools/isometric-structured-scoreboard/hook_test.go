// SPDX-License-Identifier: MPL-2.0
// Original-test observations, installed only in the pinned trace checkout.
package d2isometricimg

import (
	"context"
	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2renderers/d2scene"
)

func structuredRich(b *meshBuilder) any {
	p := b.rich
	if p == nil {
		return nil
	}
	return map[string]any{"count": p.count, "used": p.used, "pixels": p.pixels, "bytes": p.sourceBytes, "rows": p.rows, "primary": p.primary, "mono": p.mono, "theme": p.themeID, "overrides": p.themeOverrides}
}
func structuredRequest(b *meshBuilder, op string, n d2isometric.Node) map[string]any {
	r := shapeRequest(b, op)
	r["rich"] = structuredRich(b)
	r["node"] = n
	return r
}
func structuredResult(b *meshBuilder, n d2isometric.Node) any {
	return map[string]any{"state": shapeState(b, false), "rich": structuredRich(b), "node": n}
}
func structuredObserveNode(b *meshBuilder, op string, n d2isometric.Node, tint ...string) {
	r := structuredRequest(b, op, n)
	r["tint"] = tint
	if op == "hierarchy" {
		b.hierarchyNode(n, tint[0])
	} else {
		b.structuredNode(n)
	}
	solidRecord(r, structuredResult(b, n))
}
func structuredObserveRail(b *meshBuilder, n d2isometric.Node, z, depth, bottom, top float64, fill string) {
	r := structuredRequest(b, "rail", n)
	r["z"], r["depth"], r["bottom"], r["top"], r["fill"] = z, depth, bottom, top, fill
	b.structuredRail(n, z, depth, bottom, top, fill)
	solidRecord(r, structuredResult(b, n))
}
func structuredObserveDocument(b *meshBuilder, n d2isometric.Node) (*d2scene.Document, []nativeStructuredCell, error) {
	r := structuredRequest(b, "document", n)
	doc, cells, err := b.structuredDocument(n)
	out := map[string]any{"result": structuredResult(b, n), "error": "", "cells": []any{}, "viewport": nil}
	if err != nil {
		out["error"] = err.Error()
	}
	cs := []any{}
	for _, c := range cells {
		cs = append(cs, map[string]any{"id": c.node.ID, "row": c.row, "run": c.node.Primitive})
	}
	out["cells"] = cs
	if doc != nil {
		out["viewport"] = map[string]any{"box": doc.ViewBox, "width": doc.LogicalWidth, "height": doc.LogicalHeight, "fit": doc.ViewportFit, "align": doc.ViewportAlign}
	}
	solidRecord(r, out)
	return doc, cells, err
}
func structuredObserveIcon(n d2isometric.Node, y float64) labelSurface {
	s := nativeStructuredIconSurface(n, y)
	solidRecord(map[string]any{"op": "icon", "node": n, "y": y}, solidSurface(s))
	return s
}
func structuredObserveRoutes(ctx context.Context, es []d2isometric.Edge, ns []d2isometric.Node, boards []d2isometric.Board, supports ...map[string]float64) ([][]Vec, [][]Vec, error) {
	lanes, paths, e := nativeEdgeRoutes(ctx, es, ns, boards, supports...)
	err := ""
	if e != nil {
		err = e.Error()
	}
	solidRecord(map[string]any{"op": "routes", "canceled": ctx.Err() != nil, "edges": es, "nodes": ns, "boards": boards, "supports": supports}, map[string]any{"lanes": lanes, "paths": paths, "error": err})
	return lanes, paths, e
}
func structuredObserveCaption(e d2isometric.Edge, scale float64) ([]nativeSequenceCaption, labelSurface) {
	c, i := nativeSequenceCaptionData(e, scale)
	cs := []any{}
	for _, v := range c {
		cs = append(cs, map[string]any{"style": v.style, "surface": solidSurface(v.surface), "main": v.main})
	}
	solidRecord(map[string]any{"op": "caption", "edge": e, "scale": scale}, map[string]any{"captions": cs, "icon": solidSurface(i)})
	return c, i
}
func structuredPlacer(p *routeCaptionPlacer) any {
	rs := []any{}
	for _, r := range p.rects {
		h := [][2]float64{}
		for _, v := range r.hull {
			h = append(h, [2]float64{v.x, v.z})
		}
		rs = append(rs, map[string]any{"rect": map[string]any{"x": r.x, "z": r.z, "ux": r.ux, "uz": r.uz, "hw": r.hw, "hd": r.hd, "left": r.left, "right": r.right, "front": r.front, "back": r.back, "hard": r.hard, "hull": h}, "count": 1})
	}
	return map[string]any{"rects": rs, "work": p.work, "refs": p.refs}
}
func structuredObserveEdges(b *meshBuilder, es []d2isometric.Edge, p *routeCaptionPlacer, scenes ...*d2isometric.Scene) []packetRoute {
	r := shapeRequest(b, "edges")
	r["rich"] = structuredRich(b)
	r["edges"], r["placer"], r["scenes"] = es, structuredPlacer(p), scenes
	packets := b.edges(es, p, scenes...)
	ps := []any{}
	for _, v := range packets {
		ps = append(ps, map[string]any{"points": v.points, "lengths": v.lengths, "material": solidMesh([]Triangle{{Material: v.material}}, false), "forward": v.forward, "reverse": v.reverse})
	}
	solidRecord(r, map[string]any{"state": shapeState(b, false), "rich": structuredRich(b), "edges": es, "placer": structuredPlacer(p), "packets": ps})
	return packets
}
