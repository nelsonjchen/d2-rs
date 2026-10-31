// SPDX-License-Identifier: MPL-2.0
// Number/length/interval parsing over generated values (recipe mathtext-svg-length).
package d2svgimport

import (
	"encoding/json"
	"math"
	"os"
	"testing"
)

func TestD2RSSVGLength(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type row struct {
		Value string `json:"value"`
		Kind  string `json:"kind"`
		Allow bool   `json:"allow_units"`
		Bits  uint64 `json:"bits"`
		Error string `json:"error"`
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
		var value float64
		switch r.Kind {
		case "number":
			value, e = parseSVGNumber(r.Value)
		case "length":
			value, e = parseSVGLength(r.Value, r.Allow)
		case "ex":
			value, e = parseMathJaxEXLength(r.Value)
		case "interval":
			value, e = parseUnitInterval(r.Value)
		}
		if e != nil {
			r.Error = e.Error()
		} else {
			r.Bits = math.Float64bits(value)
		}
	}
	data, e = json.MarshalIndent(rows, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); e != nil {
		t.Fatal(e)
	}
}
