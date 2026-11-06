// SPDX-License-Identifier: MPL-2.0
// affineInverseIsFinite over extreme matrices (recipe mathtext-svg-inverse).
package d2svgimport

import (
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"os"
	"testing"
)

func TestD2RSGradientInverse(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	for _, a := range []float64{0, 1, 1e-300, 1e300} {
		for _, b := range []float64{0, 1, 1e-300, 1e300} {
			for _, c := range []float64{0, 1, 1e-300, 1e300} {
				for _, d := range []float64{0, 1, 1e-300, 1e300} {
					m := d2scene.Matrix{A: a, B: b, C: c, D: d, E: 1e300, F: 1e-300}
					rows = append(rows, map[string]any{"matrix": [6]float64{a, b, c, d, m.E, m.F}, "finite": affineInverseIsFinite(m)})
				}
			}
		}
	}
	data, e := json.MarshalIndent(rows, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); e != nil {
		t.Fatal(e)
	}
}
