// SPDX-License-Identifier: MPL-2.0
// d2oracle.UpdateImport over the boundary controls of
// tools/fixtures/inputs/mathtext-txt/update-import-boundaries.json (recipe
// mathtext-txt-editor-update-import-boundaries), adapted from
// tools/probes/editor-update-import-v090.
package d2oracle_test

import (
	"encoding/json"
	"os"
	"testing"

	"github.com/d2lang/d2/d2oracle"
)

func TestD2RSUpdateImportBoundaries(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type request struct {
		Name    string  `json:"name"`
		Input   string  `json:"input"`
		OldPath string  `json:"old_path"`
		NewPath *string `json:"new_path"`
		Text    string  `json:"text"`
		Error   string  `json:"error"`
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var rows []request
	if err := json.Unmarshal(data, &rows); err != nil {
		t.Fatal(err)
	}
	for i := range rows {
		text, err := d2oracle.UpdateImport(rows[i].Input, rows[i].OldPath, rows[i].NewPath)
		rows[i].Text, rows[i].Error = text, ""
		if err != nil {
			rows[i].Error = err.Error()
		}
	}
	encoded, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(encoded, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
