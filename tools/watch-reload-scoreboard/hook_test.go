// SPDX-License-Identifier: MPL-2.0
// Test-only transport; the original source writes, waits and assertions stay active.
package d2cli

import (
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"io"
	"io/fs"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/d2target"

	"github.com/d2lang/util-go/xmain"
)

var d2rsWatchStages []any
var d2rsWatchPreviewCalls []any
var d2rsLayoutCalls []any
var d2rsLayoutProcesses int

func d2rsWatchReloadRun(ctx context.Context, state *xmain.State) error {
	binary := os.Getenv("D2RS_WATCH_BINARY")
	if binary == "" {
		return Run(ctx, state)
	}
	cmd := exec.CommandContext(ctx, binary, state.Opts.Args...)
	cmd.Dir, cmd.Env = state.PWD, state.Env.Environ()
	cmd.Stdin, cmd.Stdout, cmd.Stderr = state.Stdin, state.Stdout, state.Stderr
	stem := "process"
	isWatch := false
	for _, arg := range state.Opts.Args {
		if arg == "--watch" {
			isWatch = true
		}
	}
	if !isWatch {
		stem = fmt.Sprintf("layout-%03d", d2rsLayoutProcesses)
		d2rsLayoutProcesses++
	}
	log, err := os.Create(filepath.Join(os.Getenv("D2RS_WATCH_CAPTURE"), stem+".log"))
	if err != nil {
		return err
	}
	defer log.Close()
	cmd.Stderr = io.MultiWriter(state.Stderr, log)
	errorPath := filepath.Join(os.Getenv("D2RS_WATCH_CAPTURE"), stem+"-api-error.json")
	cmd.Env = append(cmd.Env, "D2_RS_CLI_ERROR_JSON="+errorPath)
	// Exercise Rust's real signal shutdown and wait for the child to terminate.
	cmd.Cancel = func() error { return cmd.Process.Signal(os.Interrupt) }
	cmd.WaitDelay = 8 * time.Second
	err = cmd.Run()
	mapped := false
	var exited *exec.ExitError
	// main.rs intentionally exits 1 after a handled SIGINT, before its ordinary
	// error renderer. Map that process boundary only after explicit cancellation,
	// a completed child wait and no native API/terminal error. Keep the raw exit.
	logData, _ := os.ReadFile(log.Name())
	_, recordErr := os.Stat(errorPath)
	if isWatch && ctx.Err() != nil && errors.As(err, &exited) && exited.ExitCode() == 1 &&
		os.IsNotExist(recordErr) && !strings.Contains(string(logData), "err:") {
		mapped = true
	}
	exitCode := -1
	if cmd.ProcessState != nil {
		exitCode = cmd.ProcessState.ExitCode()
	}
	record := map[string]any{"canceled": ctx.Err() != nil, "mapped_cancellation": mapped,
		"exit_code": exitCode, "watch": isWatch}
	encoded, encodeErr := json.Marshal(record)
	if encodeErr != nil {
		return encodeErr
	}
	if writeErr := os.WriteFile(filepath.Join(os.Getenv("D2RS_WATCH_CAPTURE"), stem+"-exit.json"), encoded, 0600); writeErr != nil {
		return writeErr
	}
	if mapped {
		return ctx.Err()
	}
	return err
}

func d2rsLayoutRecord(t *testing.T, dir string, args []string, stdout string, err error) {
	t.Helper()
	outputs := map[string]any{}
	if walkErr := filepath.WalkDir(dir, func(path string, entry fs.DirEntry, e error) error {
		if e != nil {
			return e
		}
		if entry.IsDir() {
			return nil
		}
		rel, e := filepath.Rel(dir, path)
		if e != nil {
			return e
		}
		data, e := os.ReadFile(path)
		if e != nil {
			return e
		}
		info, e := os.Stat(path)
		if e != nil {
			return e
		}
		outputs[rel] = map[string]any{"data": data, "mode": uint32(info.Mode().Perm())}
		return nil
	}); walkErr != nil {
		t.Fatal(walkErr)
	}
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsLayoutCalls = append(d2rsLayoutCalls, map[string]any{"test": t.Name(), "args": args, "stdout": stdout, "error": message, "files": outputs})
	data, encodeErr := json.Marshal(d2rsLayoutCalls)
	if encodeErr != nil {
		t.Fatal(encodeErr)
	}
	if writeErr := os.WriteFile(filepath.Join(os.Getenv("D2RS_WATCH_CAPTURE"), "layouts.json"), data, 0600); writeErr != nil {
		t.Fatal(writeErr)
	}
}

func d2rsWatchTree(root, selected *d2target.Diagram) any {
	ids := map[*d2target.Diagram]int{}
	nodes := []any{}
	var visit func(*d2target.Diagram) int
	visit = func(board *d2target.Diagram) int {
		if id, ok := ids[board]; ok {
			return id
		}
		id := len(nodes)
		ids[board] = id
		node := map[string]any{}
		nodes = append(nodes, node)
		copy := *board
		copy.Layers, copy.Scenarios, copy.Steps = nil, nil, nil
		node["diagram"] = &copy
		for i, children := range [][]*d2target.Diagram{board.Layers, board.Scenarios, board.Steps} {
			kind := []string{"layers", "scenarios", "steps"}[i]
			indices := make([]int, 0, len(children))
			for _, child := range children {
				indices = append(indices, visit(child))
			}
			node[kind] = indices
		}
		return id
	}
	rootID := visit(root)
	selectedID := visit(selected)
	return map[string]any{"root": rootID, "selected": selectedID, "nodes": nodes}
}

func d2rsWatchPreview(t *testing.T, ctx context.Context, state *xmain.State, diagram *d2target.Diagram, render d2svg.RenderOpts, input, sourceRootID string, roots ...*d2target.Diagram) ([]byte, error) {
	t.Helper()
	root := diagram
	if len(roots) > 0 && roots[0] != nil {
		root, sourceRootID = roots[0], "root"
	}
	if sourceRootID == "" {
		sourceRootID = "root"
	}
	request := map[string]any{"tree": d2rsWatchTree(root, diagram), "render": render, "input_path": input, "source_root_id": sourceRootID}
	// Freeze request bytes before calling the original function.
	before, err := json.Marshal(request)
	if err != nil {
		t.Fatal(err)
	}
	data, err := renderIsometricPreview(ctx, state, diagram, render, input, sourceRootID, roots...)
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsWatchPreviewCalls = append(d2rsWatchPreviewCalls, map[string]any{
		"test": t.Name(), "input": json.RawMessage(before),
		"result": map[string]any{"svg": string(data), "error": message, "tree_after": d2rsWatchTree(root, diagram), "render_after": render},
	})
	encoded, encodeErr := json.Marshal(d2rsWatchPreviewCalls)
	if encodeErr != nil {
		t.Fatal(encodeErr)
	}
	if writeErr := os.WriteFile(filepath.Join(os.Getenv("D2RS_WATCH_CAPTURE"), "previews.json"), encoded, 0600); writeErr != nil {
		t.Fatal(writeErr)
	}
	return data, err
}

func d2rsWatchReloadRecord(t *testing.T, dir string, data []byte, isometric bool) {
	t.Helper()
	source, err := os.ReadFile(filepath.Join(dir, "input.d2"))
	if err != nil {
		t.Fatal(err)
	}
	outputs := map[string]any{}
	err = filepath.WalkDir(dir, func(path string, entry fs.DirEntry, err error) error {
		if err != nil {
			return err
		}
		if entry.IsDir() {
			return nil
		}
		rel, err := filepath.Rel(dir, path)
		if err != nil {
			return err
		}
		if rel == "input.d2" || rel == "watch.log" {
			return nil
		}
		value, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		info, err := os.Stat(path)
		if err != nil {
			return err
		}
		outputs[rel] = map[string]any{"data": value, "mode": uint32(info.Mode().Perm())}
		return nil
	})
	if err != nil {
		t.Fatal(err)
	}
	d2rsWatchStages = append(d2rsWatchStages, map[string]any{
		"test": t.Name(), "source": source, "isometric": isometric, "svg": data, "outputs": outputs,
	})
	encoded, err := json.Marshal(d2rsWatchStages)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(os.Getenv("D2RS_WATCH_CAPTURE"), "stages.json"), encoded, 0600); err != nil {
		t.Fatal(err)
	}
	if len(d2rsWatchStages) > 4 {
		t.Fatal(fmt.Errorf("unexpected extra watch stage"))
	}
}
