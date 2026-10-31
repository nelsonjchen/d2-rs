// SPDX-License-Identifier: MPL-2.0
// Test-only capture for the pinned upstream golden contracts.
package PACKAGE_test

import (
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"
)

func d2rsTierDump(t *testing.T, stage string, request interface{}, got interface{}, svg []byte) {
	dir := os.Getenv("D2RS_TIER_DUMP")
	if dir == "" {
		return
	}
	if err := os.MkdirAll(dir, 0755); err != nil {
		t.Fatal(err)
	}
	id := fmt.Sprintf("%x", sha256.Sum256([]byte(t.Name())))
	row := map[string]interface{}{"name": t.Name(), "request": request, "go": got}
	data, err := json.Marshal(row)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(filepath.Join(dir, id+"."+stage+".json"), data, 0644); err != nil {
		t.Fatal(err)
	}
	if svg != nil {
		if err = os.WriteFile(filepath.Join(dir, id+".svg"), svg, 0644); err != nil {
			t.Fatal(err)
		}
	}
}
