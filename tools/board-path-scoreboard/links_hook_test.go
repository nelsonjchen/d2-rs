// SPDX-License-Identifier: MPL-2.0
package d2cli

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2target"
)

var d2rsBoardLinkCalls []any

func d2rsBoardLinkRecord(t *testing.T, input, result any) {
	t.Helper()
	d2rsBoardLinkCalls = append(d2rsBoardLinkCalls, map[string]any{"test": t.Name(), "input": input, "result": result})
	data, err := json.Marshal(d2rsBoardLinkCalls)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_LINK_CAPTURE"), "calls.json"), data, 0600); err != nil {
		t.Fatal(err)
	}
}

func d2rsBoardRelink(t *testing.T, current string, diagram *d2target.Diagram, paths map[string]string) error {
	original, err := json.Marshal(diagram)
	if err != nil {
		t.Fatal(err)
	}
	err = relink(current, diagram, paths)
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsBoardLinkRecord(t, map[string]any{"op": "relink", "current": current, "diagram": json.RawMessage(original), "paths": paths}, map[string]any{"diagram": diagram, "error": message})
	return err
}

func d2rsBoardOutputLink(t *testing.T, path string) string {
	value := boardOutputLink(path)
	d2rsBoardLinkRecord(t, map[string]any{"op": "link", "path": path}, map[string]any{"value": value})
	return value
}
