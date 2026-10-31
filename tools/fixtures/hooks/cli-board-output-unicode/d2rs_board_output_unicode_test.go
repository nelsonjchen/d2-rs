// SPDX-License-Identifier: MPL-2.0
// Recipe cli-board-output-unicode (tools/fixtures/langlibelk-cli.toml):
// filepath.Clean and boardOutputCollisionKey for path spellings that differ
// only by Unicode case folding, normalization, long combining sequences and
// cleaning. Inert unless run through scripts/regen-fixtures.py.
package d2cli

import (
	"bytes"
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestD2RSBoardOutputUnicode(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	inputs := []string{
		"", ".", "./board", "a/../B", "/a/../../Board", "a//b",
		"é.svg", "é.svg", "Straße.svg", "STRASSE.svg", "Σ.svg", "ς.svg",
		"ﬃ.svg", "İ.svg", "i̇.svg", "ᾲ.svg",
		"x" + strings.Repeat("́", 31), "x" + strings.Repeat("̈́", 16),
		"가" + strings.Repeat("́", 30), strings.Repeat("ᅡ", 31), "x" + strings.Repeat("ཱི", 16),
		"Ɤ.svg", "\U000105d2̇.svg",
	}
	type row struct {
		Clean  string `json:"clean"`
		Input  string `json:"input"`
		Output string `json:"output"`
	}
	rows := []row{}
	for _, input := range inputs {
		rows = append(rows, row{Clean: filepath.Clean(input), Input: input, Output: boardOutputCollisionKey(input)})
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", "  ")
	if err := enc.Encode(rows); err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, buf.Bytes(), 0o644); err != nil {
		t.Fatal(err)
	}
}
