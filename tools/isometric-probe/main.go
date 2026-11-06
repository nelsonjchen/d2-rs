// SPDX-License-Identifier: MPL-2.0
// Test-oracle probe for d2-rs-isometric, built only inside the trace checkout
// (scripts/isometric-scoreboard.py copies it to cmd/d2rs-isometric-probe).
//
// stdin: JSON lines {"board": <board.exp.json path>, "themeID": <int|null>,
// "out": <directory>, "mode": "scene"|"svg"}. For every isometric board in
// source traversal order (e2etests/isometric_test.go isometricBoards) it
// writes <name>.json (scene mode: d2isometric.BuildScene) or <name>.svg (svg
// mode: d2isometricimg.Render with the e2e options), or <name>.err.
package main

import (
	"bufio"
	"bytes"
	"context"
	"encoding/json"
	"fmt"
	"io"
	"net/http"
	"os"
	"path/filepath"

	"github.com/d2lang/d2/d2renderers/d2isometric"
	"github.com/d2lang/d2/d2renderers/d2isometric/d2isometricimg"
	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/imageasset"
	"github.com/d2lang/d2/lib/netpolicy"
)

type request struct {
	Board        string                `json:"board"`
	ThemeID      *int64                `json:"themeID"`
	Out          string                `json:"out"`
	Mode         string                `json:"mode"`
	AssetSources map[string]assetInput `json:"assetSources"`
}

// An explicit, immutable transport input. No request may escape this map;
// diagrams and original locators are preserved verbatim for source comparisons.
type assetInput struct {
	Path string `json:"path"`
	MIME string `json:"mime"`
}
type assetTransport map[string]assetInput

func (t assetTransport) RoundTrip(req *http.Request) (*http.Response, error) {
	v, ok := t[req.URL.String()]
	if !ok {
		return nil, fmt.Errorf("unrecorded isometric asset source %q", req.URL.String())
	}
	data, err := os.ReadFile(v.Path)
	if err != nil {
		return nil, err
	}
	if len(data) > 8<<20 {
		return nil, fmt.Errorf("isometric probe asset exceeds 8 MiB")
	}
	return &http.Response{StatusCode: 200, Header: http.Header{"Content-Type": []string{v.MIME}}, ContentLength: int64(len(data)), Body: io.NopCloser(bytes.NewReader(data)), Request: req}, nil
}
func assetOptions(sources map[string]assetInput) *d2scenebuild.AssetOptions {
	if sources == nil {
		return nil
	}
	resolver, err := imageasset.New(imageasset.Options{
		HTTPClient: &http.Client{Transport: assetTransport(sources)}, NetworkPolicy: netpolicy.Policy{AllowPrivateNetworks: true},
		Limits: imageasset.Limits{MaxFetchedBytes: 12 << 20, MaxEncodedBytes: 8 << 20, MaxDecompressedBytes: 8 << 20, MaxSVGBytes: 2 << 20, MaxDecodedWidth: 8192, MaxDecodedHeight: 8192, MaxDecodedPixels: 8 << 20, MaxAssets: 7000, MaxCumulativeEncodedBytes: 16 << 20, MaxCumulativeDecodedBytes: 64 << 20},
	})
	if err != nil {
		panic(err)
	}
	return &d2scenebuild.AssetOptions{Resolver: resolver}
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
			out = append(out, board{name, d})
		}
		for i, children := range [][]*d2target.Diagram{d.Layers, d.Scenarios, d.Steps} {
			kind := []string{"layers", "scenarios", "steps"}[i]
			for j, child := range children {
				walk(child, fmt.Sprintf("%s.%s.%d", name, kind, j))
			}
		}
	}
	walk(diagram, "isometric")
	return out
}

func main() {
	scanner := bufio.NewScanner(os.Stdin)
	scanner.Buffer(make([]byte, 1<<20), 1<<26)
	for scanner.Scan() {
		var req request
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
		// e2e renderOpts: ThemeID from the case; ThemeOverrides from the
		// compiled config (d2lib.Compile applies config to nil options).
		var overrides *d2target.ThemeOverrides
		themeID := req.ThemeID
		if diagram.Config != nil {
			overrides = diagram.Config.ThemeOverrides
			if themeID == nil {
				themeID = diagram.Config.ThemeID
			}
		}
		for _, b := range boards(&diagram) {
			path := filepath.Join(req.Out, b.name)
			var payload []byte
			var renderErr error
			if req.Mode == "scene" {
				var scene *d2isometric.Scene
				scene, renderErr = d2isometric.BuildScene(b.diagram, &d2isometric.RenderOpts{ThemeID: themeID, ThemeOverrides: overrides})
				if renderErr == nil {
					payload, renderErr = json.Marshal(scene)
				}
				path += ".json"
			} else {
				options := d2isometricimg.Options{
					Assets: assetOptions(req.AssetSources),
					Render: d2isometric.RenderOpts{ThemeID: themeID, ThemeOverrides: overrides},
					Format: d2isometricimg.SVG,
					Width:  1200, Height: 1200, FitContent: true,
				}
				payload, renderErr = d2isometricimg.Render(context.Background(), b.diagram, &options)
				path += ".svg"
			}
			if renderErr != nil {
				path = filepath.Join(req.Out, b.name) + ".err"
				payload = []byte(renderErr.Error())
			}
			if err := os.WriteFile(path, payload, 0o644); err != nil {
				panic(err)
			}
		}
	}
	if err := scanner.Err(); err != nil {
		panic(err)
	}
}
