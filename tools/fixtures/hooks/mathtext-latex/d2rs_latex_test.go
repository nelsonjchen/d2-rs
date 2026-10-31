// SPDX-License-Identifier: MPL-2.0
// Overlay hook for d2renderers/d2latex (tools/fixtures/mathtext-latex.toml).
//
// Reads a JSON list of expressions (strings, or objects with an "expression"
// and passthrough fields such as "origin" or "command") from
// $D2RS_RECIPE_INPUT and writes one row per input to $D2RS_RECIPE_OUTPUT:
// the passthrough fields plus Measure's width/height and Render's svg
// (null when Render fails). Errors are recorded without D2's xdefer
// "latex failed to parse" annotation, i.e. as the admission (ValidateInput)
// or mathjax-go error that d2-rs-math reports.
//
// D2RS_LATEX_ERRORS: "always" adds measure_error/render_error to every row
// (null when absent); "omitempty" adds each only when set.
// D2RS_LATEX_JSON: "indent" or "compact", optionally with "-escape" for Go's
// default HTML escaping.
// D2RS_LATEX_HEADER=1 wraps rows as {upstream_commit, mathjax_go, licenses,
// cases}, with the commit from reference-manifest.json.
package d2latex

import (
	"bytes"
	"encoding/json"
	"errors"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func d2rsPlainError(err error) any {
	if err == nil {
		return nil
	}
	if strings.HasPrefix(err.Error(), "latex failed to parse: ") {
		if inner := errors.Unwrap(err); inner != nil {
			return inner.Error()
		}
	}
	return err.Error()
}

func TestD2RSCaptureLatex(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var inputs []json.RawMessage
	if err := json.Unmarshal(data, &inputs); err != nil {
		t.Fatal(err)
	}
	mode := os.Getenv("D2RS_LATEX_ERRORS")
	if mode != "always" && mode != "omitempty" {
		t.Fatalf("D2RS_LATEX_ERRORS=%q", mode)
	}
	rows := make([]map[string]any, 0, len(inputs))
	for _, raw := range inputs {
		row := map[string]any{}
		var expression string
		if err := json.Unmarshal(raw, &expression); err != nil {
			if err := json.Unmarshal(raw, &row); err != nil {
				t.Fatal(err)
			}
			var ok bool
			if expression, ok = row["expression"].(string); !ok {
				t.Fatalf("input row without expression: %s", raw)
			}
		}
		row["expression"] = expression
		width, height, measureErr := Measure(expression)
		svg, renderErr := Render(expression)
		row["width"], row["height"] = width, height
		if renderErr != nil {
			row["svg"] = nil
		} else {
			row["svg"] = svg
		}
		if mode == "always" || measureErr != nil {
			row["measure_error"] = d2rsPlainError(measureErr)
		}
		if mode == "always" || renderErr != nil {
			row["render_error"] = d2rsPlainError(renderErr)
		}
		rows = append(rows, row)
	}
	var value any = rows
	if os.Getenv("D2RS_LATEX_HEADER") == "1" {
		value = d2rsHeader(t, rows)
	}
	format := os.Getenv("D2RS_LATEX_JSON")
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(strings.HasSuffix(format, "-escape"))
	switch strings.TrimSuffix(format, "-escape") {
	case "indent":
		enc.SetIndent("", "  ")
	case "compact":
	default:
		t.Fatalf("D2RS_LATEX_JSON=%q", format)
	}
	if err := enc.Encode(value); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}

func d2rsHeader(t *testing.T, rows []map[string]any) any {
	manifest, err := os.ReadFile(filepath.Join(os.Getenv("D2RS_REPO_ROOT"), "reference-manifest.json"))
	if err != nil {
		t.Fatal(err)
	}
	var pin struct {
		D2 struct {
			Commit string `json:"commit"`
		} `json:"d2"`
	}
	if err := json.Unmarshal(manifest, &pin); err != nil {
		t.Fatal(err)
	}
	version := ""
	gomod, err := os.ReadFile(filepath.Join(os.Getenv("D2RS_UPSTREAM"), "go.mod"))
	if err != nil {
		t.Fatal(err)
	}
	for _, line := range strings.Split(string(gomod), "\n") {
		if fields := strings.Fields(line); len(fields) >= 2 && fields[0] == "github.com/d2lang/mathjax-go" {
			version = fields[1]
		}
	}
	if pin.D2.Commit == "" || version == "" {
		t.Fatalf("missing pin %q or mathjax-go version %q", pin.D2.Commit, version)
	}
	return struct {
		UpstreamCommit string           `json:"upstream_commit"`
		MathjaxGo      string           `json:"mathjax_go"`
		Licenses       []string         `json:"licenses"`
		Cases          []map[string]any `json:"cases"`
	}{pin.D2.Commit, version, []string{"MPL-2.0 (D2)", "Apache-2.0 (MathJax)"}, rows}
}
