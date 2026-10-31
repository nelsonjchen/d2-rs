// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/{code-input-admission,
// latex-input-admission,hidden-math-order}-v090.json (docs/FIXTURES.md):
// d2scenebuild code-label and LaTeX-input validation over unit x repeat /
// font-size / language boundary grids, and the Build error order of a hidden
// or visible LaTeX connection next to a code shape. Overlaid into
// d2renderers/d2scenebuild; inert unless D2RS_RECIPE_OUTPUT is set. Harvested
// from the v0.9.0-era codex_{code_input_admission,latex_input_admission,
// hidden_math_order}_test.go; their request lists are generated inline.
package d2scenebuild

import (
	"bytes"
	"context"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"testing"

	"github.com/d2lang/d2/d2target"
)

type d2rsAdmissionRequest struct {
	Name     string  `json:"name"`
	Object   string  `json:"object"`
	UnitHex  string  `json:"unit_hex"`
	Repeat   int     `json:"repeat"`
	Language *string `json:"language,omitempty"`
	FontSize *int    `json:"font_size,omitempty"`
}

func d2rsWriteJSON(t *testing.T, value any) {
	data, err := json.MarshalIndent(value, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0o644); err != nil {
		t.Fatal(err)
	}
}

func d2rsLabel(t *testing.T, r d2rsAdmissionRequest) string {
	unit, err := hex.DecodeString(r.UnitHex)
	if err != nil {
		t.Fatal(err)
	}
	return string(bytes.Repeat(unit, r.Repeat))
}

func TestD2RSCodeInputAdmission(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	request := func(name, object, unit string, repeat int, language string, font int) d2rsAdmissionRequest {
		return d2rsAdmissionRequest{Name: name, Object: object, UnitHex: unit, Repeat: repeat, Language: &language, FontSize: &font}
	}
	requests := []d2rsAdmissionRequest{}
	for _, unit := range []string{"61", "ceb1", "f09f9880", "61cc81", "0d0a", "ff"} {
		for _, repeat := range []int{0, 1, 99999, 100000, 100001} {
			requests = append(requests, request(fmt.Sprintf("%s-%d", unit, repeat), `shape "n"`, unit, repeat, "go", 16))
		}
	}
	for _, repeat := range []int{1048575, 1048576, 1048577} {
		for _, unit := range []string{"61", "ff"} {
			requests = append(requests, request(fmt.Sprintf("bytes-%s-%d", unit, repeat), `shape "n"`, unit, repeat, "go", 0))
		}
	}
	for _, font := range []int{-1, 0, 1, 33554431, 33554432, 4294967295} {
		for _, c := range []struct {
			unit     string
			repeat   int
			language string
		}{{"61", 1, "go"}, {"61", 0, "go"}, {"ff", 1, ""}} {
			requests = append(requests, request(fmt.Sprintf("font-%d-%s-%d-%s", font, c.unit, c.repeat, c.language), `connection "edge"`, c.unit, c.repeat, c.language, font))
		}
	}
	for _, language := range []string{"javascript", "json", "rust", "MD", "markdown", "latex", ""} {
		requests = append(requests, request("language-"+language, `shape "n"`, "61", 100001, language, 0))
	}
	results := []map[string]any{}
	for _, r := range requests {
		err := validateCodeText(r.Object, d2target.Text{Label: d2rsLabel(t, r), Language: *r.Language, FontSize: *r.FontSize})
		message := ""
		if err != nil {
			message = err.Error()
		}
		results = append(results, map[string]any{"request": r, "error": message})
	}
	d2rsWriteJSON(t, results)
}

func TestD2RSLatexInputAdmission(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	requests := []d2rsAdmissionRequest{}
	for _, unit := range []string{"61", "ceb1", "f09f9880", "ff"} {
		for _, repeat := range []int{0, 1, 1024, 2048, 4095, 4096, 4097} {
			requests = append(requests, d2rsAdmissionRequest{Name: fmt.Sprintf("%s-%d", unit, repeat), Object: `shape "n"`, UnitHex: unit, Repeat: repeat})
		}
	}
	requests = append(requests,
		d2rsAdmissionRequest{Name: "connection-multibyte", Object: `connection "edge"`, UnitHex: "ceb1", Repeat: 2049},
		d2rsAdmissionRequest{Name: "utf8-before-size", Object: `shape "n"`, UnitHex: "61ff", Repeat: 4097})
	b := builder{}
	out := []map[string]any{}
	for _, r := range requests {
		err := b.validateLatexInput(r.Object, d2rsLabel(t, r))
		message := ""
		if err != nil {
			message = err.Error()
		}
		out = append(out, map[string]any{"request": r, "error": message})
	}
	d2rsWriteJSON(t, out)
}

func TestD2RSHiddenMathOrder(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	out := []map[string]any{}
	for _, opacity := range []float64{0, 1} {
		for _, label := range []string{"AV", "🙂"} {
			for _, formula := range []string{`x`, `\href{https://example.com}{x}`, `\badcommand`} {
				d := validDiagram()
				d.Shapes[0].Type = d2target.ShapeCode
				d.Shapes[0].Text = d2target.Text{Label: label, Language: "MD", FontSize: 16, LabelWidth: 100, LabelHeight: 16}
				d.Shapes[0].Width = 100
				d.Shapes[0].Height = 100
				c := &d.Connections[0]
				c.ID = "(a -- b)[0]"
				c.Opacity = opacity
				c.Text = d2target.Text{Label: formula, Language: "latex", FontSize: 16, LabelWidth: 20, LabelHeight: 20}
				c.LabelPosition = "INSIDE_MIDDLE_CENTER"
				c.LabelPercentage = .5
				c.Color = "#000000"
				_, err := Build(context.Background(), d, Options{Assets: testAssetOptions(t)})
				message := ""
				if err != nil {
					message = err.Error()
				}
				out = append(out, map[string]any{"opacity": opacity, "code": label, "formula": formula, "error": message})
			}
		}
	}
	d2rsWriteJSON(t, out)
}
