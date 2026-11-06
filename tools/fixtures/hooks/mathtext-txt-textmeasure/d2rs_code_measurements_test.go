// SPDX-License-Identifier: MPL-2.0
// Code-block bounds: Ruler.MeasureMono with CODE_LINE_HEIGHT over Source Code
// Pro regular sizes x code sources (recipe mathtext-txt-chroma-code-measurements).
package textmeasure

import (
	"encoding/json"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
)

func TestD2RSChromaCodeMeasurements(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	ruler, err := NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	rows := []map[string]any{}
	for _, size := range []int{10, 11, 12, 13, 14, 16, 24} {
		for _, source := range []string{"hello", "func f() { return 2 }", "spaces    and tabs", "  before\n    after",
			"var x = 0xAf_12", "\tvar x = 12\n\t\tprintln(x)"} {
			ruler.LineHeightFactor = CODE_LINE_HEIGHT
			w, h := ruler.MeasureMono(d2fonts.SourceCodePro.Font(size, d2fonts.FONT_STYLE_REGULAR), source)
			rows = append(rows, map[string]any{"source": source, "size": size, "width": w, "height": h})
		}
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
