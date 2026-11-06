// SPDX-License-Identifier: MPL-2.0
// Test-only observers; original calls, results and assertions remain active.
package d2svg_test

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/d2target"
)

var d2rsMu sync.Mutex
var d2rsCounts = map[string]int{}

func d2rsRecord(t *testing.T, d json.RawMessage, opts *d2svg.RenderOpts, multi bool, out any, err error) {
	t.Helper()
	root := os.Getenv("D2RS_SVG_ADMISSION_DUMP")
	if root == "" {
		return
	}
	d2rsMu.Lock()
	defer d2rsMu.Unlock()
	n := d2rsCounts[t.Name()]
	d2rsCounts[t.Name()] = n + 1
	name := fmt.Sprintf("%s#%d", t.Name(), n)
	message := ""
	if err != nil {
		message = err.Error()
	}
	row := map[string]any{"name": name, "test": t.Name(), "request": map[string]any{"diagram": d, "opts": opts, "multi": multi}, "got": map[string]any{"output": out, "error": message}}
	data, encodeErr := json.Marshal(row)
	if encodeErr != nil {
		t.Fatal(encodeErr)
	}
	key := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if writeErr := os.WriteFile(filepath.Join(root, key+".json"), data, 0600); writeErr != nil {
		t.Fatal(writeErr)
	}
}
func d2rsRender(t *testing.T, d *d2target.Diagram, opts *d2svg.RenderOpts) ([]byte, error) {
	raw, e := json.Marshal(d)
	if e != nil {
		t.Fatal(e)
	}
	out, err := d2svg.Render(d, opts)
	d2rsRecord(t, raw, opts, false, out, err)
	return out, err
}
func d2rsRenderMultiboard(t *testing.T, d *d2target.Diagram, opts *d2svg.RenderOpts) ([][]byte, error) {
	raw, e := json.Marshal(d)
	if e != nil {
		t.Fatal(e)
	}
	out, err := d2svg.RenderMultiboard(d, opts)
	d2rsRecord(t, raw, opts, true, out, err)
	return out, err
}
