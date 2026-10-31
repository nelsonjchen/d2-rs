// SPDX-License-Identifier: MPL-2.0
// parseColor over generated CSS color values (recipe mathtext-svg-color).
package d2svgimport

import (
	"encoding/json"
	"os"
	"testing"
)

func TestD2RSColor(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, e := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if e != nil {
		t.Fatal(e)
	}
	var values []string
	if e = json.Unmarshal(data, &values); e != nil {
		t.Fatal(e)
	}
	rows := []map[string]any{}
	for _, value := range values {
		color, e := parseSolidColor(value)
		message := ""
		if e != nil {
			message = e.Error()
		}
		rows = append(rows, map[string]any{"value": value, "rgba": [4]uint8{color.R, color.G, color.B, color.A}, "error": message})
	}
	data, e = json.MarshalIndent(rows, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); e != nil {
		t.Fatal(e)
	}
}
