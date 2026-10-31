// SPDX-License-Identifier: MPL-2.0
//go:build js && wasm

package d2wasm

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"syscall/js"
	"testing"
)

var d2rsCounts = map[string]int{}

func d2rsObserve(t *testing.T, operation string, args []js.Value, data interface{}, err error) {
	t.Helper()
	root := os.Getenv("D2RS_WASM_ADMISSION_DUMP")
	if root == "" {
		return
	}
	response := WASMResponse{Data: data}
	if err != nil {
		response.Data = nil
		if typed, ok := err.(*WASMError); ok {
			response.Error = typed
		} else {
			response.Error = &WASMError{Message: err.Error(), Code: 500}
		}
	} else if operation == "compile" {
		compiled := data.(CompileResponse)
		response.Data = map[string]interface{}{"diagram": compiled.Diagram, "renderOptions": compiled.RenderOptions}
	}
	request, marshalErr := json.Marshal(map[string]interface{}{"operation": operation, "input": json.RawMessage(args[0].String())})
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	n := d2rsCounts[t.Name()]
	d2rsCounts[t.Name()] = n + 1
	name := fmt.Sprintf("%s#%d", t.Name(), n)
	row := map[string]interface{}{"name": name, "test": t.Name(), "request": string(request), "got": response}
	encoded, marshalErr := json.Marshal(row)
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	key := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if writeErr := os.WriteFile(filepath.Join(root, key+".json"), encoded, 0600); writeErr != nil {
		t.Fatal(writeErr)
	}
}

func d2rsCompile(t *testing.T, args []js.Value) (interface{}, error) {
	t.Helper()
	data, err := Compile(args)
	d2rsObserve(t, "compile", args, data, err)
	return data, err
}

func d2rsRender(t *testing.T, args []js.Value) (interface{}, error) {
	t.Helper()
	data, err := Render(args)
	d2rsObserve(t, "render", args, data, err)
	return data, err
}
