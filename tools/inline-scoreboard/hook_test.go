// SPDX-License-Identifier: MPL-2.0
// Test-only capture for pinned upstream tests whose expectations are inline.
package PACKAGE

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
)

func d2rsInlineDump(t *testing.T, request interface{}, got interface{}, err error) {
	dir := os.Getenv("D2RS_INLINE_DUMP")
	if dir == "" {
		return
	}
	errText := ""
	if err != nil {
		errText = err.Error()
	}
	row := map[string]interface{}{"name": t.Name(), "request": request, "got": got, "err": errText}
	data, marshalErr := json.Marshal(row)
	if marshalErr != nil {
		t.Fatal(marshalErr)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(t.Name())))
	if writeErr := os.WriteFile(filepath.Join(dir, id+".json"), data, 0644); writeErr != nil {
		t.Fatal(writeErr)
	}
}
