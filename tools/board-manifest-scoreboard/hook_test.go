// SPDX-License-Identifier: MPL-2.0
package d2cli

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

var d2rsManifestCalls []any

func d2rsValidateManifest(root string, manifest boardOutputManifest) error {
	err := validateBoardOutputManifest(root, manifest)
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsManifestCalls = append(d2rsManifestCalls, map[string]any{"manifest": manifest, "error": message})
	return err
}

func TestD2rsManifestCapture(t *testing.T) {
	if os.Getenv("D2RS_MANIFEST_CAPTURE") == "" {
		t.Skip("capture disabled")
	}
	t.Run("TestValidateBoardOutputManifestRejectsUnsafeOwnershipClaims", TestValidateBoardOutputManifestRejectsUnsafeOwnershipClaims)
	originalCount := len(d2rsManifestCalls)
	digest := strings.Repeat("0", 64)
	for _, test := range []struct{ files, dirs []string }{
		{}, {files: []string{"ok.svg"}}, {dirs: []string{"folder"}},
		{files: []string{"same", "same"}}, {files: []string{"Straße", "STRASSE"}},
		{files: []string{"é", "e\u0301"}}, {files: []string{"file"}, dirs: []string{"file"}},
		{files: []string{"file/child", "file"}}, {files: []string{"file"}, dirs: []string{"file/sub/deep"}},
		{files: []string{"a/../b"}}, {files: []string{"../b"}}, {files: []string{".d2-board-output-manifest-v1.json"}},
		{files: []string{"normal.svg"}, dirs: []string{"nested/dir"}},
	} {
		m := boardOutputManifest{Format: boardOutputManifestFormat, Version: boardOutputManifestVersion}
		for _, p := range test.files {
			m.Files = append(m.Files, boardOutputManifestFile{Path: p, SHA256: digest})
		}
		m.Directories = test.dirs
		d2rsValidateManifest(t.TempDir(), m)
	}
	data, err := json.Marshal(map[string]any{"originalCount": originalCount, "calls": d2rsManifestCalls})
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_MANIFEST_CAPTURE"), "calls.json"), data, 0600); err != nil {
		t.Fatal(err)
	}
	fmt.Println("manifest observations", len(d2rsManifestCalls))
}
