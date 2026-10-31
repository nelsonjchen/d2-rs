// SPDX-License-Identifier: MPL-2.0
// Test-only subprocess boundary; original board-output assertions stay active.
package d2cli

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"
)

var d2rsBoardCalls = map[string]int{}

func runBoardOutputCLIResult(t *testing.T, directory, inputPath, outputPath string) error {
	t.Helper()
	var err error
	if binary := os.Getenv("D2RS_BOARD_BINARY"); binary != "" {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		cmd := exec.CommandContext(ctx, binary, inputPath, outputPath)
		cmd.Dir = directory
		if out, e := cmd.CombinedOutput(); e != nil {
			err = fmt.Errorf("native command: %v: %s", e, out)
		}
	} else {
		err = d2rsOriginalBoardOutputCLIResult(t, directory, inputPath, outputPath)
	}
	root := os.Getenv("D2RS_BOARD_CAPTURE")
	if root == "" {
		return err
	}
	files := map[string][]byte{}
	owned := map[string][]byte{}
	ownedErrors := map[string]string{}
	walkErr := filepath.WalkDir(directory, func(path string, entry fs.DirEntry, e error) error {
		if e != nil {
			return e
		}
		if entry.IsDir() {
			return nil
		}
		rel, e := filepath.Rel(directory, path)
		if e != nil {
			return e
		}
		data, e := os.ReadFile(path)
		if e == nil {
			files[filepath.ToSlash(rel)] = data
			if entry.Name() == boardOutputManifestName {
				var manifest struct{ Files []struct{ Path string } }
				if e := json.Unmarshal(data, &manifest); e != nil {
					return e
				}
				for _, file := range manifest.Files {
					key := filepath.Join(filepath.Dir(rel), filepath.FromSlash(file.Path))
					b, e := os.ReadFile(filepath.Join(directory, key))
					if e != nil {
						ownedErrors[filepath.ToSlash(key)] = e.Error()
					} else {
						owned[filepath.ToSlash(key)] = b
					}
				}
			}
		}
		return e
	})
	if walkErr != nil {
		t.Fatal(walkErr)
	}
	name := fmt.Sprintf("%s#%d", t.Name(), d2rsBoardCalls[t.Name()])
	d2rsBoardCalls[t.Name()]++
	message := ""
	if err != nil {
		message = err.Error()
	}
	data, e := json.Marshal(map[string]any{"name": name, "test": t.Name(), "files": files, "owned": owned, "ownedErrors": ownedErrors, "error": message})
	if e != nil {
		t.Fatal(e)
	}
	key := fmt.Sprintf("%x", sha256.Sum256([]byte(name)))
	if e := os.WriteFile(filepath.Join(root, key+".json"), data, 0600); e != nil {
		t.Fatal(e)
	}
	return err
}
