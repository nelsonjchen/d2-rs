// SPDX-License-Identifier: MPL-2.0
// Test-oracle probe for d2-rs-scene/-scenebuild/-raster, built only inside the
// trace checkout as cmd/d2rs-scene-probe.
//
// stdin: JSON lines {"board": <board.exp.json>, "themeID": <int|null>,
// "out": <dir>, "mode": "scenebuild"|"raster", "assets": <font store dir>}. Each board in source
// traversal order (like e2etests isometricBoards, root named "board") writes
// <name>.scene.json (d2scenebuild.Build, scenebuild mode), or additionally
// <name>.png (d2raster.Render of that document, raster mode), or <name>.err.
// Options: Pad 0, the case theme and config overrides, Appendix true, no
// asset or font resolvers (network- and filesystem-free), and the CLI's
// production raster ceilings at Scale 1 on white.
package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"image/color"
	"image/png"
	"os"
	"path/filepath"

	"github.com/d2lang/d2/d2renderers/d2raster"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2target"
)

func frameOptions() d2raster.FrameOptions {
	return d2raster.FrameOptions{
		Scale: 1, Background: color.White,
		MaxWidth: 32_768, MaxHeight: 32_768, MaxPixels: 64 * 1024 * 1024,
		MaxNodes: 1_000_000, MaxDepth: 1_024, MaxPathCommands: 10_000_000,
		MaxTextRunesPerRun: 100_000, MaxFontFacesPerText: 64, MaxTextCoverageChecks: 10_000_000,
		MaxTextShapingRuns: 100_000, MaxAnimationTracks: 1_000_000, MaxAnimationKeyframes: 10_000_000,
		MaxAssets: 4_096, MaxAssetBytes: 512 * 1024 * 1024, MaxDecodedAssetBytes: 512 * 1024 * 1024,
		MaxImportDepth: 1_024, MaxOffscreenBytes: 512 * 1024 * 1024, MaxEvenOddClipWork: 250_000_000,
		MaxScanlineWork: 4_000_000_000,
	}
}

type board struct {
	name    string
	diagram *d2target.Diagram
}

func boards(diagram *d2target.Diagram) []board {
	var out []board
	var walk func(*d2target.Diagram, string)
	walk = func(d *d2target.Diagram, name string) {
		if !d.IsFolderOnly {
			flat := *d
			flat.Layers, flat.Scenarios, flat.Steps = nil, nil, nil
			out = append(out, board{name, &flat})
		}
		for i, children := range [][]*d2target.Diagram{d.Layers, d.Scenarios, d.Steps} {
			kind := []string{"layers", "scenarios", "steps"}[i]
			for j, child := range children {
				walk(child, fmt.Sprintf("%s.%s.%d", name, kind, j))
			}
		}
	}
	walk(diagram, "board")
	return out
}

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 1<<20), 1<<26)
	for scanner.Scan() {
		var req struct {
			Board   string `json:"board"`
			ThemeID *int64 `json:"themeID"`
			Out     string `json:"out"`
			Mode    string `json:"mode"`
			Assets  string `json:"assets"`
		}
		if err := json.Unmarshal(scanner.Bytes(), &req); err != nil {
			panic(err)
		}
		data, err := os.ReadFile(req.Board)
		if err != nil {
			panic(err)
		}
		var diagram d2target.Diagram
		if err := json.Unmarshal(data, &diagram); err != nil {
			panic(err)
		}
		if err := os.MkdirAll(req.Out, 0o755); err != nil {
			panic(err)
		}
		assetStore = req.Assets
		var overrides *d2target.ThemeOverrides
		themeID := req.ThemeID
		if diagram.Config != nil {
			overrides = diagram.Config.ThemeOverrides
			if themeID == nil {
				themeID = diagram.Config.ThemeID
			}
		}
		pad := int64(0)
		for _, b := range boards(&diagram) {
			path := filepath.Join(req.Out, b.name)
			fail := func(err error) {
				if err := os.WriteFile(path+".err", []byte(err.Error()), 0o644); err != nil {
					panic(err)
				}
			}
			doc, err := d2scenebuild.Build(context.Background(), b.diagram, d2scenebuild.Options{
				Pad: &pad, ThemeID: themeID, ThemeOverrides: overrides, Appendix: true,
				LinkBudget: d2scenebuild.LinkBudget{MaxRegions: 4096, MaxStringBytes: 1 << 20},
			})
			if err != nil {
				fail(err)
				continue
			}
			encoded, err := json.Marshal(encDocument(doc))
			if err != nil {
				panic(err)
			}
			if err := os.WriteFile(path+".scene.json", encoded, 0o644); err != nil {
				panic(err)
			}
			if req.Mode != "raster" {
				continue
			}
			frame, err := d2raster.Render(context.Background(), doc, frameOptions())
			if err != nil {
				fail(err)
				continue
			}
			var buffer bytes.Buffer
			if err := png.Encode(&buffer, frame); err != nil {
				panic(err)
			}
			if err := os.WriteFile(path+".png", buffer.Bytes(), 0o644); err != nil {
				panic(err)
			}
		}
	}
	if err := scanner.Err(); err != nil {
		panic(err)
	}
}
