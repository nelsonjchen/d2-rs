// SPDX-License-Identifier: MPL-2.0
// Observe original calls through wrapWASMCall, retaining their assertions.
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

func d2rsInvoke(t *testing.T, call js.Func, args ...any) js.Value {
	t.Helper()
	got := call.Invoke(args...)
	root := os.Getenv("D2RS_WASM_ADMISSION_DUMP")
	if root == "" {
		return got
	}
	n := d2rsCounts[t.Name()]
	d2rsCounts[t.Name()] = n + 1
	name := fmt.Sprintf("%s#%d", t.Name(), n)
	var request any
	if len(args) > 0 {
		request = args[0]
	}
	row := map[string]any{"name": name, "test": t.Name(), "request": request, "got": json.RawMessage(got.String())}
	data, err := json.Marshal(row)
	if err != nil {
		t.Fatal(err)
	}
	key := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if err := os.WriteFile(filepath.Join(root, key+".json"), data, 0600); err != nil {
		t.Fatal(err)
	}
	return got
}
