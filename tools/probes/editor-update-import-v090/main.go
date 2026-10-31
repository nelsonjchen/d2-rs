// Test oracle only. Run from the pinned instrumentation checkout with shared Go caches:
// go run /Users/nelson/code/d2-rs/tools/probes/editor-update-import-v090/main.go < requests.json
package main

import (
	"encoding/json"
	"os"

	"github.com/d2lang/d2/d2oracle"
)

type request struct {
	Name    string  `json:"name"`
	Input   string  `json:"input"`
	OldPath string  `json:"old_path"`
	NewPath *string `json:"new_path"`
	Text    string  `json:"text"`
	Error   string  `json:"error"`
}

func main() {
	var rows []request
	if err := json.NewDecoder(os.Stdin).Decode(&rows); err != nil {
		panic(err)
	}
	for i := range rows {
		text, err := d2oracle.UpdateImport(rows[i].Input, rows[i].OldPath, rows[i].NewPath)
		rows[i].Text = text
		rows[i].Error = ""
		if err != nil {
			rows[i].Error = err.Error()
		}
	}
	encoder := json.NewEncoder(os.Stdout)
	encoder.SetIndent("", "  ")
	if err := encoder.Encode(rows); err != nil {
		panic(err)
	}
}
