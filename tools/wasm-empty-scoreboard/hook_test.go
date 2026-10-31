// SPDX-License-Identifier: MPL-2.0
//go:build js && wasm

package d2wasm

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
)

// Synthetic entry-point observer, not an original Test-function coverage claim.
func TestD2rsWASMEmptyGoldens(t *testing.T) {
	root := os.Getenv("D2RS_WASM_ADMISSION_DUMP")
	raw, err := os.ReadFile(filepath.Join(filepath.Dir(root), "inputs.json"))
	if err != nil {
		t.Fatal(err)
	}
	var inputs []struct {
		Name    string
		Diagram json.RawMessage
	}
	if err := json.Unmarshal(raw, &inputs); err != nil {
		t.Fatal(err)
	}
	call := wrapWASMCall(Render)
	defer call.Release()
	for index, input := range inputs {
		request, err := json.Marshal(map[string]any{"diagram": input.Diagram, "options": map[string]any{"pad": 0, "themeID": 0, "sketch": false}})
		if err != nil {
			t.Fatal(err)
		}
		got := call.Invoke(string(request))
		var response WASMResponse
		if err := json.Unmarshal([]byte(got.String()), &response); err != nil {
			t.Fatal(err)
		}
		if response.Error != nil {
			t.Fatal(response.Error)
		}
		name := fmt.Sprintf("%s#%d", t.Name(), index)
		record := map[string]any{"name": name, "test": t.Name(), "request": string(request), "got": json.RawMessage(got.String()), "board": input.Name}
		data, err := json.Marshal(record)
		if err != nil {
			t.Fatal(err)
		}
		key := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
		if err := os.WriteFile(filepath.Join(root, key+".json"), data, 0600); err != nil {
			t.Fatal(err)
		}
	}
}
