// SPDX-License-Identifier: MPL-2.0
// Oracle hook for crates/d2-rs-render/tests/fixtures/markdown-list-width-source-v090.json
// (docs/FIXTURES.md): public LayoutMarkdown (no mono family) and
// RenderMarkdown per snippet x family x size, from the seed
// tools/fixtures/seeds/render-markdown-flow.json. Overlaid into
// lib/textmeasure; inert unless D2RS_RECIPE_INPUT is set. Harvested from the
// v0.9.0-era markdown_flow_probe / markdown_html_case_probe overlays (sets
// with "errors" record the layout error instead of failing).
package textmeasure

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
)

func TestD2RSMarkdownFlow(t *testing.T) {
	input := os.Getenv("D2RS_RECIPE_INPUT")
	if input == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	var seed struct {
		Families []string `json:"families"`
		Sizes    []int    `json:"sizes"`
		Sets     []struct {
			Output  string   `json:"output"`
			Prefix  string   `json:"prefix"`
			Errors  bool     `json:"errors"`
			Sources []string `json:"sources"`
		} `json:"sets"`
	}
	data, err := os.ReadFile(input)
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, &seed); err != nil {
		t.Fatal(err)
	}
	stage := os.Getenv("D2RS_RECIPE_STAGE")
	r, err := NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	for _, set := range seed.Sets {
		rows := []any{}
		for _, name := range seed.Families {
			for _, size := range seed.Sizes {
				for i, source := range set.Sources {
					family := d2fonts.FontFamily(name)
					l, err := LayoutMarkdown(source, r, &family, nil, size)
					if err != nil && !set.Errors {
						t.Fatal(err)
					}
					html, err2 := RenderMarkdown(source)
					if err2 != nil {
						t.Fatal(err2)
					}
					row := map[string]any{"html": html, "name": fmt.Sprintf("%s%s-%d-%d", set.Prefix, name, size, i), "source": source, "family": name, "size": size, "layout": l}
					if set.Errors {
						row["error"] = ""
						if err != nil {
							row["error"] = err.Error()
						}
					}
					rows = append(rows, row)
				}
			}
		}
		out, err := json.Marshal(rows)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(stage, set.Output), append(out, '\n'), 0o644); err != nil {
			t.Fatal(err)
		}
	}
}
