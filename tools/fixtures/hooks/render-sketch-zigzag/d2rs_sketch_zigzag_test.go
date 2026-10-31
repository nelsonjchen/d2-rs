// SPDX-License-Identifier: MPL-2.0
// Oracle for crates/d2-rs-render/tests/fixtures/sketch-zigzag-source-v090.json
// (docs/FIXTURES.md): rough-go zigzag polygon fills compiled to d2sketch scene
// paths over a polygon x seed/roughness grid and a stroke-width sweep.
// Harvested from .reference/sketch-zigzag-20261018/oracle (one case per run,
// cases in primitive-cases.json); the case grid is now built here.
// Overlaid into d2renderers/d2sketch as an external test (public APIs only); inert unless
// D2RS_RECIPE_OUTPUT is set.
package d2sketch_test

import (
	"testing"
	"context"
	"encoding/json"
	"fmt"
	"os"

	"github.com/d2lang/d2/d2renderers/d2sketch"
	rough "github.com/d2lang/rough-go"
)

type d2rsZigzagRequest struct {
	Name      string        `json:"name"`
	Points    []rough.Point `json:"points"`
	Width     float64       `json:"width"`
	Seed      float64       `json:"seed"`
	Roughness float64       `json:"roughness"`
}

func d2rsPoints(values ...float64) []rough.Point {
	out := []rough.Point{}
	for i := 0; i < len(values); i += 2 {
		out = append(out, rough.Point{values[i], values[i+1]})
	}
	return out
}

func d2rsZigzagCases() []d2rsZigzagRequest {
	out := []d2rsZigzagRequest{}
	diamond := d2rsPoints(-20, 0, -10, 5, 0, 0, -10, -5, -20, 0)
	for _, width := range []float64{0, 1, 2, 3, 4, 5, 8, 15} {
		out = append(out, d2rsZigzagRequest{fmt.Sprintf("diamond-width%g", width), diamond, width, 1, 1})
	}
	polygons := []struct {
		name   string
		points []rough.Point
	}{
		{"rectangle", d2rsPoints(0, 0, 80, 0, 80, 45, 0, 45)},
		{"concave", d2rsPoints(0, 0, 80, 0, 80, 60, 40, 30, 0, 60)},
		{"duplicates", d2rsPoints(0, 0, 0, 0, 45, 5, 45, 5, 30, 40, 0, 0)},
		{"collinear", d2rsPoints(0, 0, 10, 10, 20, 20)},
		{"tiny", d2rsPoints(0, 0, 0.01, 0, 0.003, 0.015)},
		{"negative", d2rsPoints(-65, -40, -2, -44, -7, 12, -40, 9)},
		{"near-close", d2rsPoints(10, 10, 80, 10, 80, 55, 10, 55, 10.000000000000002, 10)},
		{"bowtie", d2rsPoints(0, 0, 70, 50, 0, 50, 70, 0)},
	}
	for _, polygon := range polygons {
		for _, variant := range [][2]float64{{3, 0}, {7, 0.5}, {11, 1.5}} {
			out = append(out, d2rsZigzagRequest{fmt.Sprintf("%s-seed%g-rough%g", polygon.name, variant[0], variant[1]), polygon.points, 2, variant[0], variant[1]})
		}
	}
	out = append(out, d2rsZigzagRequest{"empty-zero-gap-skip", []rough.Point{}, 0, 40000, 1})
	out = append(out, d2rsZigzagRequest{"single-point-zero-gap-skip", d2rsPoints(0, 0), 0, 40000, 1})
	return out
}

func TestD2RSSketchZigzag(t *testing.T) {
	outputPath := os.Getenv("D2RS_RECIPE_OUTPUT")
	if outputPath == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	stdout, err := os.Create(outputPath)
	if err != nil {
		t.Fatal(err)
	}
	defer stdout.Close()

	rows := []any{}
	for _, request := range d2rsZigzagCases() {
		generator := rough.NewGenerator(nil)
		points := append([]rough.Point(nil), request.Points...)
		drawable := generator.Polygon(points, &rough.Options{
			Fill: rough.String("#123456"), Stroke: rough.String("#123456"), FillStyle: rough.String("zigzag"),
			FillWeight: rough.Float64(4), StrokeWidth: rough.Float64(request.Width), Seed: rough.Float64(request.Seed), Roughness: rough.Float64(request.Roughness),
		})
		paths, err := d2sketch.CompileScenePaths(context.Background(), drawable, d2sketch.SceneLimits{MaxSets: 16, MaxOps: 100000})
		if err != nil {
			panic(err)
		}
		rows = append(rows, map[string]any{"input": request, "paths": paths, "sets": drawable.Sets, "caller_points_after": points})
	}
	encoded, err := json.Marshal(rows)
	if err != nil {
		panic(err)
	}
	if _, err := stdout.Write(append(encoded, '\n')); err != nil {
		panic(err)
	}
}
