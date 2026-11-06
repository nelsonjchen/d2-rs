// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): lib/imageasset SVG admission outcomes for the
// committed hand-authored SVG documents (tools/fixtures/inputs/renderexport/svg-outcomes-*.json;
// runs of 64+ equal characters are stored as ["c", count]). Harvested from the
// v0.9.0-era probes lib/imageasset/{envelope,xml}_contract_probe_test.go; retargeted
// to the manifest pin. Inert unless run by scripts/regen-fixtures.py.
package imageasset

import (
	"context"
	"encoding/json"
	"os"
	"strings"
	"testing"
)

type d2rsSVGRequest struct {
	Name  string            `json:"name"`
	SVG   string            `json:"svg"`
	Parts []json.RawMessage `json:"parts"`
}

func d2rsSVGRequests(t *testing.T) []d2rsSVGRequest {
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var rows []d2rsSVGRequest
	if err := json.Unmarshal(data, &rows); err != nil {
		t.Fatal(err)
	}
	for i := range rows {
		if rows[i].Parts == nil {
			continue
		}
		var b strings.Builder
		for _, part := range rows[i].Parts {
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
		rows[i].SVG = b.String()
	}
	return rows
}

func TestD2RSSVGAssetOutcomes(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	raw := os.Getenv("D2RS_SVG_RAW") != ""
	type row struct {
		Name     string `json:"name"`
		SVG      string `json:"svg"`
		Error    string `json:"error"`
		RawError string `json:"raw_error"`
	}
	type plain struct {
		Name  string `json:"name"`
		SVG   string `json:"svg"`
		Error string `json:"error"`
	}
	rows := []any{}
	for _, r := range d2rsSVGRequests(t) {
		o := row{Name: r.Name, SVG: r.SVG}
		if raw {
			if rawErr := validateSVGWithLimits(context.Background(), []byte(r.SVG), validationLimitsForSVG(4<<20)); rawErr != nil {
				o.RawError = rawErr.Error()
			}
		}
		_, err := probeResource(context.Background(), loadedSource{data: []byte(r.SVG), mimeHint: "image/svg+xml"}, generousLimits(), func(int64, int64) (func(), error) { return func() {}, nil })
		if err != nil {
			o.Error = err.Error()
		}
		if raw {
			rows = append(rows, o)
		} else {
			rows = append(rows, plain{o.Name, o.SVG, o.Error})
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
