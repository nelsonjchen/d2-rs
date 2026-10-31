// SPDX-License-Identifier: MPL-2.0
package d2cli

import (
	"context"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2isometric/d2isometricimg"
	"github.com/d2lang/d2/d2target"
)

func d2rsSVGLinkWrite(t *testing.T, call int, value map[string]any) {
	t.Helper()
	data, err := json.Marshal(value)
	if err != nil {
		t.Fatal(err)
	}
	path := filepath.Join(os.Getenv("D2RS_MODE_CAPTURE"), "calls", fmt.Sprintf("%03d.json", call))
	if err := os.WriteFile(path, data, 0600); err != nil {
		t.Fatal(err)
	}
}

func d2rsSVGLinkRebase(t *testing.T, board *d2target.Diagram, key, sourceRoot string, paths map[string]string) (*d2target.Diagram, error) {
	t.Helper()
	original, err := json.Marshal(board)
	if err != nil {
		t.Fatal(err)
	}
	rebased, err := relinkIsometricSVGBoard(board, key, sourceRoot, paths)
	message := ""
	if err != nil {
		message = err.Error()
	}
	d2rsSVGLinkWrite(t, d2rsModeCall, map[string]any{"test": t.Name(), "call": d2rsModeCall,
		"input":  map[string]any{"operation": "svg-links", "board": json.RawMessage(original), "key": key, "source_root": sourceRoot, "paths": paths, "render": nil},
		"result": map[string]any{"board": rebased, "input_board": board, "error": message, "svg": nil, "render_error": ""}})
	d2rsModeCall++
	return rebased, err
}

func d2rsSVGLinkRender(t *testing.T, ctx context.Context, board *d2target.Diagram, options *d2isometricimg.Options) ([]byte, error) {
	t.Helper()
	// Attach the original Render call to its immediately preceding relink.
	call := d2rsModeCall - 1
	path := filepath.Join(os.Getenv("D2RS_MODE_CAPTURE"), "calls", fmt.Sprintf("%03d.json", call))
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatal(err)
	}
	var value map[string]any
	if err := json.Unmarshal(data, &value); err != nil {
		t.Fatal(err)
	}
	if value["test"] != t.Name() {
		t.Fatal("unexpected render/relink pairing")
	}
	value["input"].(map[string]any)["render"] = map[string]any{"format": options.Format, "width": options.Width, "height": options.Height}
	result, err := d2isometricimg.Render(ctx, board, options)
	message := ""
	if err != nil {
		message = err.Error()
	}
	value["result"].(map[string]any)["svg"] = string(result)
	value["result"].(map[string]any)["render_error"] = message
	d2rsSVGLinkWrite(t, call, value)
	return result, err
}
