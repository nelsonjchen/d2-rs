// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/xml-render-text-source-v090.json
// (docs/FIXTURES.md): lib/svg.EscapeText and d2svg.RenderText for valid UTF-8
// strings including XML-invalid scalar controls, at two text origins and line
// heights. Overlaid into d2renderers/d2svg; inert unless D2RS_RECIPE_OUTPUT
// is set.
package d2svg

import (
	"encoding/json"
	"os"
	"testing"

	"github.com/d2lang/d2/lib/svg"
)

func TestD2RSXMLRenderText(t *testing.T) {
	output := os.Getenv("D2RS_RECIPE_OUTPUT")
	if output == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	texts := []string{"", "a&b<c>d\"e'f", "a\rb", "a\tb", "a\nb", "a\r\nb", "\nleading", "trailing\n", "a\n\nb", "\n\n",
		" a  b ", "\x00\x01\x0b\x0c\x1f", "￾￿", "\u0085  ", "é😀", "\U0010ffff"}
	cases := []map[string]any{}
	for _, text := range texts {
		for _, at := range [][2]float64{{0.5, 39.5}, {101.125, 47}} {
			cases = append(cases, map[string]any{"text": text, "x": at[0], "height": at[1],
				"escaped": svg.EscapeText(text), "rendered": RenderText(text, at[0], at[1])})
		}
	}
	data, err := json.MarshalIndent(map[string]any{
		"scope":      "Original lib/svg.EscapeText and d2svg.RenderText;valid UTF8 strings including XML-invalid scalar controls.",
		"cases":         cases,
	}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(output, data, 0o644); err != nil {
		t.Fatal(err)
	}
}
