// SPDX-License-Identifier: MPL-2.0
// Original transaction assertions/callbacks run on both implementations.
package d2cli

import (
	"bufio"
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
)

func d2rsTransactionTempDir(t *testing.T) string {
	t.Helper()
	path := filepath.Join(os.Getenv("D2RS_TRANSACTION_WORK"), fmt.Sprintf("%x", sha256.Sum256([]byte(t.Name()))))
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

func d2rsTransactionInventory(t *testing.T, directory string) map[string]any {
	t.Helper()
	nodes := map[string]any{}
	err := filepath.WalkDir(directory, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		// Stage directory names are random private implementation details; require
		// their complete absence after publication separately, never normalize them.
		rel, err := filepath.Rel(directory, path)
		if err != nil {
			return err
		}
		info, err := entry.Info()
		if err != nil {
			return err
		}
		node := map[string]any{"mode": uint32(info.Mode().Perm())}
		switch {
		case entry.Type()&os.ModeSymlink != 0:
			target, err := os.Readlink(path)
			if err != nil {
				return err
			}
			node["type"], node["link"] = "symlink", target
		case entry.IsDir():
			node["type"] = "directory"
		case info.Mode().IsRegular():
			data, err := os.ReadFile(path)
			if err != nil {
				return err
			}
			node["type"], node["bytes"] = "file", data
		default:
			return fmt.Errorf("unsupported inventory entry %q", path)
		}
		nodes[filepath.ToSlash(rel)] = node
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	return nodes
}

type d2rsTransactionError struct {
	message string
	cause   error
}

func (e *d2rsTransactionError) Error() string { return e.message }
func (e *d2rsTransactionError) Unwrap() error { return e.cause }

func d2rsTransactionPublish(t *testing.T, directory string, w *boardOutputWorkspace) (bool, error) {
	t.Helper()
	var touched bool
	var err, callbackErr error
	callbacks := []string{}
	originalCallback := w.beforePublish
	w.beforePublish = func(path string) error {
		callbacks = append(callbacks, path)
		if originalCallback != nil {
			return originalCallback(path)
		}
		return nil
	}
	if binary := os.Getenv("D2RS_TRANSACTION_BINARY"); binary != "" {
		calls := os.Getenv("D2RS_TRANSACTION_CAPTURE")
		key := fmt.Sprintf("%x", sha256.Sum256([]byte(t.Name())))
		requestPath, responsePath := filepath.Join(calls, key+"-request.json"), filepath.Join(calls, key+"-response.json")
		data, e := json.Marshal(map[string]any{"finalRoot": w.finalRoot, "stageRoot": w.stageRoot})
		if e != nil {
			t.Fatal(e)
		}
		if e := os.WriteFile(requestPath, data, 0600); e != nil {
			t.Fatal(e)
		}
		ctx, cancel := context.WithTimeout(context.Background(), 60*time.Second)
		defer cancel()
		cmd := exec.CommandContext(ctx, binary, "--exact", "board_output::transaction_api_tests::replay_transaction", "--ignored", "--nocapture")
		cmd.Env = append(os.Environ(), "D2RS_TRANSACTION_REQUEST="+requestPath, "D2RS_TRANSACTION_RESPONSE="+responsePath)
		stdin, e := cmd.StdinPipe()
		if e != nil {
			t.Fatal(e)
		}
		stdout, e := cmd.StdoutPipe()
		if e != nil {
			t.Fatal(e)
		}
		var stderr strings.Builder
		cmd.Stderr = &stderr
		if e := cmd.Start(); e != nil {
			t.Fatal(e)
		}
		scanner := bufio.NewScanner(stdout)
		scanner.Buffer(make([]byte, 4096), 1<<20)
		transcript := strings.Builder{}
		for scanner.Scan() {
			line := scanner.Text()
			transcript.WriteString(line + "\n")
			if payload, ok := strings.CutPrefix(line, "D2RS_CALLBACK "); ok {
				var path string
				if e := json.Unmarshal([]byte(payload), &path); e != nil {
					t.Fatal(e)
				}
				e := w.beforePublish(path)
				message := ""
				if e != nil {
					callbackErr = e
					message = e.Error()
				}
				reply, _ := json.Marshal(message)
				if _, e := fmt.Fprintf(stdin, "%s\n", reply); e != nil {
					t.Fatal(e)
				}
			}
		}
		if e := scanner.Err(); e != nil {
			t.Fatal(e)
		}
		stdin.Close()
		if e := cmd.Wait(); e != nil {
			t.Fatalf("native replay: %v: %s %s", e, transcript.String(), stderr.String())
		}
		data, e = os.ReadFile(responsePath)
		if e != nil {
			t.Fatal(e)
		}
		var result struct {
			Touched bool   `json:"touched"`
			Error   string `json:"error"`
		}
		if e := json.Unmarshal(data, &result); e != nil {
			t.Fatal(e)
		}
		touched = result.Touched
		if result.Error != "" {
			err = &d2rsTransactionError{result.Error, callbackErr}
		}
		// The native workspace owns/discards this exact staging path.
		w.stageRoot = ""
	} else {
		touched, err = w.publish()
	}
	message := ""
	if err != nil {
		message = err.Error()
	}
	row := map[string]any{"name": t.Name(), "touched": touched, "error": message, "callbacks": callbacks, "after": d2rsTransactionInventory(t, directory)}
	data, e := json.Marshal(row)
	if e != nil {
		t.Fatal(e)
	}
	key := fmt.Sprintf("%x.json", sha256.Sum256([]byte(t.Name())))
	if e := os.WriteFile(filepath.Join(os.Getenv("D2RS_TRANSACTION_CAPTURE"), key), data, 0600); e != nil {
		t.Fatal(e)
	}
	return touched, err
}
