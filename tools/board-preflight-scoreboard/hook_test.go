// SPDX-License-Identifier: MPL-2.0
// Test-only boundary: original CLI tests retain their assertions.
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
	"strings"
	"testing"
	"time"

	"github.com/d2lang/util-go/xmain"
)

var d2rsPreflightRunError error

func d2rsPreflightRun(ctx context.Context, state *xmain.State) error {
	err := Run(ctx, state)
	d2rsPreflightRunError = err
	return err
}

func d2rsPreflightTempDir(t *testing.T) string {
	t.Helper()
	path := filepath.Join(os.Getenv("D2RS_PREFLIGHT_WORK"), fmt.Sprintf("%x", sha256.Sum256([]byte(t.Name()))))
	if err := os.Mkdir(path, 0700); err != nil {
		t.Fatal(err)
	}
	t.Cleanup(func() {
		if err := os.RemoveAll(path); err != nil {
			t.Error(err)
		}
	})
	return path
}

func d2rsPreflightInventory(t *testing.T, directory string) map[string]any {
	t.Helper()
	nodes := map[string]any{}
	err := filepath.WalkDir(directory, func(path string, entry fs.DirEntry, e error) error {
		if e != nil {
			return e
		}
		rel, e := filepath.Rel(directory, path)
		if e != nil {
			return e
		}
		info, e := entry.Info()
		if e != nil {
			return e
		}
		node := map[string]any{"mode": uint32(info.Mode().Perm())}
		switch {
		case entry.Type()&os.ModeSymlink != 0:
			link, e := os.Readlink(path)
			if e != nil {
				return e
			}
			node["type"], node["link"] = "symlink", link
		case entry.IsDir():
			node["type"] = "directory"
		case info.Mode().IsRegular():
			data, e := os.ReadFile(path)
			if e != nil {
				return e
			}
			node["type"], node["bytes"] = "file", data
		default:
			return fmt.Errorf("unsupported capture entry %q", path)
		}
		nodes[filepath.ToSlash(rel)] = node
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	return nodes
}

func runBoardOutputCLIResult(t *testing.T, directory, inputPath, outputPath string) error {
	t.Helper()
	before := d2rsPreflightInventory(t, directory)
	var err error
	message := ""
	if binary := os.Getenv("D2RS_BOARD_BINARY"); binary != "" {
		ctx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		cmd := exec.CommandContext(ctx, binary, inputPath, outputPath)
		cmd.Dir = directory
		if out, e := cmd.CombinedOutput(); e != nil {
			err = fmt.Errorf("native command: %v: %s", e, out)
			for _, line := range strings.Split(string(out), "\n") {
				if strings.HasPrefix(line, "err: ") {
					message += strings.TrimPrefix(line, "err: ") + "\n"
				}
			}
			message = strings.TrimSuffix(message, "\n")
			if message == "" {
				message = err.Error()
			}
		}
	} else {
		err = d2rsOriginalBoardOutputCLIResult(t, directory, inputPath, outputPath)
		if d2rsPreflightRunError != nil {
			message = d2rsPreflightRunError.Error()
		}
	}
	row := map[string]any{"name": t.Name(), "error": message, "before": before, "after": d2rsPreflightInventory(t, directory)}
	data, e := json.Marshal(row)
	if e != nil {
		t.Fatal(e)
	}
	key := fmt.Sprintf("%x.json", sha256.Sum256([]byte(t.Name())))
	if e := os.WriteFile(filepath.Join(os.Getenv("D2RS_BOARD_CAPTURE"), key), data, 0600); e != nil {
		t.Fatal(e)
	}
	return err
}

// Companion boards order the conflicting file after index.svg, so these
// exercise whole-tree admission rather than only the first lexical entry.
func TestD2rsPreflightLateSymlink(t *testing.T) {
	for _, extension := range []string{"svg", "png"} {
		t.Run(extension, func(t *testing.T) {
			directory := d2rsPreflightTempDir(t)
			input := filepath.Join(directory, "input.d2")
			output := filepath.Join(directory, "output."+extension)
			tree := filepath.Join(directory, "output")
			if err := os.WriteFile(input, []byte("root\nlayers: { z-child: { node } }\n"), 0644); err != nil {
				t.Fatal(err)
			}
			if err := os.Mkdir(tree, 0755); err != nil {
				t.Fatal(err)
			}
			victim := filepath.Join(directory, "victim")
			if err := os.WriteFile(victim, []byte("keep me"), 0640); err != nil {
				t.Fatal(err)
			}
			if err := os.Symlink(victim, filepath.Join(tree, "z-child."+extension)); err != nil {
				t.Fatal(err)
			}
			if err := runBoardOutputCLIResult(t, directory, input, output); err == nil || !strings.Contains(err.Error(), "symlink") {
				t.Fatalf("Run() = %v, want symlink rejection", err)
			}
			if _, err := os.Stat(filepath.Join(tree, "index."+extension)); !os.IsNotExist(err) {
				t.Fatalf("root output published: %v", err)
			}
		})
	}
}
