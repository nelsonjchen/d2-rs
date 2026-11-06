// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): d2scenebuild sketch internals. Harvested from
// the v0.9.0-era probes ../d2-v0.9.0-trace/d2renderers/d2scenebuild/
// sketch_rectangle_painter_probe_test.go (TestSketchStreakTileProbe) and
// sketch_font_role_probe_test.go (TestSketchFontRoleProbe), and the
// unavailable-image vector placement oracles
// .reference/vector-image-{origin,viewport}-20261018/go-oracle (case grids from
// their capture-source.py), and codex_export_preflight_test.go (raster export
// SVG preflight over tools/fixtures/inputs/renderexport/svg-native-preflight-inputs.json:
// templates whose {N} names a shared data: URL and whose [text, count] parts
// repeat text); retargeted to the manifest pin. Inert unless run by scripts/regen-fixtures.py.
package d2scenebuild

import (
	"bytes"
	"context"
	"encoding/base64"
	"image/png"
	"regexp"
	"strconv"
	"strings"
	"crypto/sha256"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"image/draw"
	"os"
	"path/filepath"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
	"github.com/d2lang/d2/d2renderers/d2raster"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/d2svgimport"
	"github.com/d2lang/d2/d2renderers/internal/patternassets"
	"github.com/d2lang/d2/d2target"
	"github.com/d2lang/d2/lib/textmeasure"
)

func d2rsWrite(t *testing.T, path string, data []byte) {
	if err := os.WriteFile(path, data, 0o644); err != nil {
		t.Fatal(err)
	}
}

// Outputs: the tile rows, then the bright/normal/dark/darker control SVGs.
func TestD2RSSketchStreakTiles(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	outputs := filepath.SplitList(os.Getenv("D2RS_RECIPE_OUTPUTS"))
	commands, err := sharedSketchStreakCommands(context.Background())
	if err != nil {
		t.Fatal(err)
	}
	type row struct {
		Category   string `json:"category"`
		Ink        string `json:"ink"`
		Alpha      uint8  `json:"alpha"`
		Scale      int    `json:"scale"`
		Background string `json:"background"`
		Width      int    `json:"width"`
		Height     int    `json:"height"`
		SHA256     string `json:"sha256"`
	}
	rows := []row{}
	for i, category := range []string{"bright", "normal", "dark", "darker"} {
		_, ink, err := sketchStreakStyle(category)
		if err != nil {
			t.Fatal(err)
		}
		root, err := sketchStreakRoot(context.Background(), commands, ink, category)
		if err != nil {
			t.Fatal(err)
		}
		paint := d2scene.PatternPaint{Tile: d2scene.Box{Width: 100, Height: 100}, Root: root, Units: d2scene.UserSpaceOnUse, Transform: d2scene.Identity()}
		doc := d2scene.NewDocument(d2scene.Box{Width: 100, Height: 100}, d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{Width: 100, Height: 100}, Fill: paint}))
		hexInk := "#000000"
		if ink.R == 255 {
			hexInk = "#ffffff"
		}
		svg := fmt.Sprintf(`<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100"><defs><pattern id="d2-rs-sketch-streak-control" width="100" height="100" patternUnits="userSpaceOnUse"><path fill-rule="evenodd" fill="%s" fill-opacity="%.17g" d="%s"/></pattern></defs><rect width="100" height="100" fill="url(#d2-rs-sketch-streak-control)"/></svg>`, hexInk, float64(ink.A)/255, patternassets.StreakPathData())
		d2rsWrite(t, outputs[1+i], []byte(svg))
		for _, scale := range []int{1, 2} {
			for _, background := range []string{"white", "transparent"} {
				options := patternFrameOptions()
				options.Scale = float64(scale)
				options.Background = color.White
				if background == "transparent" {
					options.Background = nil
				}
				frame, err := d2raster.Render(context.Background(), doc, options)
				if err != nil {
					t.Fatal(err)
				}
				rgba := image.NewRGBA(frame.Bounds())
				draw.Draw(rgba, rgba.Bounds(), frame, frame.Bounds().Min, draw.Src)
				digest := sha256.Sum256(rgba.Pix)
				rows = append(rows, row{category, hexInk, ink.A, scale, background, frame.Bounds().Dx(), frame.Bounds().Dy(), fmt.Sprintf("%x", digest)})
			}
		}
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	d2rsWrite(t, outputs[0], append(data, '\n'))
}

func TestD2RSSketchFontRoles(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type control struct {
		Primary    string `json:"primary"`
		Role       string `json:"role"`
		Bold       bool   `json:"bold"`
		Italic     bool   `json:"italic"`
		Family     string `json:"family"`
		Style      string `json:"style"`
		Weight     int    `json:"weight"`
		Asset      string `json:"asset"`
		FontSHA256 string `json:"font_sha256"`
	}
	controls := []control{}
	for _, primary := range []d2fonts.FontFamily{d2fonts.SourceSansPro, d2fonts.SourceCodePro, d2fonts.HandDrawn} {
		diagram := d2target.NewDiagram()
		diagram.FontFamily = &primary
		b := newSketchShapeTestBuilder(context.Background(), diagram, largeSketchTestBudget())
		for _, role := range []string{"default", "DEFAULT", "mono", "MoNo"} {
			for _, axes := range [][2]bool{{false, false}, {true, false}, {false, true}, {true, true}} {
				font, err := b.font(d2target.Text{FontFamily: role, FontSize: 16, Bold: axes[0], Italic: axes[1]})
				if err != nil {
					t.Fatal(err)
				}
				asset, ok := b.assets[font.Asset].(d2scene.FontAsset)
				if !ok {
					t.Fatalf("missing font asset %s", font.Asset)
				}
				controls = append(controls, control{string(primary), role, axes[0], axes[1], font.Family, font.Style, font.Weight, string(font.Asset), fmt.Sprintf("%x", sha256.Sum256(asset.Data))})
			}
		}
	}
	data, err := json.MarshalIndent(controls, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	d2rsWrite(t, out, append(data, '\n'))
}

// vector-image-{origin,viewport}-20261018: the unavailable-image placeholder
// asset placed through translated/affine matrices; complete PNG frames.
func TestD2RSVectorImageFrames(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type input struct {
		Name          string     `json:"name"`
		Width, Height int        `json:"-"`
		Matrix        [6]float64 `json:"-"`
		Box           [4]float64 `json:"-"`
		Slice         *bool      `json:"-"`
	}
	cases := []input{}
	if os.Getenv("D2RS_VECTOR_IMAGE_MODE") == "viewport" {
		matrices := [][6]float64{{1, 0, 0, 1, 35.4, 55.3}, {.8, .2, -.2, 1.1, 40.3, 35.7}, {.8, .6, -.6, .8, 70.2, 20.1}, {-1, 0, 0, 1, 130.7, 55.4}, {1, 0, 0, 1, -10.3, -7.6}}
		for i, m := range matrices {
			for j, b := range []struct {
				w, h  float64
				slice bool
			}{{80, 20, true}, {20, 80, true}, {64.2, 64.2, true}, {64.2, 64.2, false}} {
				slice := b.slice
				cases = append(cases, input{Name: fmt.Sprintf("viewport-%d-%d", i, j), Width: 192, Height: 192, Matrix: m, Box: [4]float64{.3, .7, b.w, b.h}, Slice: &slice})
			}
		}
	} else {
		for i, c := range [][3]float64{{0, 64, 64}, {10.3, 64, 64}, {200.4, 64, 64}, {1000.3, 64, 64}, {0.25, 76.8, 76.8}, {200.375, 76.8, 96.4}, {1000.625, 96.2, 76.8}, {200.3, 96.4, 96.4}} {
			cases = append(cases, input{Name: fmt.Sprintf("translated-%d", i), Width: 192, Height: 1280, Matrix: [6]float64{1, 0, 0, 1, 20.4, c[0]}, Box: [4]float64{0, 0, c[1], c[2]}})
		}
		for i, m := range [][6]float64{{1.2, 0, 0, 1.2, 20.4, 200.3}, {.8, .1, -.2, 1.2, 50.33, 200.57}, {1, 0, 0, 1, -5.3, 200.6}, {1, 0, 0, 1, 20.2, -2.4}} {
			cases = append(cases, input{Name: fmt.Sprintf("affine-%d", i), Width: 192, Height: 512, Matrix: m, Box: [4]float64{.3, .7, 86.4, 76.2}})
		}
	}
	rows := []map[string]any{}
	for _, in := range cases {
		bg := d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{Width: float64(in.Width), Height: float64(in.Height)}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 255, G: 255, B: 255, A: 255}}})
		aspect := d2scene.AspectRatio{Align: d2scene.AlignXMidYMid}
		if in.Slice != nil && *in.Slice {
			aspect.Fit = d2scene.AspectSlice
		}
		icon := d2scene.NewNode(d2scene.Image{Asset: "icon", Box: d2scene.Box{X: in.Box[0], Y: in.Box[1], Width: in.Box[2], Height: in.Box[3]}, Aspect: aspect})
		m := in.Matrix
		icon.Transform = d2scene.Matrix{A: m[0], B: m[1], C: m[2], D: m[3], E: m[4], F: m[5]}
		root := d2scene.NewNode(nil)
		root.Children = []*d2scene.Node{bg, icon}
		doc := d2scene.NewDocument(d2scene.Box{Width: float64(in.Width), Height: float64(in.Height)}, root)
		doc.Assets["icon"] = newUnavailableImageAsset()
		frame, err := d2raster.Render(context.Background(), doc, d2raster.FrameOptions{Scale: 1, MaxWidth: 2048, MaxHeight: 2048, MaxPixels: 4194304, MaxNodes: 1000, MaxDepth: 100, MaxPathCommands: 100000, MaxTextRunesPerRun: 10000, MaxAnimationTracks: 10000, MaxAnimationKeyframes: 1000000, MaxAssets: 1000, MaxAssetBytes: 67108864, MaxDecodedAssetBytes: 67108864, MaxImportDepth: 32, MaxOffscreenBytes: 67108864, MaxEvenOddClipWork: 1000000000})
		if err != nil {
			t.Fatal(err)
		}
		var encoded bytes.Buffer
		if err := png.Encode(&encoded, frame); err != nil {
			t.Fatal(err)
		}
		row := map[string]any{"name": in.Name, "Width": in.Width, "Height": in.Height, "Matrix": in.Matrix, "Box": in.Box, "png": base64.StdEncoding.EncodeToString(encoded.Bytes()), "sha256": fmt.Sprintf("%x", sha256.Sum256(encoded.Bytes()))}
		if in.Slice != nil {
			row["Slice"] = *in.Slice
		}
		rows = append(rows, row)
	}
	data, err := json.MarshalIndent(map[string]any{"cases": rows}, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	d2rsWrite(t, out, append(data, '\n'))
}

func TestD2RSExportPreflight(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var seed struct {
		Hrefs  []string          `json:"hrefs"`
		Inputs []json.RawMessage `json:"inputs"`
	}
	if err := json.Unmarshal(data, &seed); err != nil {
		t.Fatal(err)
	}
	slot := regexp.MustCompile(`\{(\d+)\}`)
	rows := []any{}
	for _, raw := range seed.Inputs {
		var template string
		if json.Unmarshal(raw, &template) != nil {
			var parts []json.RawMessage
			if err := json.Unmarshal(raw, &parts); err != nil {
				t.Fatal(err)
			}
			var b strings.Builder
			for _, part := range parts {
				var text string
				if json.Unmarshal(part, &text) == nil {
					b.WriteString(text)
					continue
				}
				var run [2]any
				if err := json.Unmarshal(part, &run); err != nil {
					t.Fatal(err)
				}
				b.WriteString(strings.Repeat(run[0].(string), int(run[1].(float64))))
			}
			template = b.String()
		}
		input := slot.ReplaceAllStringFunc(template, func(m string) string {
			i, _ := strconv.Atoi(m[1 : len(m)-1])
			return seed.Hrefs[i]
		})
		b := builder{ctx: context.Background(), options: Options{Assets: &AssetOptions{Resolver: newTestAssetResolver(t, nil), SVGImportBudget: SVGImportBudget{1 << 20, 4096, 8192, 1 << 20, 65536, 2048, 1024, 1024}, SVGImportLimits: d2svgimport.Limits{MaxBytes: 65536, MaxDepth: 32, MaxElements: 256, MaxAttributes: 512, MaxAttributeBytes: 65536, MaxPathCommands: 4096, MaxTransformFunctions: 128, MaxUseDepth: 8, MaxResources: 64}}}, assets: map[d2scene.AssetID]d2scene.Asset{}, assetIDs: map[[32]byte]d2scene.AssetID{}, sourceAssetIDs: map[string]d2scene.AssetID{}}
		source := "data:image/svg+xml;base64," + base64.StdEncoding.EncodeToString([]byte(input))
		_, e := b.resolveImageAsset("shape \"n\"", source)
		msg := ""
		if e != nil {
			msg = e.Error()
		}
		rows = append(rows, map[string]any{"input": input, "source": source, "error": msg})
	}
	data, err = json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	d2rsWrite(t, out, append(data, '\n'))
}

// tools/probes/markdown-document-scale-source-20261008.go: Markdown text shapes
// built at a scale with appendix/link budgets; layout, document links and
// viewport for the committed requests
// (tools/fixtures/inputs/renderexport/markdown-document-links-requests.json).
func TestD2RSMarkdownDocumentLinks(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var requests []struct {
		Name, Markdown, Family         string
		FontSize                       int
		Width, Height                  int
		Appendix, DisableLinks         bool
		MaxRegions, MaxStringBytes     int
		Scale                          float64
	}
	if err := json.Unmarshal(data, &requests); err != nil {
		t.Fatal(err)
	}
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	rows := []map[string]any{}
	for _, r := range requests {
		primary := d2fonts.SourceSansPro
		if r.Family == "mono" {
			primary = d2fonts.SourceCodePro
		}
		if r.Family == "hand" {
			primary = d2fonts.HandDrawn
		}
		mono := d2fonts.SourceCodePro
		layout, layoutErr := textmeasure.LayoutMarkdown(r.Markdown, ruler, &primary, &mono, r.FontSize)
		row := map[string]any{"request": r}
		if layoutErr != nil {
			row["layout_error"] = layoutErr.Error()
			rows = append(rows, row)
			continue
		}
		row["layout"] = layout
		w, h := r.Width, r.Height
		if w < 0 {
			w = layout.Width
		}
		if h < 0 {
			h = layout.Height
		}
		diagram := d2target.NewDiagram()
		diagram.FontFamily = &primary
		font := "default"
		if r.Family == "mono" {
			font = "mono"
		}
		if r.Family == "hand" {
			font = "hand"
		}
		shape := d2target.Shape{ID: "markdown", Type: d2target.ShapeText, Pos: d2target.Point{X: 0, Y: 0}, Width: w, Height: h, Fill: "N7", Stroke: "N1", StrokeWidth: 0, Opacity: 1, LabelPosition: "INSIDE_TOP_LEFT", Text: d2target.Text{Label: r.Markdown, Language: "markdown", FontFamily: font, FontSize: r.FontSize, LabelWidth: w, LabelHeight: h}}
		if r.DisableLinks {
			shape.Link = "https://outer.example.com"
			shape.PrettyLink = "outer"
		}
		diagram.Shapes = []d2target.Shape{shape}
		scale := r.Scale
		document, buildErr := Build(context.Background(), diagram, Options{Scale: &scale, Appendix: r.Appendix, LinkBudget: LinkBudget{MaxRegions: r.MaxRegions, MaxStringBytes: r.MaxStringBytes}})
		if buildErr != nil {
			row["error"] = buildErr.Error()
		} else {
			row["links"] = document.Links
			row["viewbox"] = document.ViewBox
			row["logical_width"] = document.LogicalWidth
			row["logical_height"] = document.LogicalHeight
		}
		rows = append(rows, row)
	}
	data, err = json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	d2rsWrite(t, out, append(data, '\n'))
}
