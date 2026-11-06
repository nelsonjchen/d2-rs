// SPDX-License-Identifier: MPL-2.0
// Requalify the compiler witnesses affected by master's grid/gradient changes.
package d2compiler_test

import (
	"encoding/json"
	"os"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2compiler"
)

func TestD2rsMasterDiagnostics(t *testing.T) {
	spec := os.Getenv("D2RS_MASTER_DIAGNOSTIC_CASES")
	if spec == "" {
		return
	}
	var paths [][2]string
	if err := json.Unmarshal([]byte(spec), &paths); err != nil {
		t.Fatal(err)
	}
	for _, pair := range paths {
		data, err := os.ReadFile(pair[0])
		if err != nil {
			t.Fatal(err)
		}
		var container interface{}
		if err := json.Unmarshal(data, &container); err != nil {
			t.Fatal(err)
		}
		rows, array := container.([]interface{})
		if !array {
			object := container.(map[string]interface{})
			rows = object["cases"].([]interface{})
			object["oracle"] = "D2 master " + os.Getenv("D2RS_MASTER_COMMIT")
		}
		for _, value := range rows {
			row := value.(map[string]interface{})
			utf16, _ := row["utf16"].(bool)
			_, _, err := d2compiler.Compile("validation.d2", strings.NewReader(row["source"].(string)), &d2compiler.CompileOptions{UTF16Pos: utf16})
			message := ""
			if err != nil {
				message = err.Error()
			}
			if _, ok := row["errors"]; ok {
				errors := []string{}
				if message != "" {
					errors = strings.Split(message, "\n")
				}
				row["errors"] = errors
			} else {
				row["error"] = message
			}
		}
		data, err = json.MarshalIndent(container, "", "  ")
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(pair[1], append(data, '\n'), 0644); err != nil {
			t.Fatal(err)
		}
	}
}
