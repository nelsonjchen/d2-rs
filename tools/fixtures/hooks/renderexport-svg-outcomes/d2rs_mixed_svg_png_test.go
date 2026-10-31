// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): imported SVG documents with embedded PNG
// images, placed as vector images and rendered by d2raster: nested image
// matrices/boxes and complete frame digests. The 40 committed SVG assets
// (tools/fixtures/inputs/renderexport/mixed-svg-png-assets.json) are placed in
// two boxes at scales 1 and 2. Harvested from the v0.9.0-era probe
// tools/probes/mixed-svg-png-20261009.go; retargeted to the manifest pin.
// Inert unless run by scripts/regen-fixtures.py.
package d2svgimport

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"image/color"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2raster"
	"github.com/d2lang/d2/d2renderers/d2scene"
)

func TestD2RSMixedSVGPNG(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var assets []struct {
		Name string `json:"name"`
		SVG  string `json:"svg"`
	}
	if err := json.Unmarshal(data, &assets); err != nil {
		t.Fatal(err)
	}
	type request struct {
		Name, SVG, Aspect string
		Box, Canvas       [4]float64
		Scale             float64
	}
	requests := []request{}
	for _, a := range assets {
		for bi, box := range [][4]float64{{3.125, 4.25, 18.25, 17.5}, {2, 3, 24, 20}} {
			for _, scale := range []float64{1, 2} {
				requests = append(requests, request{Name: fmt.Sprintf("%s-b%d-s%g", a.Name, bi, scale), SVG: a.SVG, Aspect: "xMidYMid meet", Box: box, Canvas: [4]float64{0, 0, 32, 28}, Scale: scale})
			}
		}
	}
	rows := []map[string]any{}
	for _, r := range requests {
		imported, err := ImportNode(context.Background(), "mixed", []byte(r.SVG), generousImportLimits())
		if err != nil {
			t.Fatal(r.Name, err)
		}
		aspect := d2scene.AspectRatio{Align: d2scene.AlignXMidYMid, Fit: d2scene.AspectMeet}
		if r.Aspect == "none" {
			aspect.Align = d2scene.AlignNone
		}
		content := d2scene.NewNode(nil)
		content.Transform = imported.ViewportTransform
		content.Children = []*d2scene.Node{imported.Root}
		viewport := d2scene.NewNode(nil)
		viewport.Children = []*d2scene.Node{content}
		w, h := imported.Width, imported.Height
		viewport.Clip = &d2scene.Clip{Transform: d2scene.Identity(), Path: d2scene.Path{Commands: []d2scene.PathCommand{d2scene.MoveTo(0, 0), d2scene.LineTo(w, 0), d2scene.LineTo(w, h), d2scene.LineTo(0, h), d2scene.ClosePath()}}}
		host := d2scene.NewNode(d2scene.Image{Asset: "vector", Box: d2scene.Box{X: r.Box[0], Y: r.Box[1], Width: r.Box[2], Height: r.Box[3]}, Aspect: aspect})
		document := d2scene.NewDocument(d2scene.Box{X: r.Canvas[0], Y: r.Canvas[1], Width: r.Canvas[2], Height: r.Canvas[3]}, host)
		document.Assets["vector"] = d2scene.VectorAsset{ViewBox: d2scene.Box{Width: w, Height: h}, Root: viewport}
		for id, asset := range imported.Assets {
			document.Assets[id] = asset
		}
		frame, err := d2raster.Render(context.Background(), document, d2raster.FrameOptions{Scale: r.Scale, Background: color.White, MaxWidth: 256, MaxHeight: 256, MaxPixels: 65536, MaxNodes: 10000, MaxDepth: 100, MaxPathCommands: 100000, MaxAnimationTracks: 1, MaxAnimationKeyframes: 1, MaxAssets: 10, MaxAssetBytes: 2 << 20, MaxDecodedAssetBytes: 2 << 20, MaxImportDepth: 10, MaxOffscreenBytes: 8 << 20, MaxEvenOddClipWork: 8 << 20})
		if err != nil {
			t.Fatal(r.Name, err)
		}
		pixels := []byte{}
		for y := 0; y < frame.Bounds().Dy(); y++ {
			for x := 0; x < frame.Bounds().Dx(); x++ {
				c := frame.NRGBAAt(x, y)
				pixels = append(pixels, c.R, c.G, c.B, c.A)
			}
		}
		digest := sha256.Sum256(pixels)
		display, err := d2scene.AspectRatioMatrix(d2scene.Box{Width: w, Height: h}, d2scene.Box{Width: r.Box[2], Height: r.Box[3]}, aspect)
		if err != nil {
			t.Fatal(err)
		}
		prefix := d2scene.Scale(r.Scale, r.Scale).Mul(d2scene.Translate(-r.Canvas[0], -r.Canvas[1])).Mul(d2scene.Translate(r.Box[0], r.Box[1])).Mul(display).Mul(imported.ViewportTransform)
		images := []map[string]any{}
		var walk func(*d2scene.Node, d2scene.Matrix)
		walk = func(node *d2scene.Node, parent d2scene.Matrix) {
			matrix := parent.Mul(node.Transform)
			if primitive, ok := node.Primitive.(d2scene.Image); ok {
				images = append(images, map[string]any{"matrix": [6]float64{matrix.A, matrix.B, matrix.C, matrix.D, matrix.E, matrix.F}, "box": [4]float64{primitive.Box.X, primitive.Box.Y, primitive.Box.Width, primitive.Box.Height}})
			}
			for _, child := range node.Children {
				walk(child, matrix)
			}
		}
		walk(imported.Root, prefix)
		rows = append(rows, map[string]any{"request": r, "images": images, "dimensions": [2]int{frame.Bounds().Dx(), frame.Bounds().Dy()}, "sha256": hex.EncodeToString(digest[:])})
	}
	data, err = json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
