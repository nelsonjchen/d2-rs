// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

package routing

import (
	"encoding/json"
	"fmt"
	"math"
	"testing"

	"github.com/d2lang/d2/d2layouts/d2talalayout/internal/labelgeom"
	"github.com/d2lang/d2/lib/geo"
	"github.com/d2lang/d2/lib/label"
)

func geometryMathScalar(value float64) map[string]string {
	return map[string]string{"value": fmt.Sprintf("%.17g", value), "bits": fmt.Sprintf("0x%016x", math.Float64bits(value))}
}

func geometryMathPoint(point *geo.Point) map[string]interface{} {
	return map[string]interface{}{"x": geometryMathScalar(point.X), "y": geometryMathScalar(point.Y)}
}

func geometryMathLog(t *testing.T, record map[string]interface{}) {
	t.Helper()
	encoded, err := json.Marshal(record)
	if err != nil {
		t.Fatalf("encode exact geometry bits: %v", err)
	}
	t.Logf("GEOMETRY_BITS %s", encoded)
}

// Record exact ARM64 Go arithmetic, including degenerate segment selection.
// NaN payloads are retained as evidence, without making payload stability part
// of the mathematical assertions below.
func TestCoincidentLabelGeometryMathWitness(t *testing.T) {
	finiteMixed := geo.Route{
		geo.NewPoint(10.25, -7.75), geo.NewPoint(88.1, 51.9),
		geo.NewPoint(88.1, 143.35), geo.NewPoint(204.125, 143.35),
	}
	trailingDuplicate := geo.Route{geo.NewPoint(10.25, -7.75), geo.NewPoint(88.1, 51.9), geo.NewPoint(88.1, 51.9)}
	for _, test := range []struct {
		name        string
		route       geo.Route
		distances   []float64
		percentages []float64
		nanProbes   map[int]bool
	}{
		{"finite_mixed", finiteMixed, []float64{37.125, finiteMixed.Length() / 2, finiteMixed.Length() + 19.75}, []float64{0.37}, nil},
		{"axis_subnormal", geo.Route{geo.NewPoint(0, 7.25), geo.NewPoint(16*math.SmallestNonzeroFloat64, 7.25)}, []float64{8 * math.SmallestNonzeroFloat64}, []float64{0.5}, nil},
		{"axis_huge", geo.Route{geo.NewPoint(3.5, -1e200), geo.NewPoint(3.5, 1e200)}, []float64{7.5e199}, []float64{0.5}, nil},
		{"coincident_pair", geo.Route{geo.NewPoint(0, 0), geo.NewPoint(0, 0)}, []float64{0, 1}, []float64{0}, map[int]bool{0: true, 1: true}},
		{"leading_duplicate", geo.Route{geo.NewPoint(10.25, -7.75), geo.NewPoint(10.25, -7.75), geo.NewPoint(88.1, 51.9)}, []float64{0, 1}, []float64{0, 0.37}, map[int]bool{0: true}},
		{"trailing_duplicate", trailingDuplicate, []float64{trailingDuplicate.Length(), trailingDuplicate.Length() + 1}, []float64{1, 1.1}, map[int]bool{1: true}},
	} {
		t.Run(test.name, func(t *testing.T) {
			length := test.route.Length()
			if test.name == "axis_subnormal" && length != 16*math.SmallestNonzeroFloat64 {
				t.Fatalf("axis subnormal length = %g, want sixteen minimum subnormals", length)
			}
			if test.name == "axis_huge" && (math.IsInf(length, 0) || length != 2e200) {
				t.Fatalf("axis huge length = %g, want finite 2e200", length)
			}
			points := make([]map[string]interface{}, 0, len(test.route))
			for _, point := range test.route {
				points = append(points, geometryMathPoint(point))
			}
			geometryMathLog(t, map[string]interface{}{"kind": "route", "case": test.name, "points": points, "length": geometryMathScalar(length)})
			for index, distance := range test.distances {
				point, segment := test.route.GetPointAtDistance(distance)
				if test.nanProbes[index] {
					if !math.IsNaN(point.X) || !math.IsNaN(point.Y) {
						t.Fatalf("probe %d = (%g,%g), want both NaN", index, point.X, point.Y)
					}
				} else if math.IsNaN(point.X) || math.IsNaN(point.Y) || math.IsInf(point.X, 0) || math.IsInf(point.Y, 0) {
					t.Fatalf("probe %d = (%g,%g), want finite coordinates", index, point.X, point.Y)
				}
				geometryMathLog(t, map[string]interface{}{"kind": "point_at_distance", "case": test.name, "probe": index, "distance": geometryMathScalar(distance), "point": geometryMathPoint(point), "segment": segment})
			}
			for _, percentage := range test.percentages {
				for _, position := range []label.Position{label.UnlockedTop, label.UnlockedMiddle, label.UnlockedBottom} {
					point, segment := position.GetPointOnRoute(test.route, 3.5, percentage, 20.75, 10.25)
					if point == nil {
						t.Fatalf("%s returned nil", position)
					}
					geometryMathLog(t, map[string]interface{}{"kind": "unlocked_label", "case": test.name, "position": position.String(), "percentage": geometryMathScalar(percentage), "stroke_width": geometryMathScalar(3.5), "width": geometryMathScalar(20.75), "height": geometryMathScalar(10.25), "point": geometryMathPoint(point), "segment": segment})
				}
			}
		})
	}

	t.Run("oblique_arrowheads", func(t *testing.T) {
		route := geo.Route{geo.NewPoint(10.25, -7.75), geo.NewPoint(88.1, 51.9)}
		for _, isTarget := range []bool{false, true} {
			point := labelgeom.ArrowheadTopLeft(route, isTarget, "triangle", "filled-diamond", 20.75, 10.25)
			integerPoint := labelgeom.ArrowheadTopLeft(route, isTarget, "triangle", "filled-diamond", 20, 10)
			if math.IsNaN(point.X) || math.IsNaN(point.Y) || math.IsInf(point.X, 0) || math.IsInf(point.Y, 0) {
				t.Fatalf("target=%v has nonfinite arrowhead label: %v", isTarget, point)
			}
			if math.Float64bits(point.X) != math.Float64bits(integerPoint.X) || math.Float64bits(point.Y) != math.Float64bits(integerPoint.Y) {
				t.Fatalf("target=%v did not truncate decimal dimensions", isTarget)
			}
			geometryMathLog(t, map[string]interface{}{"kind": "arrowhead_label", "case": "oblique_arrowheads", "is_target": isTarget, "source_arrowhead": "triangle", "target_arrowhead": "filled-diamond", "width": geometryMathScalar(20.75), "height": geometryMathScalar(10.25), "point": geometryMathPoint(point)})
		}
	})
}
