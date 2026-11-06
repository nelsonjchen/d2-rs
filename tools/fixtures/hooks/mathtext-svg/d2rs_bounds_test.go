// SPDX-License-Identifier: MPL-2.0
// d2scene.PrimitiveBounds and validatePrimitiveBounds over geometry x matrix x
// paint x context (recipe mathtext-svg-bounds).
package d2svgimport

import (
	"image/color"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

func TestD2RSSvgBounds(t *testing.T) {
	out := d2rsOutput(t)
	paths := [][]d2scene.PathCommand{{}, {d2scene.MoveTo(0, 0)}, {d2scene.MoveTo(0, 0), d2scene.LineTo(2, 3)},
		{d2scene.MoveTo(0, 0), d2scene.QuadraticTo(20, 30, 0, 0)}, {d2scene.MoveTo(0, 0), d2scene.CubicTo(20, 30, -20, -30, 0, 0)},
		{d2scene.MoveTo(0, 0), d2scene.ArcTo(10, 20, 0.3, true, true, 2, 3)},
		{d2scene.MoveTo(1e308, 0), d2scene.ArcTo(1e-308, 1e-308, 0, false, true, -1e308, 0)}, {d2scene.LineTo(2, 3)},
		{d2scene.MoveTo(0, 0), d2scene.QuadraticTo(1e308, 1e308, 0, 0)}, {d2scene.MoveTo(0, 0), d2scene.ClosePath()}}
	long := []d2scene.PathCommand{d2scene.MoveTo(1, 2)}
	for n := 0; n < 600; n++ {
		long = append(long, d2scene.LineTo(float64(n), float64(n%7)))
		if n == 255 || n == 511 {
			long = append(long, d2scene.ClosePath())
		}
	}
	paths = append(paths, long)
	geometries := []svgGeometry{{kind: geometryRect, box: d2scene.Box{X: 1, Y: 2, Width: 10, Height: 20}, radiusX: 2, radiusY: 3},
		{kind: geometryRect, box: d2scene.Box{X: 1e308, Y: 1e308, Width: 1e308, Height: 1e308}}, {kind: geometryRect, box: d2scene.Box{Width: -1, Height: 2}},
		{kind: geometryRect, box: d2scene.Box{Width: 2, Height: 3}, radiusX: -1}, {kind: geometryEllipse, center: d2scene.Point{X: 1, Y: 2}, radiusX: 3, radiusY: 4},
		{kind: geometryEllipse, center: d2scene.Point{X: 1e308}, radiusX: 1e308, radiusY: 1e308}, {kind: geometryEllipse, radiusX: -1, radiusY: 1}}
	for _, commands := range paths {
		geometries = append(geometries, svgGeometry{kind: geometryPath, commands: commands})
	}
	matrices := []d2scene.Matrix{d2scene.Identity(), {A: 2, B: 0.5, C: -0.3, D: 3, E: 1, F: 2}, d2scene.Scale(1e308, 1e308),
		d2scene.Scale(1e-308, 1e-308), d2scene.Translate(1e308, -1e308)}
	rows := []map[string]any{}
	for _, g := range geometries {
		for _, matrix := range matrices {
			for _, mode := range []string{"fill", "none", "stroke", "huge-stroke"} {
				var fill d2scene.Paint
				var stroke *d2scene.Stroke
				if mode == "fill" {
					fill = d2scene.SolidPaint{Color: color.NRGBA{A: 255}}
				}
				if mode == "stroke" || mode == "huge-stroke" {
					w := 2.
					if mode == "huge-stroke" {
						w = 1e308
					}
					stroke = &d2scene.Stroke{Paint: d2scene.SolidPaint{Color: color.NRGBA{R: 1, A: 0}}, Width: w, Cap: d2scene.CapSquare, Join: d2scene.JoinMiter, MiterLimit: 4}
				}
				var p d2scene.Primitive
				switch g.kind {
				case geometryRect:
					p = d2scene.Rect{Box: g.box, RadiusX: g.radiusX, RadiusY: g.radiusY, Fill: fill, Stroke: stroke}
				case geometryEllipse:
					p = d2scene.Ellipse{Center: g.center, RadiusX: g.radiusX, RadiusY: g.radiusY, Fill: fill, Stroke: stroke}
				case geometryPath:
					p = d2scene.Path{Commands: g.commands, Fill: fill, Stroke: stroke}
				}
				var st any
				if stroke != nil {
					st = map[string]any{"paint": d2rsPaint(stroke.Paint), "width": stroke.Width, "cap": stroke.Cap, "join": stroke.Join,
						"miter_limit": stroke.MiterLimit, "dashes": nil, "dash_offset": 0}
				}
				bounds, err := d2scene.PrimitiveBounds(p, matrix)
				for _, cancel := range []string{"", "cancelled", "deadline"} {
					validation := validatePrimitiveBounds(d2rsContext(t, cancel), p, matrix)
					rows = append(rows, map[string]any{"geometry": d2rsGeometry(g), "fill": d2rsPaint(fill), "stroke": st,
						"matrix": d2rsMatrix(matrix), "error": d2rsError(err), "finite": bounds.IsFinite(), "valid": bounds.Valid,
						"cancel": cancel, "validation_error": d2rsError(validation)})
				}
			}
		}
	}
	d2rsWrite(t, out, rows)
}
