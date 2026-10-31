// SPDX-License-Identifier: MPL-2.0
// Test-only observers for pinned lib/color/gradient_test.go. Original assertions
// execute on the original results; only API calls in selected tests are wrapped.
package color

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
	"testing"
)

var d2rsColorMu sync.Mutex
var d2rsColorCounts = map[string]int{}

func d2rsColorRecord(t *testing.T, request, got any, rawErrors ...error) {
	t.Helper()
	root := os.Getenv("D2RS_COLOR_DUMP")
	if root == "" {
		return
	}
	d2rsColorMu.Lock()
	defer d2rsColorMu.Unlock()
	n := d2rsColorCounts[t.Name()]
	d2rsColorCounts[t.Name()] = n + 1
	row := map[string]any{"test": t.Name(), "name": fmt.Sprintf("%s#%d", t.Name(), n), "request": request, "got": got}
	if len(rawErrors) > 0 && rawErrors[0] != nil {
		row["raw_error"] = rawErrors[0].Error()
	}
	b, err := json.Marshal(row)
	if err != nil {
		t.Fatal(err)
	}
	// Subtest slashes are encoded into filenames; the original name stays in JSON.
	key := fmt.Sprintf("%x", sha256.Sum256([]byte(row["name"].(string))))
	if err := os.WriteFile(filepath.Join(root, key+".json"), b, 0600); err != nil {
		t.Fatal(err)
	}
}
func d2rsParseGradient(t *testing.T, css string) (Gradient, error) {
	g, err := ParseGradient(css)
	d2rsColorRecord(t, map[string]any{"op": "parse", "css": css}, map[string]any{"gradient": g, "error": err != nil}, err)
	return g, err
}
func d2rsGradientToSVG(t *testing.T, g Gradient) string {
	got := GradientToSVG(g)
	d2rsColorRecord(t, map[string]any{"op": "svg", "gradient": g}, got)
	return got
}
func d2rsValidColor(t *testing.T, css string) bool {
	got := ValidColor(css)
	d2rsColorRecord(t, map[string]any{"op": "valid", "css": css}, got)
	return got
}
