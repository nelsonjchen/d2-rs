// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): d2raster frames and prepared-scene internals.
// Harvested from the v0.9.0-era probes ../d2-v0.9.0-trace/d2renderers/d2raster/
// {appendix_blur_bounds,sketch_blend,markdown_reference,markdown_reference_matrix}_probe_test.go,
// ../d2-v0.9.0-trace/{codex_raster_bands_probe_test,codex_imported_primitives_test,
// codex_math_vector_test,opacity_overhanging_probe}.go.overlay,
// .reference/canonical-aggregate-bindings-20261004/{effect-text-bounds,native-underline}-source-probe.go
// and tools/probes/{opaque-arrow-mask-20261004,markdown-clip-pixels-source-20261008}.go.
// Grids are the probes' own. Each test writes $D2RS_RECIPE_OUTPUT(S); retargeted to
// the manifest pin. Inert unless run by scripts/regen-fixtures.py.
package d2raster

import (
	"context"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"image/draw"
	"math"
	"os"
	"path/filepath"
	"strings"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
	"github.com/d2lang/d2/d2renderers/d2latex"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/d2svgimport"
	"github.com/d2lang/d2/lib/textmeasure"
)

func d2rsOut(t *testing.T) string {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	return out
}

func d2rsWriteJSON(t *testing.T, path string, value any, indent bool) {
	var data []byte
	var err error
	if indent {
		data, err = json.MarshalIndent(value, "", "  ")
	} else {
		data, err = json.Marshal(value)
	}
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}

// appendix_blur_bounds_probe_test.go
func TestD2RSBlurExpanded(t *testing.T) {
	out := d2rsOut(t)
	type layer struct {
		Bounds [4]int `json:"bounds"`
		Alpha  []int  `json:"alpha"`
	}
	type probe struct {
		Name   string `json:"name"`
		Input  layer  `json:"input"`
		Radii  [3]int `json:"radii"`
		Output layer  `json:"output"`
	}
	capture := func(img *image.Alpha) layer {
		b := img.Bounds()
		values := []int{}
		for y := b.Min.Y; y < b.Max.Y; y++ {
			for x := b.Min.X; x < b.Max.X; x++ {
				values = append(values, int(img.AlphaAt(x, y).A))
			}
		}
		return layer{[4]int{b.Min.X, b.Min.Y, b.Max.X, b.Max.Y}, values}
	}
	rows := []probe{}
	for _, c := range []struct {
		name          string
		width, height int
		values        []byte
	}{
		{"single_edge_impulse", 1, 1, []byte{255}},
		{"opposite_edge_impulses", 3, 2, []byte{255, 0, 0, 0, 0, 127}},
	} {
		initial := image.NewAlpha(image.Rect(0, 0, c.width, c.height))
		copy(initial.Pix, c.values)
		passes, _, err := prepareBlurPasses(initial.Bounds(), 4, 4)
		if err != nil {
			t.Fatal(err)
		}
		current := initial
		for _, pass := range passes {
			next := image.NewAlpha(pass.bounds)
			if err := boxBlurAlpha(context.Background(), next, current, pass); err != nil {
				t.Fatal(err)
			}
			current = next
		}
		rows = append(rows, probe{c.name, capture(initial), splitBlurRadius(4), capture(current)})
	}
	d2rsWriteJSON(t, out, rows, true)
}

var d2rsBlendModes = []struct {
	name string
	mode d2scene.BlendMode
}{{"multiply", d2scene.BlendMultiply}, {"darken", d2scene.BlendDarken}, {"color-burn", d2scene.BlendColorBurn}, {"overlay", d2scene.BlendOverlay}, {"lighten", d2scene.BlendLighten}}

// sketch_blend_probe_test.go TestSketchBlendProbe
func TestD2RSSketchBlends(t *testing.T) {
	out := d2rsOut(t)
	type row struct {
		Mode     string   `json:"mode"`
		Backdrop [4]uint8 `json:"backdrop"`
		Source   [4]uint8 `json:"source"`
		Opacity  float64  `json:"opacity"`
		Result   [4]uint8 `json:"result"`
	}
	pixels := [][2][4]uint8{
		{{64, 192, 255, 255}, {128, 64, 255, 255}}, {{64, 32, 16, 128}, {96, 16, 48, 192}},
		{{0, 0, 0, 0}, {64, 32, 16, 128}}, {{20, 40, 60, 80}, {0, 0, 0, 0}},
		{{255, 127, 128, 255}, {0, 255, 128, 255}}, {{33, 65, 97, 101}, {51, 101, 151, 191}},
	}
	rows := []row{}
	for _, m := range d2rsBlendModes {
		for _, p := range pixels {
			for _, opacity := range []float64{1, .5, .37} {
				dst := image.NewRGBA(image.Rect(0, 0, 1, 1))
				src := image.NewRGBA(dst.Bounds())
				dst.SetRGBA(0, 0, color.RGBA{R: p[0][0], G: p[0][1], B: p[0][2], A: p[0][3]})
				src.SetRGBA(0, 0, color.RGBA{R: p[1][0], G: p[1][1], B: p[1][2], A: p[1][3]})
				if err := compositeLayer(context.Background(), dst, src, opacity, m.mode); err != nil {
					t.Fatal(err)
				}
				c := dst.RGBAAt(0, 0)
				rows = append(rows, row{m.name, p[0], p[1], opacity, [4]uint8{c.R, c.G, c.B, c.A}})
			}
		}
	}
	d2rsWriteJSON(t, out, rows, true)
}

// sketch_blend_probe_test.go TestSketchMaskedBlendProbe
func TestD2RSSketchMaskedBlends(t *testing.T) {
	out := d2rsOut(t)
	type row struct {
		Mode    string  `json:"mode"`
		Opacity float64 `json:"opacity"`
		Pixels  []uint8 `json:"pixels"`
	}
	rows := []row{}
	for _, m := range d2rsBlendModes {
		for _, opacity := range []float64{1, .5, .37} {
			background := d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{Width: 8, Height: 8}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 64, G: 192, B: 255, A: 255}}})
			maskRoot := d2scene.NewNode(nil)
			maskRoot.Children = []*d2scene.Node{d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{Width: 8, Height: 8}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 255, G: 255, B: 255, A: 255}}}), d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{X: 2, Width: 4, Height: 8}, Fill: d2scene.SolidPaint{Color: color.NRGBA{A: 255}}})}
			maskRoot.Children[1].Opacity = .75
			group := d2scene.NewNode(nil)
			group.Blend = m.mode
			group.Opacity = opacity
			group.Mask = &d2scene.Mask{Type: d2scene.MaskLuminance, Root: maskRoot, Transform: d2scene.Identity()}
			group.Children = []*d2scene.Node{d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{Width: 8, Height: 8}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 128, G: 64, B: 255, A: 128}}})}
			root := d2scene.NewNode(nil)
			root.Children = []*d2scene.Node{background, group}
			frame, err := Render(context.Background(), d2scene.NewDocument(d2scene.Box{Width: 8, Height: 8}, root), testOptions())
			if err != nil {
				t.Fatal(err)
			}
			rows = append(rows, row{m.name, opacity, frame.Pix})
		}
	}
	d2rsWriteJSON(t, out, rows, true)
}

func d2rsCodeFont(t *testing.T) []byte {
	font, err := os.ReadFile("../d2fonts/ttf/SourceCodePro-Regular.ttf")
	if err != nil {
		t.Fatal(err)
	}
	return font
}

func d2rsClipBox(w, h float64) *d2scene.Clip {
	return &d2scene.Clip{Path: d2scene.Path{Commands: []d2scene.PathCommand{d2scene.MoveTo(0, 0), d2scene.LineTo(w, 0), d2scene.LineTo(w, h), d2scene.LineTo(0, h), d2scene.ClosePath()}}, Transform: d2scene.Identity()}
}

// markdown_reference_probe_test.go
func TestD2RSMarkdownReference(t *testing.T) {
	out := d2rsOut(t)
	family := d2fonts.SourceSansPro
	mono := d2fonts.SourceCodePro
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	layout, err := textmeasure.LayoutMarkdown("`Agm`", ruler, &family, &mono, 16)
	if err != nil {
		t.Fatal(err)
	}
	font := d2rsCodeFont(t)
	root := d2scene.NewNode(nil)
	root.Transform = d2scene.Translate(float64(220-layout.Width)/2, float64(120-layout.Height)/2)
	root.Clip = d2rsClipBox(float64(layout.Width), float64(layout.Height))
	for _, p := range layout.Primitives {
		switch p.Kind {
		case textmeasure.MarkdownRectPrimitive:
			root.Children = append(root.Children, d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{X: p.X, Y: p.Y, Width: p.Width, Height: p.Height}, RadiusX: p.Radius, RadiusY: p.Radius, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 238, G: 241, B: 248, A: 255}}}))
		case textmeasure.MarkdownTextPrimitive:
			root.Children = append(root.Children, d2scene.NewNode(d2scene.TextRun{Text: p.Text, Origin: d2scene.Point{X: p.X, Y: p.Y}, Font: d2scene.Font{Asset: "font", Family: "Source Code Pro", Size: p.FontSize}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 10, G: 15, B: 37, A: 255}}, Underline: true, Ink: d2scene.NewBounds(p.X, p.Y-p.Height, p.X+p.Width, p.Y+p.FontSize*.25)}))
		}
	}
	document := d2scene.NewDocument(d2scene.Box{X: -21, Y: -21, Width: 262, Height: 162}, root)
	document.Assets["font"] = d2scene.FontAsset{MIMEType: "font/ttf", Data: font}
	options := testOptions()
	options.Scale = 2
	options.Background = color.NRGBA{R: 255, G: 255, B: 255, A: 255}
	p, err := prepare(context.Background(), document, options)
	if err != nil {
		t.Fatal(err)
	}
	if err := setBandReferenceBounds(context.Background(), p.root, image.Rect(0, 0, p.width, p.height)); err != nil {
		t.Fatal(err)
	}
	type row struct {
		Bounds    any  `json:"bounds"`
		Reference any  `json:"reference"`
		Clip      bool `json:"clip"`
		Transform any  `json:"transform"`
	}
	rows := []row{}
	var walk func(*preparedNode)
	walk = func(n *preparedNode) {
		r := row{Bounds: n.bounds, Clip: n.clip != nil}
		if n.primitive != nil {
			r.Reference = n.primitive.referenceBounds
			r.Transform = n.primitive.transform
		}
		rows = append(rows, r)
		for _, child := range n.children {
			walk(child)
		}
	}
	walk(p.root)
	d2rsWriteJSON(t, out, rows, true)
}

// markdown_reference_matrix_probe_test.go (frames=false) and
// tools/probes/markdown-clip-pixels-source-20261008.go (frames=true).
func d2rsMarkdownMatrix(t *testing.T, out string, frames bool) {
	family := d2fonts.SourceCodePro
	ruler, err := textmeasure.NewRuler()
	if err != nil {
		t.Fatal(err)
	}
	font := d2rsCodeFont(t)
	type result struct {
		SVG       string          `json:"svg"`
		Scale     float64         `json:"scale"`
		Reference image.Rectangle `json:"reference"`
	}
	type framed struct {
		SVG       string          `json:"svg"`
		Scale     float64         `json:"scale"`
		Reference image.Rectangle `json:"reference"`
		Width     int             `json:"width"`
		Height    int             `json:"height"`
		RGBA      string          `json:"rgba_sha256"`
	}
	rows := []any{}
	for _, source := range []string{"`Agm`", "Agm", "plain `Agm` text", "`Agm`  \n`gy`"} {
		for _, size := range []int{12, 23, 32} {
			for _, scale := range []float64{1, 2} {
				for _, position := range [][2]float64{{91, 48}, {-24, -12}} {
					for _, underline := range []bool{false, true} {
						layout, err := textmeasure.LayoutMarkdown(source, ruler, &family, &family, size)
						if err != nil {
							t.Fatal(err)
						}
						root := d2scene.NewNode(nil)
						root.Transform = d2scene.Translate(position[0], position[1])
						root.Clip = d2rsClipBox(float64(layout.Width), float64(layout.Height))
						var svg strings.Builder
						fmt.Fprintf(&svg, `<svg xmlns="http://www.w3.org/2000/svg" width="262" height="162" viewBox="0 0 262 162"><svg id="d2-rs-native-canvas-viewport-main" class="d2-svg" data-d2-rs-canvas-reference="ordinary" width="262" height="162" viewBox="-21 -21 262 162"><svg id="d2-rs-native-md-viewport-control" data-d2-rs-markdown-reference="ordinary" x="%g" y="%g" width="%.3f" height="%.3f" viewBox="0 0 %.3f %.3f" overflow="hidden"><g>`, position[0], position[1], float64(layout.Width), float64(layout.Height), float64(layout.Width), float64(layout.Height))
						for i, p := range layout.Primitives {
							switch p.Kind {
							case textmeasure.MarkdownRectPrimitive:
								root.Children = append(root.Children, d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{X: p.X, Y: p.Y, Width: p.Width, Height: p.Height}, RadiusX: p.Radius, RadiusY: p.Radius, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 238, G: 241, B: 248, A: 255}}}))
								fmt.Fprintf(&svg, `<rect id="d2-rs-native-md-rect-%d" x="%g" y="%g" width="%g" height="%g" rx="%g" fill="#eef1f8"/>`, i, p.X, p.Y, p.Width, p.Height, p.Radius)
							case textmeasure.MarkdownTextPrimitive:
								root.Children = append(root.Children, d2scene.NewNode(d2scene.TextRun{Text: p.Text, Origin: d2scene.Point{X: p.X, Y: p.Y}, Font: d2scene.Font{Asset: "font", Family: "Source Code Pro", Size: p.FontSize}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 10, G: 15, B: 37, A: 255}}, Underline: underline, Ink: d2scene.NewBounds(p.X, p.Y-p.Height, p.X+p.Width, p.Y+p.FontSize*.25)}))
								decoration := ""
								if underline {
									decoration = ` text-decoration="underline"`
								}
								fmt.Fprintf(&svg, `<text xml:space="preserve" id="native-%d" x="%g" y="%g" data-d2-rs-text-x-bits="%016x" data-d2-rs-text-y-bits="%016x" font-family="Source Code Pro" font-size="%g" fill="#0a0f25"%s>%s</text>`, i, p.X, p.Y, math.Float64bits(p.X), math.Float64bits(p.Y), p.FontSize, decoration, p.Text)
							}
						}
						svg.WriteString(`</g></svg></svg></svg>`)
						document := d2scene.NewDocument(d2scene.Box{X: -21, Y: -21, Width: 262, Height: 162}, root)
						document.Assets["font"] = d2scene.FontAsset{MIMEType: "font/ttf", Data: font}
						options := testOptions()
						options.Scale = scale
						if frames {
							options.Background = color.NRGBA{R: 255, G: 255, B: 255, A: 255}
						}
						prepared, err := prepare(context.Background(), document, options)
						if err != nil {
							t.Fatal(err)
						}
						if err := setBandReferenceBounds(context.Background(), prepared.root, image.Rect(0, 0, prepared.width, prepared.height)); err != nil {
							t.Fatal(err)
						}
						reference := image.Rectangle{}
						for _, child := range prepared.root.children {
							if child.primitive != nil {
								reference = child.primitive.referenceBounds
								break
							}
						}
						if !frames {
							rows = append(rows, result{svg.String(), scale, reference})
							continue
						}
						frame, err := Render(context.Background(), document, options)
						if err != nil {
							t.Fatal(err)
						}
						rows = append(rows, framed{svg.String(), scale, reference, frame.Bounds().Dx(), frame.Bounds().Dy(), fmt.Sprintf("%x", sha256.Sum256(frame.Pix))})
					}
				}
			}
		}
	}
	d2rsWriteJSON(t, out, rows, true)
}

func TestD2RSMarkdownReferenceMatrix(t *testing.T) { d2rsMarkdownMatrix(t, d2rsOut(t), false) }
func TestD2RSMarkdownClipPixels(t *testing.T)     { d2rsMarkdownMatrix(t, d2rsOut(t), true) }

// codex_raster_bands_probe_test.go.overlay; each row also carries the committed
// Rust-side SVG rendition of its mode (tools/fixtures/inputs/renderexport/raster-bands-svgs.json).
func TestD2RSRasterBands(t *testing.T) {
	out := d2rsOut(t)
	seed, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	svgs := map[string]string{}
	if err := json.Unmarshal(seed, &svgs); err != nil {
		t.Fatal(err)
	}
	rows := []map[string]any{}
	face, ok := d2fonts.FontFaces.Lookup(d2fonts.Font{Family: d2fonts.SourceSansPro, Style: d2fonts.FONT_STYLE_REGULAR})
	if !ok {
		t.Fatal("font")
	}
	for _, mode := range []string{"plain", "opacity", "nested", "multiply", "mask", "clip", "shadow"} {
		for _, scale := range []int{1, 2} {
			first := d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{X: 10.25, Y: 110.75, Width: 110.5, Height: 100.75}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 237, G: 240, B: 253, A: 255}}, Stroke: &d2scene.Stroke{Paint: d2scene.SolidPaint{Color: color.NRGBA{R: 13, G: 50, B: 178, A: 255}}, Width: 2, Join: d2scene.JoinRound, Cap: d2scene.CapButt, MiterLimit: 4}})
			second := d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{X: 60.125, Y: 180.25, Width: 120.25, Height: 100.5}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 230, G: 90, B: 40, A: 255}}})
			label := d2scene.NewNode(d2scene.TextRun{Text: "default", Origin: d2scene.Point{X: 150.25, Y: 300.6666666666667}, Anchor: d2scene.AnchorMiddle, Font: d2scene.Font{Asset: "font", Size: 24}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 10, G: 15, B: 37, A: 255}}})
			g := d2scene.NewNode(nil)
			g.Children = []*d2scene.Node{first, second, label}
			switch mode {
			case "opacity":
				g.Opacity = .6
			case "nested":
				inner := d2scene.NewNode(nil)
				inner.Opacity = .7
				inner.Children = g.Children
				g.Children = []*d2scene.Node{inner}
				g.Opacity = .6
			case "multiply":
				inner := d2scene.NewNode(nil)
				inner.Blend = d2scene.BlendMultiply
				inner.Children = []*d2scene.Node{second, label}
				g.Children = []*d2scene.Node{first, inner}
			case "mask":
				g.Mask = &d2scene.Mask{Type: d2scene.MaskLuminance, Transform: d2scene.Identity(), Root: d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{Width: 260, Height: 400}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 255, G: 255, B: 255, A: 191}}})}
			case "clip":
				g.Clip = &d2scene.Clip{Transform: d2scene.Identity(), Path: d2scene.Path{Commands: []d2scene.PathCommand{d2scene.MoveTo(20, 90), d2scene.LineTo(190, 90), d2scene.LineTo(190, 310), d2scene.LineTo(20, 310), d2scene.ClosePath()}}}
			case "shadow":
				g.Filters = []d2scene.Filter{d2scene.DropShadow{OffsetX: 3, OffsetY: 5, Color: color.NRGBA{R: 61, G: 69, B: 116, A: 102}}}
				g.Children = []*d2scene.Node{first, second}
			}
			doc := d2scene.NewDocument(d2scene.Box{Width: 260, Height: 400}, g)
			doc.Assets["font"] = d2scene.FontAsset{MIMEType: "font/ttf", Data: face}
			opts := testOptions()
			opts.Scale = float64(scale)
			opts.Background = color.White
			for _, band := range []int{0, 64, 256} {
				var frame *image.NRGBA
				var err error
				if band == 0 {
					frame, err = Render(context.Background(), doc, opts)
				} else {
					frame = image.NewNRGBA(image.Rect(0, 0, 260*scale, 400*scale))
					err = RenderBands(context.Background(), doc, opts, band, func(part *image.NRGBA) error {
						draw.Draw(frame, part.Bounds(), part, part.Bounds().Min, draw.Src)
						return nil
					})
				}
				if err != nil {
					t.Fatal(mode, scale, band, err)
				}
				rows = append(rows, map[string]any{"mode": mode, "scale": scale, "band_height": band, "width": frame.Bounds().Dx(), "height": frame.Bounds().Dy(), "rgba_sha256": fmt.Sprintf("%x", sha256.Sum256(frame.Pix)), "svg": svgs[mode]})
			}
		}
	}
	d2rsWriteJSON(t, out, map[string]any{"probe": "codex_raster_bands_probe_test.go.overlay", "cases": rows}, true)
}

var d2rsImportLimits = d2svgimport.Limits{MaxBytes: 1 << 20, MaxDepth: 64, MaxElements: 1000, MaxAttributes: 10000, MaxAttributeBytes: 1 << 20, MaxPathCommands: 10000, MaxTransformFunctions: 1000, MaxUseDepth: 32, MaxResources: 1000}

// codex_imported_primitives_test.go.overlay
func TestD2RSImportedPrimitives(t *testing.T) {
	out := d2rsOut(t)
	rows := []map[string]any{}
	bodies := map[string]string{
		"ellipse":  `<ellipse fill="#5ee1eb" cx="63.976" cy="56.766" rx="22.11" ry="12.555"/>`,
		"circle":   `<circle fill="#2db357" cx="61.333333333333336" cy="53.875" r="21.125"/>`,
		"polygon":  `<polygon fill="#0e834c" points="10.125,20.625 90.33333333333333,44.125 113.75,95.125 55.976,102.766"/>`,
		"polyline": `<polyline fill="#2db357" points="10.125,20.625 90.33333333333333,44.125 113.75,95.125 55.976,102.766"/>`,
		"mixed":    `<polygon fill="#2db357" points="10.125,20.625 90.33333333333333,44.125 113.75,95.125 55.976,102.766"/><ellipse fill="#5ee1eb" cx="63.976" cy="56.766" rx="22.11" ry="12.555"/><circle fill="#0e834c" cx="55.976" cy="64.766" r="8.125"/>`,
		"rotated":  `<g transform="matrix(0 1 -1 0 128 0)"><ellipse fill="#5ee1eb" cx="63.976" cy="56.766" rx="22.11" ry="12.555"/><polygon fill="#0e834c" points="10.125,20.625 90.33333333333333,44.125 113.75,95.125 55.976,102.766"/></g>`,
	}
	for _, mode := range []string{"ellipse", "circle", "polygon", "polyline", "mixed", "rotated"} {
		svg := `<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128" viewBox="0 0 128 128">` + bodies[mode] + `</svg>`
		imported, err := d2svgimport.ImportNode(context.Background(), "control.svg", []byte(svg), d2rsImportLimits)
		if err != nil {
			t.Fatal(err)
		}
		node := d2scene.NewNode(d2scene.Image{Asset: "control", Box: d2scene.Box{X: 16, Y: 32, Width: 128, Height: 128}, Aspect: d2scene.AspectRatio{Align: d2scene.AlignXMidYMid, Fit: d2scene.AspectMeet}})
		doc := d2scene.NewDocument(d2scene.Box{Width: 192, Height: 192}, node)
		doc.Assets["control"] = d2scene.VectorAsset{Root: imported.Root, ViewBox: imported.ViewBox}
		for _, scale := range []int{1, 2} {
			opts := testOptions()
			opts.Scale = float64(scale)
			opts.Background = color.White
			frame, err := Render(context.Background(), doc, opts)
			if err != nil {
				t.Fatal(err)
			}
			rows = append(rows, map[string]any{"mode": mode, "scale": scale, "asset_svg": svg, "width": frame.Bounds().Dx(), "height": frame.Bounds().Dy(), "rgba_sha256": fmt.Sprintf("%x", sha256.Sum256(frame.Pix))})
		}
	}
	d2rsWriteJSON(t, out, rows, true)
}

// d2rsMathFrame: codex_math_vector_test.go.overlay / tools/probes/nested-math-clip-20261007.go.
func d2rsMathFrame(t *testing.T, svg string) (*d2scene.Document, float64, float64) {
	ink := color.NRGBA{R: 10, G: 15, B: 37, A: 255}
	imported, err := d2svgimport.ImportNodeWithOptions(context.Background(), "math.svg", []byte(svg), d2rsImportLimits, d2svgimport.ImportOptions{CurrentColor: &ink})
	if err != nil {
		t.Fatal(err)
	}
	content := d2scene.NewNode(nil)
	content.Transform = imported.ViewportTransform
	content.Children = []*d2scene.Node{imported.Root}
	viewport := d2scene.Box{Width: imported.Width, Height: imported.Height}
	root := d2scene.NewNode(nil)
	root.Children = []*d2scene.Node{content}
	root.Clip = d2rsClipBox(viewport.Width, viewport.Height)
	node := d2scene.NewNode(d2scene.Image{Asset: "math", Box: d2scene.Box{X: 20.25, Y: 40.5, Width: imported.Width, Height: imported.Height}, Aspect: d2scene.AspectRatio{Align: d2scene.AlignXMidYMid, Fit: d2scene.AspectMeet}})
	doc := d2scene.NewDocument(d2scene.Box{Width: 192, Height: 160}, node)
	doc.Assets["math"] = d2scene.VectorAsset{Root: root, ViewBox: viewport}
	return doc, imported.Width, imported.Height
}

// codex_math_vector_test.go.overlay (formulas rendered by d2latex), or the
// tools/probes/nested-math-clip-20261007.go request list ($D2RS_RECIPE_INPUT:
// [{"name", "formula" (rendered by d2latex here) or a hand-authored "svg"}]).
func TestD2RSMathVectorFrames(t *testing.T) {
	out := d2rsOut(t)
	type request struct {
		Name    string `json:"name"`
		Formula string `json:"formula"`
		SVG     string `json:"svg"`
	}
	requests := []request{}
	nested := os.Getenv("D2RS_RECIPE_INPUT") != ""
	if nested {
		data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
		if err != nil {
			t.Fatal(err)
		}
		if err := json.Unmarshal(data, &requests); err != nil {
			t.Fatal(err)
		}
	} else {
		for i, f := range []string{`1 + 1`, `\frac{1}{2}`, `a + b = c`, `\sqrt{x^2+y^2}`, `\sin(x)`, `\frac{x+1}{y+2}`} {
			requests = append(requests, request{Name: fmt.Sprintf("math%d", i), Formula: f})
		}
	}
	rows := []map[string]any{}
	for _, r := range requests {
		svg := r.SVG
		if svg == "" {
			var err error
			if svg, err = d2latex.Render(r.Formula); err != nil {
				t.Fatal(err)
			}
		}
		doc, w, h := d2rsMathFrame(t, svg)
		for _, scale := range []int{1, 2} {
			opts := testOptions()
			opts.Scale = float64(scale)
			opts.Background = color.White
			frame, err := Render(context.Background(), doc, opts)
			if err != nil {
				t.Fatal(err)
			}
			row := map[string]any{"name": fmt.Sprintf("%s-scale%d", r.Name, scale), "asset_svg": svg, "intrinsic_width": w, "intrinsic_height": h, "scale": scale, "width": frame.Bounds().Dx(), "height": frame.Bounds().Dy(), "rgba_sha256": fmt.Sprintf("%x", sha256.Sum256(frame.Pix))}
			if !nested {
				row["formula"] = r.Formula
			}
			rows = append(rows, row)
		}
	}
	d2rsWriteJSON(t, out, rows, true)
}

// tools/probes/opaque-arrow-mask-20261004.go: complete NRGBA frames, direct and masked.
func TestD2RSOpaqueArrow(t *testing.T) {
	d2rsOut(t)
	outputs := filepath.SplitList(os.Getenv("D2RS_RECIPE_OUTPUTS"))
	for i, masked := range []bool{false, true} {
		paint := d2scene.SolidPaint{Color: color.NRGBA{R: 13, G: 50, B: 178, A: 255}}
		group := d2scene.NewNode(nil)
		group.Children = []*d2scene.Node{
			d2scene.NewNode(d2scene.Path{Commands: []d2scene.PathCommand{d2scene.MoveTo(20, 10), d2scene.LineTo(20, 54)}, Stroke: &d2scene.Stroke{Paint: paint, Width: 2, Cap: d2scene.CapRound, Join: d2scene.JoinRound, MiterLimit: 4}}),
			d2scene.NewNode(d2scene.Path{Commands: []d2scene.PathCommand{d2scene.MoveTo(14, 47), d2scene.LineTo(20, 57), d2scene.LineTo(26, 47), d2scene.ClosePath()}, Fill: paint}),
		}
		if masked {
			mask := d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{Width: 80, Height: 80}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 255, G: 255, B: 255, A: 255}}})
			group.Mask = &d2scene.Mask{Root: mask, Type: d2scene.MaskLuminance, Transform: d2scene.Identity()}
		}
		document := d2scene.NewDocument(d2scene.Box{Width: 80, Height: 80}, group)
		options := testOptions()
		options.Scale = 2
		options.Background = color.White
		frame, err := Render(context.Background(), document, options)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(outputs[i], frame.Pix, 0o644); err != nil {
			t.Fatal(err)
		}
	}
}

// opacity_overhanging_probe.go.overlay (mode overhang: three origins),
// effect-text-bounds-source-probe.go (bounds) and native-underline-source-probe.go
// (underline: text only, no box/opacity/offset). $D2RS_EFFECT_MODE selects.
func TestD2RSEffectText(t *testing.T) {
	out := d2rsOut(t)
	mode := os.Getenv("D2RS_EFFECT_MODE")
	origins := [][2]float64{{70.25, 46.5}}
	if mode == "overhang" {
		origins = [][2]float64{{70.25, -15.5}, {-20.125, 46.5}, {70.25, 46.5}}
	}
	rows := []map[string]any{}
	cases := []struct {
		w, h, scale float64
		center      bool
	}{{428, 353, 2, false}, {428, 353, 2, true}, {342, 282, 2, false}, {342, 282, 2, true}, {114, 94, 2, false}, {114, 94, 2, true}}
	for face := 0; face < 4; face++ {
		family, style := d2fonts.SourceSansPro, d2fonts.FONT_STYLE_REGULAR
		if face == 1 {
			style = d2fonts.FONT_STYLE_BOLD
		}
		if face >= 2 {
			family = d2fonts.SourceCodePro
		}
		if face == 3 {
			style = d2fonts.FONT_STYLE_BOLD
		}
		fontBytes, ok := d2fonts.FontFaces.Lookup(d2fonts.Font{Family: family, Style: style})
		if !ok {
			t.Fatal("font missing")
		}
		for _, origin := range origins {
			for i, c := range cases {
				offsetX, offsetY := 0.0, 0.0
				path := d2scene.NewNode(d2scene.TextRun{Text: "gy p  ", Underline: true, Origin: d2scene.Point{X: origin[0], Y: origin[1]}, Anchor: d2scene.AnchorMiddle, Font: d2scene.Font{Size: 16, Asset: "font"}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 10, G: 15, B: 37, A: 255}}})
				root := d2scene.NewNode(nil)
				if mode == "underline" {
					root.Children = []*d2scene.Node{path}
				} else {
					box := d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{X: 4.25, Y: 7.125, Width: 140.5, Height: 80.75}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 247, G: 248, B: 254, A: 255}}, Stroke: &d2scene.Stroke{Paint: d2scene.SolidPaint{Color: color.NRGBA{R: 13, G: 50, B: 178, A: 255}}, Width: 2, Join: d2scene.JoinRound}})
					root.Children = []*d2scene.Node{box, path}
					root.Opacity = .6
					offsetX, offsetY = 13.125, 17.25
				}
				root.Transform = d2scene.Translate(offsetX, offsetY)
				doc := d2scene.NewDocument(d2scene.Box{X: -101, Y: -101, Width: 342, Height: 282}, root)
				doc.Assets["font"] = d2scene.FontAsset{MIMEType: "font/ttf", Data: fontBytes}
				doc.LogicalWidth = c.w
				doc.LogicalHeight = c.h
				doc.ViewportFit = d2scene.ViewportMeet
				if c.center {
					doc.ViewportAlign = d2scene.ViewportAlignXMidYMid
				}
				opts := FrameOptions{Scale: 1, Background: color.White, MaxWidth: 1000, MaxHeight: 1000, MaxPixels: 1000000, MaxNodes: 1000, MaxDepth: 100, MaxPathCommands: 100000, MaxTextRunesPerRun: 10000, MaxAnimationTracks: 10000, MaxAnimationKeyframes: 1000000, MaxAssets: 1000, MaxAssetBytes: 64 * 1024 * 1024, MaxDecodedAssetBytes: 64 * 1024 * 1024, MaxImportDepth: 32, MaxOffscreenBytes: 64 * 1024 * 1024, MaxEvenOddClipWork: 1000000000}
				opts.MaxWidth = 4000
				opts.MaxHeight = 4000
				opts.MaxPixels = 16000000
				opts.Scale = c.scale
				frame, err := Render(context.Background(), doc, opts)
				if err != nil {
					t.Fatal(err)
				}
				digest := sha256.New()
				for y := 0; y < frame.Bounds().Dy(); y++ {
					for x := 0; x < frame.Bounds().Dx(); x++ {
						pixel := frame.NRGBAAt(x, y)
						digest.Write([]byte{pixel.R, pixel.G, pixel.B, pixel.A})
					}
				}
				row := map[string]any{"case": i, "face": face, "center": c.center, "logical_width": c.w, "logical_height": c.h, "scale": c.scale, "offset_x": offsetX, "offset_y": offsetY, "width": frame.Bounds().Dx(), "height": frame.Bounds().Dy(), "rgba_sha256": hex.EncodeToString(digest.Sum(nil))}
				if mode == "overhang" {
					row["origin_x"], row["origin_y"] = origin[0], origin[1]
				}
				rows = append(rows, row)
			}
		}
	}
	value := map[string]any{"source": "d2raster.Render at the manifest pin; complete NRGBA bytes (" + mode + ")", "controls": rows}
	if mode != "overhang" {
		value["text"] = "gy p  "
	}
	d2rsWriteJSON(t, out, value, true)
}

// math-braket-raster-native-20261012/add-contracts.py: flattenTolerance for the
// 30 imported-math path matrices of three braket boards
// (tools/fixtures/inputs/renderexport/svg-clip-tolerance-matrices.json).
func TestD2RSClipTolerance(t *testing.T) {
	out := d2rsOut(t)
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var rows []struct {
		Case      int            `json:"case"`
		Path      int            `json:"path"`
		Matrix    d2scene.Matrix `json:"matrix"`
		Tolerance float64        `json:"tolerance"`
	}
	if err := json.Unmarshal(data, &rows); err != nil {
		t.Fatal(err)
	}
	for i := range rows {
		rows[i].Tolerance = flattenTolerance(rows[i].Matrix)
	}
	d2rsWriteJSON(t, out, rows, true)
}
