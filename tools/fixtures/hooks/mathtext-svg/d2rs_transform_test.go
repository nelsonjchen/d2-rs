// SPDX-License-Identifier: MPL-2.0
// transform attribute parsing x byte/function budgets (recipe mathtext-svg-transform).
package d2svgimport

import (
	"context"
	"encoding/json"
	"math"
	"os"
	"testing"
)

func TestD2RSTransformBudget(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type row struct {
		Value        string      `json:"value"`
		MaxBytes     int         `json:"max_bytes"`
		MaxFunctions int         `json:"max_functions"`
		Matrix       *[6]float64 `json:"matrix"`
		Bits         *[6]uint64  `json:"bits"`
		Count        *int        `json:"count"`
		Error        *string     `json:"error"`
	}
	data, e := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if e != nil {
		t.Fatal(e)
	}
	var rows []row
	if e = json.Unmarshal(data, &rows); e != nil {
		t.Fatal(e)
	}
	for n := range rows {
		r := &rows[n]
		m, count, e := parseTransformWithCount(context.Background(), "math", r.Value, transformLimits{r.MaxBytes, r.MaxFunctions})
		if e != nil {
			s := e.Error()
			r.Error = &s
			continue
		}
		r.Count = &count
		v := [6]float64{m.A, m.B, m.C, m.D, m.E, m.F}
		r.Matrix = &v
		b := [6]uint64{}
		for j := range v {
			b[j] = math.Float64bits(v[j])
		}
		r.Bits = &b
	}
	data, e = json.MarshalIndent(rows, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); e != nil {
		t.Fatal(e)
	}
}
