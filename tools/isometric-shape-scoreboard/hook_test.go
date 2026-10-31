// SPDX-License-Identifier: MPL-2.0
// Observation of original tests; only installed in the pinned trace checkout.
package d2isometricimg

import (
	"context"
	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2target"
	"image"
)

func shapeRequest(b *meshBuilder, op string) map[string]any {
	return map[string]any{"op": op, "state": shapeState(b, true), "canceled": b.ctx.Err() != nil, "text": b.text != nil, "vector": b.options.vector, "background": b.arrowBackground}
}
func shapeObserveSource(n d2isometric.Node, fill string) d2target.Shape {
	s := nativeFaceSource(n, fill)
	solidRecord(map[string]any{"op": "source", "node": n, "fill": fill}, s)
	return s
}
func shapeObserveProfile(s d2target.Shape) ([]Vec, error) {
	p, e := nativeShapeProfile(s)
	err := ""
	if e != nil {
		err = e.Error()
	}
	solidRecord(map[string]any{"op": "profile", "shape": s}, map[string]any{"profile": p, "error": err})
	return p, e
}
func shapeObserveProfiles(s d2target.Shape) ([][]Vec, error) {
	p, e := nativeShapeProfiles(s)
	err := ""
	if e != nil {
		err = e.Error()
	}
	solidRecord(map[string]any{"op": "profiles", "shape": s}, map[string]any{"profile": p, "error": err})
	return p, e
}
func shapeObserveLabel(n d2isometric.Node, s d2target.Shape, y float64) labelSurface {
	a := nativeNodeLabelSurface(n, s, y)
	solidRecord(map[string]any{"op": "label", "node": n, "shape": s, "y": y}, solidSurface(a))
	return a
}
func shapeObserveFace(b *meshBuilder, s d2target.Shape) (*image.RGBA, labelSurface) {
	r := shapeRequest(b, "face")
	r["shape"] = s
	t, a := b.nativeFace(s)
	solidRecord(r, map[string]any{"state": shapeState(b, false), "texture": solidTexture(t, false), "area": solidSurface(a)})
	return t, a
}
func shapeObserveLayers(b *meshBuilder, s d2target.Shape, opacity ...float64) (*image.RGBA, *image.RGBA, labelSurface) {
	r := shapeRequest(b, "layers")
	r["shape"], r["opacity"] = s, opacity
	f, i, a := b.nativeFaceLayers(s, opacity...)
	solidRecord(r, map[string]any{"state": shapeState(b, false), "fill": solidTexture(f, false), "ink": solidTexture(i, false), "area": solidSurface(a)})
	return f, i, a
}
func shapeObserveNode(b *meshBuilder, op string, n d2isometric.Node, tint string) {
	r := shapeRequest(b, op)
	r["node"], r["tint"] = n, tint
	switch op {
	case "node":
		b.node(n, tint)
	case "hierarchy":
		b.hierarchyNode(n, tint)
	case "sequence":
		b.sequenceNode(n, tint)
	case "canonical":
		b.canonicalNode(n, tint)
	default:
		panic(op)
	}
	solidRecord(r, map[string]any{"state": shapeState(b, false), "node": n})
}
func shapeObserveArrowOpacity(b *meshBuilder, kind string, tip, dir Vec, m *Material, width int, opacity float64) {
	r := shapeRequest(b, "arrow")
	r["kind"], r["tip"], r["dir"], r["material"], r["width"], r["opacity"] = kind, tip, dir, solidMesh([]Triangle{{Material: m}}, true), width, opacity
	b.arrowWithOpacity(kind, tip, dir, m, width, opacity)
	solidRecord(r, shapeState(b, false))
}
func shapeObserveArrow(b *meshBuilder, kind string, tip, dir Vec, m *Material, width ...int) {
	w := 2
	if len(width) > 0 {
		w = width[0]
	}
	opacity := 1.
	if m != nil {
		opacity = float64(m.Color.A) / 255
	}
	shapeObserveArrowOpacity(b, kind, tip, dir, m, w, opacity)
}
func shapeObserveBoard(b *meshBuilder, board d2isometric.Board, n *d2isometric.Node, tint string, opacity float64) {
	r := shapeRequest(b, "board")
	r["board"], r["node"], r["tint"], r["opacity"] = board, n, tint, opacity
	b.hierarchyBoard(board, n, tint, opacity)
	solidRecord(r, shapeState(b, false))
}

var _ = context.Background

// Preserve private animation ranges rather than encoding empty Go structs.
func shapeState(b *meshBuilder, raw bool) any {
	s := solidState(b, raw).(map[string]any)
	var animations []any
	for _, a := range b.animatedNodes {
		animations = append(animations, map[string]any{"first": a.first, "last": a.last})
	}
	s["animated"] = animations
	return s
}
