// SPDX-License-Identifier: MPL-2.0
package d2cli

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

// Observe both original direct calls without replacing their assertions.
func d2rsPagedRebase(t *testing.T, links []d2scene.LinkRegion, sourceRoot string, pages map[string]int) ([]d2scene.LinkRegion, error) {
	input := map[string]any{"operation": "rebase", "links": append([]d2scene.LinkRegion(nil), links...), "source_root": sourceRoot, "pages": pages}
	result, err := rebaseIsometricPageLinks(links, sourceRoot, pages)
	message := ""
	if err != nil {
		message = err.Error()
	}
	data, encodeErr := json.Marshal(map[string]any{"test": t.Name(), "call": d2rsModeCall, "input": input,
		"result": map[string]any{"links": result, "input_links": links, "error": message}})
	if encodeErr != nil {
		t.Fatal(encodeErr)
	}
	path := filepath.Join(os.Getenv("D2RS_MODE_CAPTURE"), "calls", fmt.Sprintf("%03d.json", d2rsModeCall))
	d2rsModeCall++
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}
	return result, err
}
