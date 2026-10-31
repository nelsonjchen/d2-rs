// SPDX-License-Identifier: MPL-2.0
// instantiate: compiled SVG documents to d2scene trees under emitted-node,
// command, resource and use-depth budgets (recipe mathtext-svg-scene).
package d2svgimport

import (
	"context"
	"image/color"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

func d2rsScenePrimitive(p d2scene.Primitive) any {
	if p == nil {
		return nil
	}
	out := map[string]any{}
	var fill d2scene.Paint
	var stroke *d2scene.Stroke
	switch p := p.(type) {
	case d2scene.Image:
		return map[string]any{"kind": "image", "bounds": [4]float64{p.Box.X, p.Box.Y, p.Box.Width, p.Box.Height}, "asset": string(p.Asset),
			"aspect": [2]uint8{uint8(p.Aspect.Align), uint8(p.Aspect.Fit)}}
	case d2scene.Rect:
		out["kind"] = "rect"
		out["bounds"] = [4]float64{p.Box.X, p.Box.Y, p.Box.Width, p.Box.Height}
		out["radii"] = [2]float64{p.RadiusX, p.RadiusY}
		fill, stroke = p.Fill, p.Stroke
	case d2scene.Ellipse:
		out["kind"] = "ellipse"
		out["center"] = [2]float64{p.Center.X, p.Center.Y}
		out["radii"] = [2]float64{p.RadiusX, p.RadiusY}
		fill, stroke = p.Fill, p.Stroke
	case d2scene.Path:
		out["kind"] = "path"
		out["commands"] = d2rsCommands(p.Commands)
		out["fill_rule"] = p.FillRule
		fill, stroke = p.Fill, p.Stroke
	}
	out["fill"] = d2rsPaint(fill)
	out["stroke"] = d2rsStroke(stroke)
	return out
}

func d2rsSceneNode(n *d2scene.Node) any {
	if n == nil {
		return nil
	}
	children := []any{}
	for _, c := range n.Children {
		children = append(children, d2rsSceneNode(c))
	}
	classes := n.Classes
	if classes == nil {
		classes = []string{}
	}
	var clip any
	if n.Clip != nil {
		clip = map[string]any{"commands": d2rsCommands(n.Clip.Path.Commands), "fill_rule": n.Clip.Path.FillRule, "transform": d2rsMatrix(n.Clip.Transform)}
	}
	return map[string]any{"id": n.ID, "classes": classes, "transform": d2rsMatrix(n.Transform), "opacity": n.Opacity, "clip": clip,
		"primitive": d2rsScenePrimitive(n.Primitive), "children": children}
}

func TestD2RSSvgScene(t *testing.T) {
	out := d2rsOutput(t)
	bodies := []string{"", "<g id='g'><rect id='r' class='a' width='10' height='20' rx='2'/><circle id='c' r='3'/><line x2='2' stroke='red'/></g>",
		"<path id='p' d='M0 0L2 3' stroke='blue' fill='none'/>", "<g opacity='0.5'><rect width='2' height='3' fill-opacity='0.5'/></g>",
		"<g visibility='hidden'><rect width='2' height='3'/></g>", "<g display='none'><rect width='2' height='3'/></g>", "<g opacity='0'><rect width='2' height='3'/></g>",
		"<defs><path id='p' d='M0 0L1 1'/></defs><use href='#p'/><use id='u' href='#p' x='3'/>",
		"<defs><g id='p'><rect width='2' height='3'/></g><use id='u' href='#p'/></defs><use id='v' href='#u'/>",
		"<defs><rect id='p' width='2' height='3' display='none'/></defs><use href='#p'/>",
		"<defs><path id='p' d='M0 0L0 0' fill='none'/></defs><use href='#p' stroke='red' stroke-linecap='round'/>",
		"<svg id='n' x='2pt' y='3' width='10' height='20' viewBox='0 0 20 30'><rect width='2' height='3'/></svg>",
		"<svg width='10' height='20' viewBox='0 0 20 30'><svg width='2' height='3' viewBox='0 0 2 3'/></svg>",
		"<clipPath id='c'><rect width='10' height='20'/></clipPath><rect id='r' width='2' height='3' clip-path='url(#c)'/>",
		"<clipPath id='c'><use href='#p'/></clipPath><path id='p' d='M0 0L1 1'/><g clip-path='url(#c)'><rect width='2' height='3'/><use href='#p'/></g>",
		"<defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='0' gradientUnits='userSpaceOnUse'><stop offset='0' stop-color='red'/><stop offset='1' stop-color='blue'/></linearGradient></defs><rect width='2' height='3' fill='url(#g)' fill-opacity='0.5'/>",
		"<rect width='2' height='3' fill='currentColor' stroke='currentColor' stroke-dasharray='2 3' stroke-width='2'/>",
		"<rect width='0' height='3'/><ellipse rx='0' ry='2'/><path d='M0 0'/><title>x</title>"}
	profiles := [][4]int{{100, 100, 100, 100}, {0, 100, 100, 100}, {1, 100, 100, 100}, {2, 100, 100, 100}, {5, 100, 100, 100}, {100, 0, 100, 100},
		{100, 1, 100, 100}, {100, 2, 100, 100}, {100, 5, 100, 100}, {100, 100, 0, 100}, {100, 100, 1, 100}, {100, 100, 2, 100}, {100, 100, 100, 0},
		{100, 100, 100, 1}, {2, 1, 1, 1}}
	rows := []map[string]any{}
	for _, body := range bodies {
		for _, maximum := range profiles {
			for _, used := range []int{0, 1} {
				input := "<svg id='root'><style>.a {fill:red}</style>" + body + "</svg>"
				i := svgImporter{ctx: context.Background(), source: "math", rootCurrentColor: color.NRGBA{R: 1, G: 2, B: 3, A: 200}, ids: map[string]*svgElement{},
					limits: Limits{MaxBytes: 10000, MaxDepth: 100, MaxElements: 100, MaxAttributes: 100, MaxAttributeBytes: 10000, MaxPathCommands: 100,
						MaxTransformFunctions: 100, MaxUseDepth: 100, MaxResources: 100}}
				root, err := i.parse([]byte(input))
				if err == nil {
					err = i.compile(root)
				}
				i.limits.MaxElements, i.limits.MaxPathCommands, i.limits.MaxResources, i.limits.MaxUseDepth = maximum[0], maximum[1], maximum[2], maximum[3]
				i.emittedElements, i.emittedCommands, i.useInstances = used, used, used
				var node *d2scene.Node
				if err == nil {
					node, err = i.instantiate(root, defaultSVGStyleWithColor(i.rootCurrentColor), "pre/", 0, false)
				}
				rows = append(rows, map[string]any{"input": input, "maximum": maximum, "used": used, "error": d2rsError(err), "tree": d2rsSceneNode(node),
					"nodes": i.emittedElements, "commands": i.emittedCommands, "uses": i.useInstances})
			}
		}
	}
	d2rsWrite(t, out, rows)
}
