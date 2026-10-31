// SPDX-License-Identifier: MPL-2.0
// Original upstream assertions remain active; native receives inputs only.
package d2cli

import (
	"context"
	"crypto/sha256"
	"encoding/json"
	"errors"
	"fmt"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"testing"
	"time"
)

var d2rsPublicationCalls = map[string]int{}
var d2rsPublicationDirectories = map[string]int{}

func d2rsPublicationTempDir(t *testing.T) string {
	t.Helper()
	n := d2rsPublicationDirectories[t.Name()]
	d2rsPublicationDirectories[t.Name()] = n + 1
	path := filepath.Join(os.Getenv("D2RS_PUBLICATION_WORK"), fmt.Sprintf("%x-%03d", sha256.Sum256([]byte(t.Name())), n))
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
func d2rsPublicationRecord(t *testing.T, row map[string]any) {
	t.Helper()
	n := d2rsPublicationCalls[t.Name()]
	d2rsPublicationCalls[t.Name()] = n + 1
	row["name"] = t.Name()
	row["call"] = n
	data, err := json.Marshal(row)
	if err != nil {
		t.Fatal(err)
	}
	key := fmt.Sprintf("%x-%03d.json", sha256.Sum256([]byte(t.Name())), n)
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_PUBLICATION_CAPTURE"), key), data, 0600); err != nil {
		t.Fatal(err)
	}
}
func imageCLI(t *testing.T, dir, source string, args ...string) ([]byte, error) {
	t.Helper()
	before := d2rsPublicationInventory(t, dir)
	var data []byte
	var err error
	if binary := os.Getenv("D2RS_PUBLICATION_BINARY"); binary != "" {
		input := filepath.Join(dir, "input.d2")
		if e := os.WriteFile(input, []byte(source), 0600); e != nil {
			t.Fatal(e)
		}
		ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
		defer cancel()
		cmd := exec.CommandContext(ctx, binary, append([]string{input}, args...)...)
		cmd.Dir = dir
		data, err = cmd.Output()
	} else {
		data, err = d2rsOriginalImageCLI(t, dir, source, args...)
	}
	if data == nil {
		data = []byte{}
	}
	d2rsPublicationRecord(t, map[string]any{"source": source, "args": args, "failed": err != nil, "stdout": data, "before": before, "after": d2rsPublicationInventory(t, dir)})
	return data, err
}
func d2rsPublicationAtomic(t *testing.T, ctx context.Context, path string, data []byte) error {
	t.Helper()
	before := d2rsPublicationInventory(t, filepath.Dir(path))
	var err error
	if binary := os.Getenv("D2RS_PUBLICATION_NATIVE_TEST"); binary != "" {
		request := filepath.Join(os.Getenv("D2RS_PUBLICATION_CAPTURE"), fmt.Sprintf("atomic-request-%03d.json", d2rsPublicationCalls[t.Name()]))
		response := filepath.Join(os.Getenv("D2RS_PUBLICATION_CAPTURE"), fmt.Sprintf("atomic-response-%03d.json", d2rsPublicationCalls[t.Name()]))
		encoded, e := json.Marshal(map[string]any{"path": path, "data": data, "canceled": ctx.Err() != nil})
		if e != nil {
			t.Fatal(e)
		}
		if e = os.WriteFile(request, encoded, 0600); e != nil {
			t.Fatal(e)
		}
		commandCtx, cancel := context.WithTimeout(context.Background(), 30*time.Second)
		defer cancel()
		cmd := exec.CommandContext(commandCtx, binary, "--exact", "isometric_cli::publication_api_tests::replay_atomic", "--ignored", "--nocapture")
		cmd.Env = append(os.Environ(), "D2RS_PUBLICATION_REQUEST="+request, "D2RS_PUBLICATION_RESPONSE="+response)
		if output, e := cmd.CombinedOutput(); e != nil {
			t.Fatalf("native atomic transport: %v: %s", e, output)
		}
		output, e := os.ReadFile(response)
		if e != nil {
			t.Fatal(e)
		}
		var result struct {
			Error string `json:"error"`
		}
		if e = json.Unmarshal(output, &result); e != nil {
			t.Fatal(e)
		}
		if result.Error == "context canceled" {
			err = context.Canceled
		} else if result.Error != "" {
			err = errors.New(result.Error)
		}
	} else {
		err = atomicIsometricImage(ctx, path, data)
	}
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsPublicationRecord(t, map[string]any{"path": path, "data": data, "canceled": ctx.Err() != nil, "error": message, "before": before, "after": d2rsPublicationInventory(t, filepath.Dir(path))})
	return err
}
func d2rsPublicationInventory(t *testing.T, directory string) map[string]any {
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
