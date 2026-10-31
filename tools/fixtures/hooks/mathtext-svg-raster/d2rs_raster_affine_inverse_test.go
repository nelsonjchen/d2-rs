// SPDX-License-Identifier: MPL-2.0
// d2raster affine inverse over seed matrices (recipe mathtext-svg-raster-affine-inverse).
package d2raster

import (
	"encoding/json"
	"fmt"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"math"
	"os"
	"strconv"
	"testing"
)

func TestD2RSGradientInverse(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	b, e := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if e != nil {
		t.Fatal(e)
	}
	var inputs [][6]string
	if e = json.Unmarshal(b, &inputs); e != nil {
		t.Fatal(e)
	}
	rows := []any{}
	for _, bits := range inputs {
		var m [6]float64
		for n, raw := range bits {
			v, e := strconv.ParseUint(raw, 16, 64)
			if e != nil {
				t.Fatal(e)
			}
			m[n] = math.Float64frombits(v)
		}
		inv, ok, e := finiteAffineInverse(d2scene.Matrix{A: m[0], B: m[1], C: m[2], D: m[3], E: m[4], F: m[5]})
		err := ""
		if e != nil {
			err = e.Error()
		}
		var output any
		if ok {
			values := [6]float64{inv.A, inv.B, inv.C, inv.D, inv.E, inv.F}
			var out [6]string
			for n, v := range values {
				out[n] = fmt.Sprintf("%016x", math.Float64bits(v))
			}
			output = out
		}
		rows = append(rows, map[string]any{"input": bits, "output": output, "error": err, "invertible": ok})
	}
	b, e = json.Marshal(rows)
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), b, 0644); e != nil {
		t.Fatal(e)
	}
}
