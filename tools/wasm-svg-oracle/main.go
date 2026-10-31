// SPDX-License-Identifier: MPL-2.0
// Native adapter for pinned D2 v0.9.0 d2wasm.Render's SVG path
// (d2js/d2wasm/functions.go:347-551). The syscall/js transport cannot be
// imported on a native Go host. All rendering remains the original Go code.
// Build from ../d2-v0.9.0-trace with its pinned module/dependency inputs.
package main

import (
	"encoding/json"
	"fmt"
	"os"

	"github.com/d2lang/d2/d2renderers/d2animate"
	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/util-go/go2"
)

type row struct {
	Input   string `json:"input"`
	Output  string `json:"output"`
	ThemeID *int64 `json:"theme_id"`
}

func render(r row) (out []byte, err error) {
	defer func() {
		if fault := recover(); fault != nil {
			err = fmt.Errorf("panic: %v", fault)
		}
	}()
	raw, err := os.ReadFile(r.Input)
	if err != nil {
		return nil, err
	}
	var diagram d2target.Diagram
	if err = json.Unmarshal(raw, &diagram); err != nil {
		return nil, err
	}
	// The scoreboard supplies the same hydrated e2e render options to both
	// APIs. These are explicit Render options, not inferred by d2wasm.Render.
	opts := &d2svg.RenderOpts{Pad: go2.Pointer(int64(0)), ThemeID: r.ThemeID}
	if c := diagram.Config; c != nil {
		if opts.ThemeID == nil {
			opts.ThemeID = c.ThemeID
		}
		opts.DarkThemeID, opts.Sketch, opts.Center = c.DarkThemeID, c.Sketch, c.Center
		opts.ThemeOverrides, opts.DarkThemeOverrides = c.ThemeOverrides, c.DarkThemeOverrides
	}
	descendants := len(diagram.Layers)+len(diagram.Scenarios)+len(diagram.Steps) > 0
	if !descendants {
		// No target wildcard: Render calls renderSingleBoard even for an
		// empty/folder-only target. The e2e harness instead calls renderBoards.
		diagram.Layers, diagram.Scenarios, diagram.Steps = nil, nil, nil
		return d2svg.Render(&diagram, opts)
	}
	opts.MasterID, err = diagram.HashID(nil)
	if err != nil {
		return nil, err
	}
	boards, err := d2svg.RenderMultiboard(&diagram, opts)
	if err != nil {
		return nil, err
	}
	if len(boards) == 0 {
		return nil, nil
	}
	return d2animate.Wrap(&diagram, boards, *opts, 1000)
}

func main() {
	raw, err := os.ReadFile(os.Args[1])
	if err != nil {
		panic(err)
	}
	var rows []row
	if err = json.Unmarshal(raw, &rows); err != nil {
		panic(err)
	}
	for _, r := range rows {
		out, failure := render(r)
		if failure != nil {
			err = os.WriteFile(r.Output+".err", []byte(failure.Error()), 0600)
		} else {
			err = os.WriteFile(r.Output, out, 0600)
		}
		if err != nil {
			panic(err)
		}
	}
}
