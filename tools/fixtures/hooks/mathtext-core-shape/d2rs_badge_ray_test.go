// SPDX-License-Identifier: MPL-2.0
// Badge ray/segment intersection geometry (recipe mathtext-core-badge-ray).
package shape

import (
	"encoding/json"
	"github.com/d2lang/d2/lib/geo"
	"math"
	"os"
	"testing"
)

func TestD2RSBadgeRay(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	box := geo.NewBox(geo.NewPoint(0, 0), 10, 10)
	corner := geo.NewPoint(10, 0)
	previous := geo.NewPoint(15, -5)
	v := previous.VectorTo(corner)
	extended := v.AddLength(10)
	ray := geo.Segment{Start: previous, End: previous.AddVector(extended)}
	s := NewShape(DIAMOND_TYPE, box)
	segments := []map[string]any{}
	for _, p := range s.Perimeter() {
		row := map[string]any{"intersections": p.Intersections(ray)}
		if line, ok := p.(*geo.Segment); ok {
			udx := line.End.X - line.Start.X
			vdx := ray.End.X - ray.Start.X
			uvdx := ray.Start.X - line.Start.X
			udy := line.End.Y - line.Start.Y
			vdy := ray.End.Y - ray.Start.Y
			uvdy := ray.Start.Y - line.Start.Y
			denom := udy*vdx - udx*vdy
			num := vdx*uvdy - vdy*uvdx
			ratio := num / denom
			row["line"] = line
			row["denom"] = math.Float64bits(denom)
			row["numerator"] = math.Float64bits(num)
			row["ratio"] = math.Float64bits(ratio)
			row["x_step"] = math.Float64bits(ratio * udx)
			row["y_step"] = math.Float64bits(ratio * udy)
		}
		segments = append(segments, row)
	}
	out := map[string]any{"vector": v, "length_bits": math.Float64bits(v.Length()), "unit": v.Unit(), "extended": extended, "ray": ray, "segments": segments, "point": TraceToShapeBorder(s, corner, previous)}
	b, err := json.MarshalIndent(out, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), b, 0644); err != nil {
		t.Fatal(err)
	}
}
