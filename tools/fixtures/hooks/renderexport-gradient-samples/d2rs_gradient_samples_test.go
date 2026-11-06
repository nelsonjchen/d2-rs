// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): prepared gradient inverse matrices and
// per-pixel colorAt samples for generated linear/radial gradient requests
// (scripts/renderexport_inputs.py gradient). Harvested from the v0.9.0-era
// overlay gradient_samples_source.go.overlay; inert unless run by
// scripts/regen-fixtures.py.
package d2raster

import (
	"encoding/json"
	"fmt"
	"image/color"
	"math"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

func TestD2RSGradientSamples(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	input, err := os.Open(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	defer input.Close()
	var cases []struct {
		Name           string      `json:"name"`
		SVG            string      `json:"svg"`
		Kind           string      `json:"kind"`
		Geometry       []float64   `json:"geometry"`
		Matrix         []float64   `json:"matrix"`
		GradientMatrix []float64   `json:"gradient_matrix"`
		Spread         uint8       `json:"spread"`
		Stops          [][]float64 `json:"stops"`
	}
	if err := json.NewDecoder(input).Decode(&cases); err != nil {
		t.Fatal(err)
	}
	rows := []any{}
	matrix := func(v []float64) d2scene.Matrix {
		return d2scene.Matrix{A: v[0], B: v[1], C: v[2], D: v[3], E: v[4], F: v[5]}
	}
	for _, c := range cases {
		stops := []d2scene.GradientStop{}
		for _, s := range c.Stops {
			stops = append(stops, d2scene.GradientStop{Offset: s[0], Color: color.NRGBA{R: uint8(s[1]), G: uint8(s[2]), B: uint8(s[3]), A: uint8(s[4])}})
		}
		var p d2scene.Paint
		g := c.Geometry
		if c.Kind == "linear" {
			p = d2scene.LinearGradient{Start: d2scene.Point{X: g[0], Y: g[1]}, End: d2scene.Point{X: g[2], Y: g[3]}, Stops: stops, Units: d2scene.ObjectBoundingBox, Transform: matrix(c.GradientMatrix), Spread: d2scene.SpreadMethod(c.Spread)}
		} else {
			p = d2scene.RadialGradient{Focal: d2scene.Point{X: g[0], Y: g[1]}, FocalRadius: g[2], Center: d2scene.Point{X: g[3], Y: g[4]}, Radius: g[5], Stops: stops, Units: d2scene.ObjectBoundingBox, Transform: matrix(c.GradientMatrix), Spread: d2scene.SpreadMethod(c.Spread)}
		}
		prepared, err := prepareAnimatedPaintWithPattern(p, nil, d2scene.Box{X: -8, Y: -4, Width: 48, Height: 32}, matrix(c.Matrix), nil)
		if err != nil {
			t.Fatal(c.Name, err)
		}
		samples := []any{}
		for y := 0; y < 32; y++ {
			for x := 0; x < 48; x++ {
				v, ok := prepared.colorAt(float64(x)+.5, float64(y)+.5)
				if !ok {
					samples = append(samples, nil)
				} else {
					samples = append(samples, []int{int(v.R), int(v.G), int(v.B), int(v.A)})
				}
			}
		}
		m := prepared.gradient.deviceToGradient
		bits := []string{}
		for _, v := range []float64{m.A, m.B, m.C, m.D, m.E, m.F} {
			bits = append(bits, fmt.Sprintf("%016x", math.Float64bits(v)))
		}
		rows = append(rows, map[string]any{"name": c.Name, "svg": c.SVG, "samples": samples,
			"inverse": []float64{m.A, m.B, m.C, m.D, m.E, m.F}, "prepared_kind": prepared.kind, "inverse_bits": bits})
	}
	file, err := os.Create(out)
	if err != nil {
		t.Fatal(err)
	}
	defer file.Close()
	if err := json.NewEncoder(file).Encode(rows); err != nil {
		t.Fatal(err)
	}
}
