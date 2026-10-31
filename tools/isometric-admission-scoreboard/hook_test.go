// SPDX-License-Identifier: MPL-2.0
package d2isometric

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2target"
)

func d2rsAdmissionStart(t *testing.T) {}

func d2rsAdmissionBuild(t *testing.T, d *d2target.Diagram, mutation string, change func(*d2target.Diagram)) (*Scene, error) {
	// Snapshot BEFORE applying the original invalid mutation. JSON cannot
	// represent NaN, Inf or invalid UTF-8; no sanitized input is substituted.
	data, err := json.Marshal(d)
	if err != nil {
		t.Fatal(err)
	}
	request := map[string]interface{}{"op": "validate", "id": 1, "diagram": json.RawMessage(data), "mutation": mutation}
	if change != nil {
		change(d)
	}
	s, err := BuildScene(d, nil)
	message := ""
	if err != nil {
		message = err.Error()
	}
	row := map[string]interface{}{"name": t.Name(), "operations": []interface{}{map[string]interface{}{"request": request, "got": map[string]interface{}{"error": message, "scene_nil": s == nil}}}}
	output, encodeErr := json.Marshal(row)
	if encodeErr != nil {
		t.Fatal(encodeErr)
	}
	if writeErr := os.WriteFile(filepath.Join(os.Getenv("D2RS_SCENE_API"), strings.ReplaceAll(t.Name(), "/", "__")+".json"), output, 0600); writeErr != nil {
		t.Fatal(writeErr)
	}

	return s, err
}
