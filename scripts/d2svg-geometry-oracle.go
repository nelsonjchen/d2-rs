//go:build d2svgoracle

// SPDX-License-Identifier: MPL-2.0
// Test oracle for crates/d2-rs-render/src/d2svg/{geo,label,shape,svgpath}.rs.
// It is compiled *inside* the pinned D2 v0.9.0 module via `go build -overlay`
// (see scripts/d2svg-geometry-fixtures.py) so it calls the original
// lib/geo, lib/label, lib/shape, lib/svg and math packages. Inputs are drawn
// from a fixed-seed generator and written next to the Go outputs; float
// values are serialized as IEEE-754 bit patterns so the Rust test compares
// bit-for-bit. It is never linked into d2-rs.
package main

import (
	"encoding/json"
	"fmt"
	"math"
	"math/rand"
	"os"
	"strings"

	"github.com/d2lang/d2/lib/geo"
	"github.com/d2lang/d2/lib/label"
	"github.com/d2lang/d2/lib/shape"
	"github.com/d2lang/d2/lib/svg"
)

type row map[string]any

func b(f float64) string { return fmt.Sprintf("%016x", math.Float64bits(f)) }

func pt(p *geo.Point) []string {
	if p == nil {
		return nil
	}
	return []string{b(p.X), b(p.Y)}
}

func bx(x *geo.Box) []string {
	if x == nil {
		return nil
	}
	return []string{b(x.TopLeft.X), b(x.TopLeft.Y), b(x.Width), b(x.Height)}
}

var rng = rand.New(rand.NewSource(20261022))

// Indirect calls keep each function's own compiled body under test: when the
// Go compiler inlines these into main it may contract a*b+c*d differently,
// which would test main's code generation rather than lib/geo's.
var (
	euclid       = geo.EuclideanDistance
	unitNormal   = geo.GetUnitNormalVector
	isectPoint   = geo.IntersectionPoint
	interpolate  = (*geo.Point).Interpolate
	distLine     = (*geo.Point).DistanceToLine
	vecLength    = geo.Vector.Length
	vecUnit      = geo.Vector.Unit
	vecAddLength = geo.Vector.AddLength
	vecRadians   = geo.Vector.Radians
	vecDegrees   = geo.Vector.Degrees
	routeLength  = geo.Route.Length
	routeAt      = geo.Route.GetPointAtDistance
	dashAttrs    = svg.GetStrokeDashAttributes
	bezSegment   = svg.BezierCurveSegment
	log10        = math.Log10
)

// coord draws layout-like coordinates: mostly integers, sometimes halves or
// arbitrary fractions, occasionally negative.
func coord(lo, hi float64) float64 {
	v := lo + rng.Float64()*(hi-lo)
	switch rng.Intn(4) {
	case 0, 1:
		return math.Round(v)
	case 2:
		return math.Round(v*2) / 2
	default:
		return v
	}
}

func size() float64 {
	switch rng.Intn(6) {
	case 0:
		return coord(1, 60)
	default:
		return coord(20, 600)
	}
}

func main() {
	out := map[string][]row{}

	// ---------------------------------------------------------------- math
	for i := 0; i < 400; i++ {
		x := (rng.Float64() - 0.5) * math.Pow(10, float64(rng.Intn(8)-3))
		y := (rng.Float64() - 0.5) * math.Pow(10, float64(rng.Intn(8)-3))
		p := math.Abs(x)
		out["math"] = append(out["math"], row{
			"x": b(x), "y": b(y),
			"sin": b(math.Sin(x)), "cos": b(math.Cos(x)), "atan2": b(math.Atan2(y, x)),
			"acos": b(math.Acos(math.Mod(x, 1))), "log10": b(log10(p)), "exp": b(math.Exp(math.Mod(x, 700))),
			"pow2": b(math.Pow(x, 2)), "pow3": b(math.Pow(x, 3)), "pow13": b(math.Pow(p, 1/3.0)),
		})
	}

	// ----------------------------------------------------------------- geo
	for i := 0; i < 400; i++ {
		a := geo.NewPoint(coord(-500, 500), coord(-500, 500))
		c := geo.NewPoint(coord(-500, 500), coord(-500, 500))
		d := geo.NewPoint(coord(-500, 500), coord(-500, 500))
		e := geo.NewPoint(coord(-500, 500), coord(-500, 500))
		t := rng.Float64()*1.4 - 0.2
		v := a.VectorTo(c)
		nx, ny := unitNormal(a.X, a.Y, c.X, c.Y)
		ell := geo.NewEllipse(d, coord(1, 300), coord(1, 300))
		seg := geo.Segment{Start: a, End: c}
		var vertical geo.Segment
		vertical = geo.Segment{Start: geo.NewPoint(a.X, a.Y), End: geo.NewPoint(a.X, c.Y)}
		var ellPts, vellPts []any
		for _, p := range ell.Intersections(seg) {
			ellPts = append(ellPts, pt(p))
		}
		for _, p := range ell.Intersections(vertical) {
			vellPts = append(vellPts, pt(p))
		}
		bez := geo.NewBezierCurve([]*geo.Point{a, c, d, e})
		var bezPts []any
		cut := geo.Segment{Start: geo.NewPoint(coord(-500, 500), coord(-500, 500)), End: geo.NewPoint(coord(-500, 500), coord(-500, 500))}
		for _, p := range bez.Intersections(cut) {
			bezPts = append(bezPts, pt(p))
		}
		route := geo.Route{a, c, d, e}
		dist := rng.Float64() * routeLength(route) * 1.2
		rp, ri := routeAt(route, dist)
		out["geo"] = append(out["geo"], row{
			"a": pt(a), "c": pt(c), "d": pt(d), "e": pt(e), "t": b(t),
			"euclid":       b(euclid(a.X, a.Y, c.X, c.Y)),
			"interp":       pt(interpolate(a, c, t)),
			"len":          b(vecLength(v)),
			"unit":         []string{b(vecUnit(v)[0]), b(vecUnit(v)[1])},
			"add_length":   []string{b(vecAddLength(v, t*100)[0]), b(vecAddLength(v, t*100)[1])},
			"radians":      b(vecRadians(v)),
			"degrees":      b(vecDegrees(v)),
			"normal":       []string{b(nx), b(ny)},
			"isect":        pt(isectPoint(a, c, d, e)),
			"dist_line":    b(distLine(e, a, c)),
			"ell":          []string{b(ell.Rx), b(ell.Ry)},
			"ell_pts":      ellPts,
			"vell_pts":     vellPts,
			"cut":          []any{pt(cut.Start), pt(cut.End)},
			"bez_pts":      bezPts,
			"bez_at":       pt(bez.At(math.Abs(t))),
			"route_dist":   b(dist),
			"route_pt":     pt(rp),
			"route_idx":    ri,
			"route_length": b(routeLength(route)),
		})
	}

	// --------------------------------------------------------------- label
	for i := 0; i < 300; i++ {
		box := geo.NewBox(geo.NewPoint(coord(-300, 300), coord(-300, 300)), size(), size())
		w, h := coord(0, 200), coord(0, 60)
		pad := []float64{label.PADDING, 0, 10, coord(0, 20)}[rng.Intn(4)]
		pos := label.Position(rng.Intn(int(label.UnlockedBottom) + 1))
		n := 2 + rng.Intn(4)
		var route geo.Route
		var routeRow []any
		for j := 0; j < n; j++ {
			p := geo.NewPoint(coord(-500, 500), coord(-500, 500))
			if j > 0 && rng.Intn(2) == 0 {
				// orthogonal segment, as layouts produce
				if rng.Intn(2) == 0 {
					p.X = route[j-1].X
				} else {
					p.Y = route[j-1].Y
				}
			}
			route = append(route, p)
			routeRow = append(routeRow, pt(p))
		}
		stroke := []float64{1, 2, 3, coord(0, 10)}[rng.Intn(4)]
		perc := rng.Float64()
		rp, ri := pos.GetPointOnRoute(route, stroke, perc, w, h)
		out["label"] = append(out["label"], row{
			"pos": pos.String(), "box": bx(box), "w": b(w), "h": b(h), "pad": b(pad),
			"on_box": pt(pos.GetPointOnBox(box, pad, w, h)), "mirrored": pos.Mirrored().String(),
			"route": routeRow, "stroke": b(stroke), "perc": b(perc), "on_route": pt(rp), "route_idx": ri,
		})
	}

	// ----------------------------------------------------------------- svg
	for i := 0; i < 200; i++ {
		sw := coord(0, 15)
		gap := coord(0, 12)
		ds, gs := dashAttrs(sw, gap)
		p := []*geo.Point{geo.NewPoint(coord(-500, 500), coord(-500, 500)), geo.NewPoint(coord(-500, 500), coord(-500, 500)), geo.NewPoint(coord(-500, 500), coord(-500, 500)), geo.NewPoint(coord(-500, 500), coord(-500, 500))}
		t0 := rng.Float64()
		t1 := t0 + (1-t0)*rng.Float64()
		q1, q2, q3, q4 := bezSegment(p[0], p[1], p[2], p[3], t0, t1)
		// a path in the shape of d2svg's connection paths
		var sb strings.Builder
		fmt.Fprintf(&sb, "M %s %s", svg.FormatFloat(p[0].X), svg.FormatFloat(p[0].Y))
		segs := 1 + rng.Intn(4)
		for j := 0; j < segs; j++ {
			switch rng.Intn(3) {
			case 0:
				fmt.Fprintf(&sb, " L %s %s", svg.FormatFloat(coord(-500, 500)), svg.FormatFloat(coord(-500, 500)))
			case 1:
				fmt.Fprintf(&sb, " C %s %s %s %s %s %s", svg.FormatFloat(coord(-500, 500)), svg.FormatFloat(coord(-500, 500)), svg.FormatFloat(coord(-500, 500)), svg.FormatFloat(coord(-500, 500)), svg.FormatFloat(coord(-500, 500)), svg.FormatFloat(coord(-500, 500)))
			default:
				fmt.Fprintf(&sb, " S %s %s %s %s", svg.FormatFloat(coord(-500, 500)), svg.FormatFloat(coord(-500, 500)), svg.FormatFloat(coord(-500, 500)), svg.FormatFloat(coord(-500, 500)))
			}
		}
		perc := rng.Float64()
		p1, p2, err := svg.SplitPath(sb.String(), perc)
		errs := ""
		if err != nil {
			errs = err.Error()
		}
		out["svg"] = append(out["svg"], row{
			"sw": b(sw), "gap": b(gap), "dash": []string{b(ds), b(gs)},
			"p": []any{pt(p[0]), pt(p[1]), pt(p[2]), pt(p[3])}, "t0": b(t0), "t1": b(t1),
			"seg": []any{pt(&q1), pt(&q2), pt(&q3), pt(&q4)},
			"path": sb.String(), "perc": b(perc), "split": []string{p1, p2}, "err": errs,
		})
	}

	// --------------------------------------------------------------- shape
	types := []string{
		shape.SQUARE_TYPE, shape.REAL_SQUARE_TYPE, shape.PARALLELOGRAM_TYPE, shape.DOCUMENT_TYPE,
		shape.CYLINDER_TYPE, shape.QUEUE_TYPE, shape.PAGE_TYPE, shape.PACKAGE_TYPE, shape.STEP_TYPE,
		shape.CALLOUT_TYPE, shape.STORED_DATA_TYPE, shape.PERSON_TYPE, shape.C4_PERSON_TYPE,
		shape.DIAMOND_TYPE, shape.OVAL_TYPE, shape.CIRCLE_TYPE, shape.HEXAGON_TYPE, shape.CLOUD_TYPE,
		shape.TABLE_TYPE, shape.CLASS_TYPE, shape.TEXT_TYPE, shape.CODE_TYPE, shape.IMAGE_TYPE, "", "Bogus",
	}
	for _, typ := range types {
		for i := 0; i < 40; i++ {
			box := geo.NewBox(geo.NewPoint(coord(-300, 300), coord(-300, 300)), size(), size())
			s := shape.NewShape(typ, box)
			ar := 0.0
			if typ == shape.CLOUD_TYPE && rng.Intn(2) == 0 {
				ar = rng.Float64() * 3
				s.SetInnerBoxAspectRatio(ar)
			}
			w, h := coord(0, 400), coord(0, 200)
			px, py := s.GetDefaultPadding()
			if rng.Intn(2) == 0 {
				px, py = coord(0, 60), coord(0, 60)
			}
			ip := s.GetInsidePlacement(w, h, px, py)
			dw, dh := s.GetDimensionsToFit(w, h, px, py)
			dpx, dpy := s.GetDefaultPadding()
			center := box.Center()
			corner := geo.NewPoint(box.TopLeft.X+box.Width, box.TopLeft.Y)
			if rng.Intn(2) == 0 {
				center.Y = box.TopLeft.Y
			}
			v1 := center.VectorTo(corner)
			trace := shape.TraceToShapeBorder(s, corner.Copy(), corner.AddVector(v1))
			out["shape"] = append(out["shape"], row{
				"type": typ, "box": bx(box), "ar": b(ar), "w": b(w), "h": b(h), "px": b(px), "py": b(py),
				"inner": bx(s.GetInnerBox()), "content_box": bx(s.GetInnerBoxForContent(w, h)),
				"inside": pt(&ip), "fit": []string{b(dw), b(dh)}, "default_padding": []string{b(dpx), b(dpy)},
				"paths": s.GetSVGPathData(), "rect": s.IsRectangular(), "ar1": s.AspectRatio1(),
				"center": pt(center), "corner": pt(corner), "trace": pt(trace),
			})
		}
	}
	for i := 0; i < 100; i++ {
		w, h, ar := coord(1, 900), coord(1, 900), []float64{1.5, 3, coord(0.1, 4)}[rng.Intn(3)]
		lw, lh := shape.LimitAR(w, h, ar)
		out["limit_ar"] = append(out["limit_ar"], row{"w": b(w), "h": b(h), "ar": b(ar), "out": []string{b(lw), b(lh)}})
	}

	enc := json.NewEncoder(os.Stdout)
	if err := enc.Encode(out); err != nil {
		panic(err)
	}
}
