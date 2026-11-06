// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): d2svgimport.ImportNode outcomes for the
// committed hand-authored SVG documents (tools/fixtures/inputs/renderexport/svg-outcomes-*.json;
// runs of 64+ equal characters are stored as ["c", count]). Harvested from the
// v0.9.0-era probe d2svgimport/attribute_contract_probe_test.go; retargeted to the
// manifest pin. Inert unless run by scripts/regen-fixtures.py.
package d2svgimport

import (
	"context"
	"encoding/json"
	"os"
	"strings"
	"testing"
)

func TestD2RSSVGImportOutcomes(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type request struct {
		Name  string            `json:"name"`
		SVG   string            `json:"svg"`
		Parts []json.RawMessage `json:"parts"`
	}
	type row struct {
		Name  string `json:"name"`
		SVG   string `json:"svg"`
		Error string `json:"error"`
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var requests []request
	if err := json.Unmarshal(data, &requests); err != nil {
		t.Fatal(err)
	}
	limits := Limits{MaxBytes: 1 << 20, MaxDepth: 64, MaxElements: 1000, MaxAttributes: 10000, MaxAttributeBytes: 1 << 20, MaxPathCommands: 10000, MaxTransformFunctions: 1000, MaxUseDepth: 64, MaxResources: 1000}
	rows := []row{}
	for _, r := range requests {
		svg := r.SVG
		if r.Parts != nil {
			var b strings.Builder
			for _, part := range r.Parts {
				var text string
				if json.Unmarshal(part, &text) == nil {
					b.WriteString(text)
					continue
				}
				var run [2]any
				if err := json.Unmarshal(part, &run); err != nil {
					t.Fatal(err)
				}
				b.WriteString(strings.Repeat(run[0].(string), int(run[1].(float64))))
			}
			svg = b.String()
		}
		o := row{Name: r.Name, SVG: svg}
		if _, err := ImportNode(context.Background(), "probe", []byte(svg), limits); err != nil {
			o.Error = err.Error()
		}
		rows = append(rows, o)
	}
	data, err = json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
