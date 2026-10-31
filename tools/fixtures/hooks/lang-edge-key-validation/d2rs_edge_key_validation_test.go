// SPDX-License-Identifier: MPL-2.0
// d2-rs fixture hook (recipe lang-edge-key-validation): compiles every
// generated {name, source[, utf16]} row with the public d2compiler.Compile
// and records its complete error text. Inert unless run by
// scripts/regen-fixtures.py.
package d2compiler_test

import (
	"encoding/json"
	"os"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2compiler"
)

func TestD2RSEdgeKeyValidation(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var rows []map[string]any
	if err := json.Unmarshal(data, &rows); err != nil {
		t.Fatal(err)
	}
	for _, row := range rows {
		utf16, _ := row["utf16"].(bool)
		_, _, err := d2compiler.Compile("validation.d2", strings.NewReader(row["source"].(string)), &d2compiler.CompileOptions{UTF16Pos: utf16})
		message := ""
		if err != nil {
			message = err.Error()
		}
		row["error"] = message
	}
	data, err = json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
