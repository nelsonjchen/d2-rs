// SPDX-License-Identifier: MPL-2.0
// Installed only in the pinned trace checkout; original assertions stay active.
package d2cli

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"github.com/d2lang/util-go/xmain"
	"github.com/d2lang/util-go/xos"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"sync/atomic"
	"testing"
	"time"
)

var d2rsPolicyCalls []any

type d2rsPolicyBuffer struct{ buffer bytes.Buffer }

func (b *d2rsPolicyBuffer) Bytes() []byte  { return b.buffer.Bytes() }
func (b *d2rsPolicyBuffer) String() string { return b.buffer.String() }

func (b *d2rsPolicyBuffer) Write(p []byte) (int, error) {
	if len(p) > 128*1024*1024-b.buffer.Len() {
		return 0, errors.New("policy capture exceeds 128 MiB")
	}
	return b.buffer.Write(p)
}

func d2rsPolicyCLI(t *testing.T, requests *atomic.Int32, dir, source string, args ...string) ([]byte, error) {
	t.Helper()
	allowed := args[0] == "--allow-private-network"
	index := len(d2rsPolicyCalls)
	work := os.Getenv("D2RS_POLICY_CAPTURE")
	results := map[string]any{}
	var nativeData []byte
	var nativeErr error
	for _, side := range []string{"go", "native"} {
		before := requests.Load()
		var data []byte
		var err error
		if side == "go" {
			data, err = imageCLI(t, dir, source, args...)
		} else {
			ctx, cancel := context.WithTimeout(context.Background(), time.Minute)
			defer cancel()
			var stdout d2rsPolicyBuffer
			state := &xmain.TestState{Args: []string{"d2"}, PWD: dir, Stdout: &stdout, Env: xos.NewEnv([]string{"PATH="}),
				Run: func(ctx context.Context, ms *xmain.State) error {
					cmd := exec.CommandContext(ctx, os.Getenv("D2RS_POLICY_BINARY"), append([]string{filepath.Join(dir, "input.d2")}, args...)...)
					cmd.Dir = dir
					errorPath := filepath.Join(work, fmt.Sprintf("%02d-error.json", index))
					cmd.Env = []string{"PATH=", "SOURCE_DATE_EPOCH=" + os.Getenv("SOURCE_DATE_EPOCH"), "D2_RS_CLI_ERROR_JSON=" + errorPath}
					var stderr d2rsPolicyBuffer
					cmd.Stdout, cmd.Stderr = ms.Stdout, &stderr
					processErr := cmd.Run()
					if e := os.WriteFile(filepath.Join(work, fmt.Sprintf("%02d-stderr.log", index)), stderr.Bytes(), 0600); e != nil {
						return e
					}
					if processErr != nil {
						var record struct {
							Error string `json:"error"`
							Code  int    `json:"code"`
						}
						encoded, e := os.ReadFile(errorPath)
						if e != nil {
							return fmt.Errorf("native process failed without API error: %w: %s", processErr, stderr.String())
						}
						if e := json.Unmarshal(encoded, &record); e != nil {
							return e
						}
						var exited *exec.ExitError
						if !errors.As(processErr, &exited) || exited.ExitCode() != record.Code || record.Error == "" {
							return errors.New("invalid native error transport")
						}
						// Reverse only the verified command-level mainFatal prefix, as the
						// shared CLI transport does; xmain adds its own API wrappers below.
						if !strings.HasPrefix(record.Error, "err: ") || !strings.Contains(stderr.String(), record.Error+"\n") {
							return errors.New("native terminal/API error disagreement")
						}
						processErr = errors.New(strings.TrimPrefix(record.Error, "err: "))
					}
					return processErr
				},
			}
			state.Start(t, ctx)
			defer state.Cleanup(t)
			err = state.Wait(ctx)
			data = stdout.Bytes()
			nativeData, nativeErr = data, err
		}
		fetched := requests.Load() - before
		message := ""
		if err != nil {
			message = err.Error()
		}
		input, e := os.ReadFile(filepath.Join(dir, "input.d2"))
		if e != nil {
			t.Fatal(e)
		}
		entries, e := os.ReadDir(dir)
		if e != nil {
			t.Fatal(e)
		}
		if len(entries) != 1 || entries[0].Name() != "input.d2" {
			t.Fatal("unexpected file output")
		}
		info, e := entries[0].Info()
		if e != nil {
			t.Fatal(e)
		}
		results[side] = map[string]any{"data": data, "error": message, "requests": fetched,
			"files": map[string]any{"input.d2": map[string]any{"data": input, "mode": uint32(info.Mode().Perm())}}}
		// The same original denied/allowed conditions also apply independently
		// to Go; the enclosing, unchanged test checks the returned native values.
		if side == "go" && ((!allowed && (err == nil || len(data) != 0 || fetched != 0)) || (allowed && (err != nil || len(data) == 0 || fetched <= 0))) {
			t.Fatalf("Go did not reproduce original policy contract: %v, bytes=%d, requests=%d", err, len(data), fetched)
		}
	}
	d2rsPolicyCalls = append(d2rsPolicyCalls, map[string]any{"test": t.Name(), "source": source, "args": args, "allowed": allowed, "results": results})
	encoded, err := json.Marshal(d2rsPolicyCalls)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(work, "calls.json"), encoded, 0600); err != nil {
		t.Fatal(err)
	}
	return nativeData, nativeErr
}
