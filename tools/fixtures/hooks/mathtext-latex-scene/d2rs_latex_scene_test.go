// SPDX-License-Identifier: MPL-2.0
// Overlay hooks for d2renderers/d2svgimport (tools/fixtures/mathtext-latex.toml):
// ImportNode over MathJax SVG documents, adapted from the v0.9.0 probe
// tools/probes/mathjax-scene-20261007.go.
//
// TestD2RSMathScene reads $D2RS_RECIPE_INPUT, a list of {expression, svg}
// (MathJax renders and hand-edited variants of them), and writes rows
// {error, expression, scene (when imported), svg}.
//
// TestD2RSLiteralFontScene reads the literal-font d2latex artifact of the
// mathtext-latex-literal-font recipe from $D2RS_FIXTURES_DIR and imports
// the SVG of every row whose origin is a letter-font context, writing
// ordered rows {expression, svg, error}.
package d2svgimport

import (
	"bytes"
	"context"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

var d2rsSceneLimits = Limits{
	MaxBytes:              10000000,
	MaxDepth:              1000,
	MaxElements:           100000,
	MaxAttributes:         100000,
	MaxAttributeBytes:     10000000,
	MaxPathCommands:       1000000,
	MaxTransformFunctions: 100000,
	MaxUseDepth:           100,
	MaxResources:          100000,
}

func d2rsImportScene(svg string) (*Result, string) {
	result, err := ImportNode(context.Background(), "probe", []byte(svg), d2rsSceneLimits)
	if err != nil {
		return nil, strings.TrimPrefix(err.Error(), "d2svgimport: probe: ")
	}
	return result, ""
}

func d2rsWrite(t *testing.T, value any, indent string, escapeHTML bool) {
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(escapeHTML)
	enc.SetIndent("", indent)
	if err := enc.Encode(value); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

func TestD2RSMathScene(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var requests []struct {
		Expression string `json:"expression"`
		SVG        string `json:"svg"`
	}
	if err := json.Unmarshal(data, &requests); err != nil {
		t.Fatal(err)
	}
	rows := []map[string]any{}
	for _, r := range requests {
		row := map[string]any{"expression": r.Expression, "svg": r.SVG}
		result, message := d2rsImportScene(r.SVG)
		row["error"] = message
		if result != nil {
			row["scene"] = result
		}
		rows = append(rows, row)
	}
	d2rsWrite(t, rows, "  ", true)
}

func TestD2RSLiteralFontScene(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	source := filepath.Join(os.Getenv("D2RS_FIXTURES_DIR"), "crates/d2-rs-math/tests/fixtures/literal-font-source-v090.json")
	data, err := os.ReadFile(source)
	if err != nil {
		t.Fatal(err)
	}
	var literal []struct {
		Expression string  `json:"expression"`
		Origin     string  `json:"origin"`
		SVG        *string `json:"svg"`
	}
	if err := json.Unmarshal(data, &literal); err != nil {
		t.Fatal(err)
	}
	contexts := map[string]bool{"literal": true, "normal": true, "script": true, "fraction": true, "named-command": true}
	type row struct {
		Expression string `json:"expression"`
		SVG        string `json:"svg"`
		Error      string `json:"error"`
	}
	rows := []row{}
	for _, r := range literal {
		if !contexts[r.Origin] {
			continue
		}
		if r.SVG == nil {
			t.Fatalf("literal-font row %q has no svg (render error)", r.Expression)
		}
		_, message := d2rsImportScene(*r.SVG)
		rows = append(rows, row{r.Expression, *r.SVG, message})
	}
	d2rsWrite(t, rows, "  ", false)
}
