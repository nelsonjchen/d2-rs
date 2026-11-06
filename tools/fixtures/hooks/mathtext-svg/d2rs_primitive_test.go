// SPDX-License-Identifier: MPL-2.0
// svgGeometry.primitive over geometry x fill x stroke x opacity x width x
// visibility x currentColor (recipe mathtext-svg-primitive).
package d2svgimport

import (
	"context"
	"image/color"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

func TestD2RSSvgPrimitive(t *testing.T) {
	out := d2rsOutput(t)
	paints := []d2scene.Paint{nil, d2scene.SolidPaint{Color: color.NRGBA{R: 12, G: 34, B: 56, A: 255}}, d2scene.SolidPaint{Color: color.NRGBA{R: 12, G: 34, B: 56, A: 0}},
		d2scene.LinearGradient{Start: d2scene.Point{X: 1, Y: 2}, End: d2scene.Point{X: 3, Y: 4}, Units: d2scene.UserSpaceOnUse, Transform: d2scene.Identity(),
			Spread: d2scene.SpreadReflect, Stops: []d2scene.GradientStop{{Offset: 0, Color: color.NRGBA{R: 1, A: 255}}, {Offset: .5, Color: color.NRGBA{G: 2, A: 127}},
				{Offset: 1, Color: color.NRGBA{B: 3, A: 1}}}}}
	geometries := []svgGeometry{{}, {kind: geometryRect, box: d2scene.Box{X: 1, Y: 2, Width: 10, Height: 20}, radiusX: 3, radiusY: 4},
		{kind: geometryRect, box: d2scene.Box{Height: 20}}, {kind: geometryEllipse, center: d2scene.Point{X: 1, Y: 2}, radiusX: 3, radiusY: 4},
		{kind: geometryEllipse, radiusX: 0, radiusY: 4}, {kind: geometryPath}, {kind: geometryPath, commands: []d2scene.PathCommand{d2scene.MoveTo(0, 0), d2scene.LineTo(1, 2)}},
		{kind: geometryPath, forceNoFill: true, commands: []d2scene.PathCommand{d2scene.MoveTo(0, 0), d2scene.LineTo(1, 2)}}}
	rows := []map[string]any{}
	for _, g := range geometries {
		for _, fill := range paints {
			for _, stroke := range paints {
				for _, op := range []float64{0, .5, 1} {
					for _, width := range []float64{0, 2} {
						for _, visible := range []bool{false, true} {
							for _, current := range []bool{false, true} {
								s := svgStyle{visible: visible, fill: fill, stroke: stroke, fillCurrent: current, strokeCurrent: current,
									color: color.NRGBA{R: 9, G: 8, B: 7, A: 129}, fillOpacity: op, strokeOpacity: op, strokeWidth: width,
									lineCap: d2scene.CapRound, lineJoin: d2scene.JoinBevel, miterLimit: 5, dashes: []float64{2, 3}, dashOffset: 1.5, fillRule: d2scene.EvenOdd}
								beforeFill, beforeStroke := d2rsPaint(fill), d2rsPaint(stroke)
								p, count, err := g.primitive(context.Background(), s)
								var output any
								if p != nil {
									var pf d2scene.Paint
									var ps *d2scene.Stroke
									rule := d2scene.NonZero
									actual := g
									switch p := p.(type) {
									case d2scene.Rect:
										actual.box = p.Box
										actual.radiusX, actual.radiusY = p.RadiusX, p.RadiusY
										pf, ps = p.Fill, p.Stroke
									case d2scene.Ellipse:
										actual.center = p.Center
										actual.radiusX, actual.radiusY = p.RadiusX, p.RadiusY
										pf, ps = p.Fill, p.Stroke
									case d2scene.Path:
										actual.commands = p.Commands
										pf, ps = p.Fill, p.Stroke
										rule = p.FillRule
									}
									output = map[string]any{"geometry": d2rsGeometry(actual), "fill": d2rsPaint(pf), "stroke": d2rsStroke(ps), "fill_rule": rule}
								}
								rows = append(rows, map[string]any{"geometry": d2rsGeometry(g), "style": map[string]any{"visible": visible, "fill": beforeFill,
									"stroke": beforeStroke, "fill_current": current, "stroke_current": current, "color": [4]uint8{9, 8, 7, 129}, "fill_opacity": op,
									"stroke_opacity": op, "stroke_width": width, "cap": s.lineCap, "join": s.lineJoin, "miter_limit": s.miterLimit, "dashes": s.dashes,
									"dash_offset": s.dashOffset, "fill_rule": s.fillRule}, "primitive": output, "count": count, "error": d2rsError(err),
									"after_fill": d2rsPaint(fill), "after_stroke": d2rsPaint(stroke)})
							}
						}
					}
				}
			}
		}
	}
	d2rsWrite(t, out, rows)
}
