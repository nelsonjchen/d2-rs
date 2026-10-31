// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/color-positions-api.json
// (docs/FIXTURES.md): finite regression witnesses for
// canonicalGradientStopPosition's exponent boundaries and the radial
// counterpart of the original constructed-ID test. Overlaid into lib/color;
// inert unless D2RS_RECIPE_OUTPUT is set. Adapted from
// tools/color-scoreboard/positions/main.go.
package color

import (
	"bytes"
	"encoding/json"
	"os"
	"testing"
)

func TestD2RSColorPositions(t *testing.T) {
	output := os.Getenv("D2RS_RECIPE_OUTPUT")
	if output == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	var requests []struct {
		Op       string   `json:"op"`
		CSS      string   `json:"css"`
		Gradient Gradient `json:"gradient"`
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, &requests); err != nil {
		t.Fatal(err)
	}
	var rows []any
	for _, r := range requests {
		var got any
		rawError := ""
		switch r.Op {
		case "parse":
			g, err := ParseGradient(r.CSS)
			if err != nil {
				rawError = err.Error()
			}
			got = map[string]any{"gradient": g, "error": err != nil}
		case "svg":
			got = GradientToSVG(r.Gradient)
		default:
			t.Fatalf("unknown op %q", r.Op)
		}
		rows = append(rows, map[string]any{"request": r, "got": got, "raw_error": rawError})
	}
	var out bytes.Buffer
	encoder := json.NewEncoder(&out)
	encoder.SetEscapeHTML(false)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(rows); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(output, out.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}
