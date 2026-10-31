// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): d2scenebuild.Build of one 120x80 target shape,
// placed under five affine matrices at scales 1 and 2 and rendered by d2raster;
// complete NRGBA frame digests. Harvested from the v0.9.0-era public-API probes
// .reference/pattern-transforms-20261016/{source,double}-module/main.go (fill
// patterns, plain and double border) and .reference/legend-sketch-20261016/
// source{2,3,4}-module/main.go (sketch rectangle, C4 person, oval); plus
// TestD2RSLegendShadows from .reference/legend-effects-20261016/source-module/main.go.
// $D2RS_SHAPE_FRAME_MODE selects; retargeted to the manifest pin. Inert unless
// run by scripts/regen-fixtures.py.
package d2scenebuild

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
	"github.com/d2lang/d2/d2target"
)

func TestD2RSShapeFrames(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	mode := os.Getenv("D2RS_SHAPE_FRAME_MODE")
	type variant struct {
		kinds  []string // fill patterns or fill colors
		sketch bool
		double bool
		shape  string
		label  string // shape name recorded in rows (sketch C4/oval)
		prefix string
	}
	v, ok := map[string]variant{
		"pattern":        {kinds: []string{"dots", "lines", "grain", "paper"}},
		"pattern-double": {kinds: []string{"dots", "lines", "grain", "paper"}, double: true, prefix: "double-"},
		"sketch":         {kinds: []string{"#f7f8fe", "#ffffff", "#0a0f25", "#ffcc00"}, sketch: true, shape: d2target.ShapeRectangle, prefix: "sketch-"},
		"sketch-c4":      {kinds: []string{"#f7f8fe", "#ffffff", "#0a0f25", "#ffcc00"}, sketch: true, shape: d2target.ShapeC4Person, label: "c4-person", prefix: "sketch-c4-"},
		"sketch-oval":    {kinds: []string{"#f7f8fe", "#ffffff", "#0a0f25", "#ffcc00"}, sketch: true, shape: d2target.ShapeOval, label: "oval", prefix: "sketch-oval-"},
	}[mode]
	if !ok {
		t.Fatalf("unknown D2RS_SHAPE_FRAME_MODE %q", mode)
	}
	rows := []map[string]any{}
	for ki, kind := range v.kinds {
		d := d2target.NewDiagram()
		d.Root.Fill = "transparent"
		d.Root.Stroke = "none"
		s := *d2target.BaseShape()
		s.ID = "probe"
		s.Type = d2target.ShapeRectangle
		s.Width = 120
		s.Height = 80
		if v.sketch {
			s.Type = v.shape
			s.Fill = kind
			s.Stroke = "#000410"
			s.StrokeWidth = 2
		} else {
			s.Fill = "#f7f8fe"
			s.Stroke = "none"
			s.StrokeWidth = 0
			if v.double {
				s.Stroke = "#000410"
				s.StrokeWidth = 2
				s.DoubleBorder = true
			}
			s.FillPattern = kind
		}
		d.Shapes = []d2target.Shape{s}
		pad := int64(0)
		options := Options{Pad: &pad}
		if v.sketch {
			options.Sketch = true
			options.SketchBudget = SketchBudget{MaxOperationSets: 100000, MaxOperations: 1000000, MaxPathCommands: 1000000}
		}
		built, err := Build(context.Background(), d, options)
		if err != nil {
			t.Fatal(err)
		}
		var shape *d2scene.Node
		var find func(*d2scene.Node)
		find = func(n *d2scene.Node) {
			if n.ID == "probe" {
				shape = n
			}
			for _, c := range n.Children {
				find(c)
			}
		}
		find(built.Root)
		if shape == nil {
			t.Fatal("source shape missing")
		}
		for mi, m := range [][6]float64{{.2, 0, 0, .2, 17.25, 13.125}, {.37, 0, 0, .37, 17.25, 13.125}, {.625, 0, 0, .625, 17.25, 13.125}, {.37, 0, 0, .625, 17.25, 13.125}, {.4, .1, -.05, .3, 17.25, 13.125}} {
			for _, scale := range []float64{1, 2} {
				var name string
				if v.sketch {
					name = fmt.Sprintf("%sf%d-m%d-scale%g", v.prefix, ki, mi, scale)
				} else {
					name = fmt.Sprintf("%s%s-m%d-scale%g", v.prefix, kind, mi, scale)
				}
				wrapper := d2scene.NewNode(nil)
				wrapper.Transform = d2scene.Matrix{A: m[0], B: m[1], C: m[2], D: m[3], E: m[4], F: m[5]}
				wrapper.Children = []*d2scene.Node{shape}
				root := d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{Width: 192, Height: 128}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 255, G: 255, B: 255, A: 255}}})
				root.Children = []*d2scene.Node{wrapper}
				doc := d2scene.NewDocument(d2scene.Box{Width: 192, Height: 128}, root)
				doc.Assets = built.Assets
				frame, err := d2raster.Render(context.Background(), doc, d2raster.FrameOptions{Scale: scale, MaxWidth: 1000, MaxHeight: 1000, MaxPixels: 1000000, MaxNodes: 100000, MaxDepth: 100, MaxPathCommands: 1000000, MaxTextRunesPerRun: 10000, MaxAnimationTracks: 10000, MaxAnimationKeyframes: 1000000, MaxAssets: 1000, MaxAssetBytes: 64 << 20, MaxDecodedAssetBytes: 64 << 20, MaxImportDepth: 32, MaxOffscreenBytes: 64 << 20, MaxEvenOddClipWork: 1000000000})
				if err != nil {
					t.Fatal(err)
				}
				w, h := frame.Bounds().Dx(), frame.Bounds().Dy()
				if len(frame.Pix) != w*h*4 {
					t.Fatal("incomplete inventory")
				}
				sum := sha256.Sum256(frame.Pix)
				row := map[string]any{"name": name, "matrix": m, "scale": scale, "width": w, "height": h, "rgba_sha256": hex.EncodeToString(sum[:])}
				if v.sketch {
					row["fill"] = kind
					if v.label != "" {
						row["shape"] = v.label
					}
				} else {
					row["pattern"] = kind
				}
				rows = append(rows, row)
			}
		}
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}

// legend-effects-20261016/source-module: drop-shadowed rect under scale/opacity/radius.
func TestD2RSLegendShadows(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	for _, s := range []float64{.2, .37, .625} {
		for _, o := range []float64{.25, 1} {
			for _, radius := range []float64{0, 7} {
				for _, scale := range []float64{1, 2} {
					name := fmt.Sprintf("s%g-o%g-r%g-scale%g", s, o, radius, scale)
					ink := d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{Width: 120, Height: 80}, RadiusX: radius, RadiusY: radius, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 247, G: 248, B: 254, A: 255}}, Stroke: &d2scene.Stroke{Paint: d2scene.SolidPaint{Color: color.NRGBA{R: 13, G: 50, B: 178, A: 255}}, Width: 2, Cap: d2scene.CapButt, Join: d2scene.JoinRound}})
					ink.ID = "geometry"
					ink.Filters = []d2scene.Filter{d2scene.DropShadow{OffsetX: 3, OffsetY: 5, SigmaX: 1.7, SigmaY: 1.7, Color: color.NRGBA{R: 61, G: 69, B: 116, A: 102}}}
					wrapper := d2scene.NewNode(nil)
					wrapper.Transform = d2scene.Translate(17.25, 13.125).Mul(d2scene.Scale(s, s))
					wrapper.Opacity = o
					wrapper.Children = []*d2scene.Node{ink}
					root := d2scene.NewNode(d2scene.Rect{Box: d2scene.Box{Width: 128, Height: 96}, Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 255, G: 255, B: 255, A: 255}}})
					root.Children = []*d2scene.Node{wrapper}
					doc := d2scene.NewDocument(d2scene.Box{Width: 128, Height: 96}, root)
					frame, err := d2raster.Render(context.Background(), doc, d2raster.FrameOptions{Scale: scale, MaxWidth: 1000, MaxHeight: 1000, MaxPixels: 1000000, MaxNodes: 1000, MaxDepth: 100, MaxPathCommands: 100000, MaxTextRunesPerRun: 10000, MaxAnimationTracks: 10000, MaxAnimationKeyframes: 1000000, MaxAssets: 1000, MaxAssetBytes: 64 << 20, MaxDecodedAssetBytes: 64 << 20, MaxImportDepth: 32, MaxOffscreenBytes: 64 << 20, MaxEvenOddClipWork: 1000000000})
					if err != nil {
						t.Fatal(err)
					}
					w, h := frame.Bounds().Dx(), frame.Bounds().Dy()
					sum := sha256.Sum256(frame.Pix)
					rows = append(rows, map[string]any{"name": name, "local_scale": s, "opacity": o, "radius": radius, "scale": scale, "width": w, "height": h, "rgba_sha256": hex.EncodeToString(sum[:])})
				}
			}
		}
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
