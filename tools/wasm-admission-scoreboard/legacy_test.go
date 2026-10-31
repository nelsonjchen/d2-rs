// SPDX-License-Identifier: MPL-2.0
//go:build js && wasm

package d2wasm

import (
	"encoding/json"
	"os"
	"syscall/js"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/d2target"
)

// Re-evaluate retained v0.9.0 regression inputs at the current pin. This
// synthetic observer is separate from the original-test admission capture.
func TestD2rsRefreshLegacyWASM(t *testing.T) {
	path := os.Getenv("D2RS_WASM_REGRESSION_INPUT")
	if path == "" {
		t.Skip("capture only")
	}
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var inputs []struct {
		Name, Kind, Source string
		Request            json.RawMessage
		Line, Column       int
	}
	if err := json.Unmarshal(data, &inputs); err != nil {
		t.Fatal(err)
	}
	rows := make([]any, 0, len(inputs))
	for _, input := range inputs {
		var got any
		switch input.Kind {
		case "direct":
			var request struct {
				Diagram *d2target.Diagram
				Options *d2svg.RenderOpts
			}
			if err := json.Unmarshal(input.Request, &request); err != nil {
				t.Fatal(err)
			}
			out, err := d2svg.Render(request.Diagram, request.Options)
			message := ""
			if err != nil {
				message = err.Error()
			}
			got = map[string]any{"output": out, "error": message}
		default:
			var call js.Func
			if input.Kind == "ascii" {
				call = wrapWASMCall(Render)
			} else {
				call = wrapWASMCall(GetCompletions)
			}
			var result js.Value
			if input.Kind == "ascii" {
				result = call.Invoke(string(input.Request))
			} else {
				result = call.Invoke(input.Source, input.Line, input.Column)
			}
			got = json.RawMessage(result.String())
			call.Release()
		}
		rows = append(rows, map[string]any{"name": input.Name, "kind": input.Kind, "got": got})
	}
	data, err = json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(os.Getenv("D2RS_WASM_REGRESSION_OUTPUT"), data, 0600); err != nil {
		t.Fatal(err)
	}
}
