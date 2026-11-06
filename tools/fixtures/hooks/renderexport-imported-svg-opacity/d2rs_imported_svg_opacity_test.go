// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): complete frames of imported SVG images with
// group opacity, nesting, width and scale. Harvested from the v0.9.0 probe
// imported_svg_opacity_probe_test.go; inert unless run by regen-fixtures.py.
package d2raster

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"image/color"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/d2svgimport"
)

func TestD2RSImportedSVGOpacity(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type row struct {
		SVG    string  `json:"svg"`
		Width  int     `json:"width"`
		Height int     `json:"height"`
		Scale  int     `json:"scale"`
		Pixels []uint8 `json:"pixels"`
	}
	rows := []row{}
	for _, opacity := range []float64{.25, .5, .9} {
		for _, nested := range []bool{false, true} {
			for _, width := range []float64{16, 32} {
				for _, scale := range []int{1, 2} {
					second := `<path d="M 4 3 H 17 V 15 H 4 Z" fill="blue"/>`
					if nested {
						second = `<g opacity="0.5">` + second + `</g>`
					}
					svg := fmt.Sprintf(`<svg xmlns="http://www.w3.org/2000/svg" width="40" height="40" viewBox="0 0 20 20"><g opacity="%g" transform="rotate(30 10 10)"><path d="M -5 0 H 25 V 20 H -5 Z" fill="red"/>%s</g></svg>`, opacity, second)
					limits := d2svgimport.Limits{MaxBytes: 1 << 20, MaxDepth: 64, MaxElements: 1000, MaxAttributes: 10000, MaxAttributeBytes: 1 << 20, MaxPathCommands: 10000, MaxTransformFunctions: 1000, MaxUseDepth: 64, MaxResources: 1000}
					result, err := d2svgimport.ImportNode(context.Background(), "opacity-probe", []byte(svg), limits)
					if err != nil {
						t.Fatal(err)
					}
					content := d2scene.NewNode(nil)
					content.Transform = result.ViewportTransform
					content.Children = []*d2scene.Node{result.Root}
					assetRoot := d2scene.NewNode(nil)
					assetRoot.Clip = &d2scene.Clip{Path: clipRect(0, 0, 40, 40, d2scene.NonZero), Transform: d2scene.Identity()}
					assetRoot.Children = []*d2scene.Node{content}
					node := d2scene.NewNode(d2scene.Image{Asset: "vector", Box: d2scene.Box{X: 8, Y: 7, Width: width, Height: 16}, Aspect: d2scene.AspectRatio{Align: d2scene.AlignXMidYMid, Fit: d2scene.AspectMeet}})
					doc := d2scene.NewDocument(d2scene.Box{Width: 48, Height: 40}, node)
					doc.Assets["vector"] = d2scene.VectorAsset{ViewBox: d2scene.Box{Width: 40, Height: 40}, Root: assetRoot}
					options := testOptions()
					options.Background = color.NRGBA{R: 255, G: 255, B: 255, A: 255}
					options.Scale = float64(scale)
					frame, err := Render(context.Background(), doc, options)
					if err != nil {
						t.Fatal(err)
					}
					outer := fmt.Sprintf(`<svg xmlns="http://www.w3.org/2000/svg" width="48" height="40"><image x="8" y="7" width="%g" height="16" href="data:image/svg+xml;base64,%%s"/></svg>`, width)
					rows = append(rows, row{fmt.Sprintf(outer, base64.StdEncoding.EncodeToString([]byte(svg))), 48 * scale, 40 * scale, scale, append([]uint8(nil), frame.Pix...)})
				}
			}
		}
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(out, data, 0o644); err != nil {
		t.Fatal(err)
	}
}
