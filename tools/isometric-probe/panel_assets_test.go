// SPDX-License-Identifier: MPL-2.0
// Public whole-board calls for source_document.go's asset handoff.
package d2isometricimg

import (
	"context"
	"net/url"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scenebuild"
	"github.com/d2lang/d2/d2renderers/d2svgimport"
)

func TestD2rsLegendAssetBoards(t *testing.T) {
	for _, engine := range []string{"dagre", "elk", "tala"} {
		for _, mode := range []string{"embedded", "external", "limited"} {
			t.Run(engine+"/"+mode, func(t *testing.T) {
				d := sourcePanelFixture(t, "txtar/legend-mono/"+engine+"/board.exp.json")
				if d.Legend == nil || len(d.Legend.Shapes) == 0 {
					t.Fatal("missing original legend")
				}
				u := iconData(t, "image/svg+xml", []byte(surfaceTestSVG))
				if mode == "external" {
					u, _ = url.Parse("https://example.invalid/legend.svg")
				}
				d.Legend.Shapes[0].Icon = u
				d.Legend.Shapes[0].IconPosition = "INSIDE_MIDDLE_LEFT"
				for _, api := range []string{"svg", "png", "page"} {
					o := &Options{Format: Format(api), Width: 320, Height: 200, FitContent: true}
					if mode == "limited" {
						o.Assets = &d2scenebuild.AssetOptions{SVGImportLimits: d2svgimport.Limits{MaxElements: 1}}
					}
					var err error
					if api == "page" {
						o.Format = PNG
						p, e := RenderPage(context.Background(), d, o)
						err = e
						if e != nil && p != nil {
							t.Fatal("failed call published page")
						}
					} else {
						data, e := Render(context.Background(), d, o)
						err = e
						if e != nil && data != nil {
							t.Fatal("failed call published image")
						}
					}
					if mode == "embedded" && err != nil {
						t.Fatal(err)
					}
					if mode == "external" && (err == nil || !strings.Contains(err.Error(), "native surface external icon requires an explicit asset resolver")) {
						t.Fatalf("implicit external access: %v", err)
					}
					if mode == "limited" && err == nil {
						t.Fatal("caller SVG limit ignored")
					}
				}
			})
		}
	}
}
