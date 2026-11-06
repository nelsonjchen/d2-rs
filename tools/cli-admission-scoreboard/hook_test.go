// SPDX-License-Identifier: MPL-2.0
// Observe original Run calls, preserving the original test assertions.
package d2cli

import (
	"bytes"
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/d2lang/util-go/cmdlog"
	"github.com/d2lang/util-go/xmain"
)

var d2rsAdmissionCall int

func d2rsAdmissionDir(t *testing.T) string {
	t.Helper()
	dir := filepath.Join(os.Getenv("D2RS_ADMISSION_CAPTURE"), "work", strings.ReplaceAll(t.Name(), "/", "_"))
	if err := os.MkdirAll(dir, 0700); err != nil {
		t.Fatal(err)
	}
	return dir
}

func d2rsAdmissionFiles(t *testing.T, dir string) map[string]any {
	t.Helper()
	files := map[string]any{}
	for _, name := range []string{"input.d2", "output.svg"} {
		path := filepath.Join(dir, name)
		data, err := os.ReadFile(path)
		if os.IsNotExist(err) {
			files[name] = nil
			continue
		}
		if err != nil {
			t.Fatal(err)
		}
		info, err := os.Stat(path)
		if err != nil {
			t.Fatal(err)
		}
		files[name] = map[string]any{"data": data, "mode": uint32(info.Mode().Perm())}
	}
	return files
}

func d2rsAdmissionRun(t *testing.T, ctx context.Context, ms *xmain.State, dir string, args []string) error {
	t.Helper()
	input := map[string]any{"directory": dir, "args": args, "canceled": ctx.Err() != nil, "files": d2rsAdmissionFiles(t, dir)}
	var stdout bytes.Buffer
	ms.Stdout = struct {
		io.Writer
		io.Closer
	}{io.MultiWriter(ms.Stdout, &stdout), ms.Stdout}
	err := Run(ctx, ms)
	var terminal bytes.Buffer
	if err != nil {
		// xmain.mainFatal's public error renderer, without its os.Exit.
		logger := cmdlog.New(ms.Env, &terminal)
		var exit xmain.ExitError
		var usage xmain.UsageError
		if errors.As(err, &exit) {
			if exit.Message != "" {
				logger.Error.Print(exit.Message)
			}
		} else {
			logger.Error.Print(err.Error())
			if errors.As(err, &usage) {
				logger.Error.Print("Run with --help to see usage.")
			}
		}
	}
	result := map[string]any{"error": terminal.String(), "stdout": base64.StdEncoding.EncodeToString(stdout.Bytes()), "files": d2rsAdmissionFiles(t, dir)}
	data, encodeErr := json.Marshal(map[string]any{"test": t.Name(), "call": d2rsAdmissionCall, "input": input, "result": result})
	if encodeErr != nil {
		t.Fatal(encodeErr)
	}
	path := filepath.Join(os.Getenv("D2RS_ADMISSION_CAPTURE"), "calls", fmt.Sprintf("%03d.json", d2rsAdmissionCall))
	d2rsAdmissionCall++
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}
	return err
}
