// SPDX-License-Identifier: MPL-2.0
// Authored diagnostic overlay for pinned D2 v0.9.0; not shipped code.
package d2raster

import (
	"context"
	"encoding/json"
	"fmt"
	"image"
	"image/color"
	"math"
	"os"
	"path/filepath"
	"strconv"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/lib/svg"
)

// Render the three dashed Jupyter container rectangles independently of text,
// routes and masks. Vary only the dash precision lost at the SVG/usvg boundary.
func TestFrozenJupyterDashPrecisionWitness(t *testing.T) {
	output := os.Getenv("D2_DASH_WITNESS_OUTPUT")
	if output == "" {
		t.Fatal("D2_DASH_WITNESS_OUTPUT is required")
	}
	dash, gap := svg.GetStrokeDashAttributes(2, 5)
	rounded, err := strconv.ParseFloat(fmt.Sprintf("%.6f", gap), 64)
	if err != nil {
		t.Fatal(err)
	}
	variants := []struct {
		name string
		gap  float64
	}{{"exact", gap}, {"svg-rounded", rounded}, {"usvg-f32", float64(float32(rounded))}}
	boxes := []d2scene.Box{
		{X: 398, Y: 380, Width: 786, Height: 711},
		{X: 1344, Y: 120, Width: 284, Height: 191},
		{X: 1344, Y: 491, Width: 284, Height: 191},
	}
	fills := []color.NRGBA{{R: 255, G: 224, B: 178, A: 255}, {R: 187, G: 222, B: 251, A: 255}, {R: 225, G: 190, B: 231, A: 255}}
	strokes := []color.NRGBA{{R: 245, G: 124, B: 0, A: 255}, {R: 21, G: 101, B: 192, A: 255}, {R: 142, G: 36, B: 170, A: 255}}
	frames := map[string]*image.NRGBA{}
	variantReports := []map[string]any{}
	for _, variant := range variants {
		root := d2scene.NewNode(nil)
		// Preserve the actual shared container backdrop beneath antialiased
		// dash endpoints; otherwise colors outside each child box differ.
		root.Children = append(root.Children, d2scene.NewNode(d2scene.Rect{
			Box: d2scene.Box{X: 278, Y: 0, Width: 1470, Height: 1211},
			Fill: d2scene.SolidPaint{Color: color.NRGBA{R: 227, G: 242, B: 253, A: 255}},
			Stroke: &d2scene.Stroke{Paint: d2scene.SolidPaint{Color: color.NRGBA{R: 21, G: 101, B: 192, A: 255}},
				Width: 3, Cap: d2scene.CapButt, Join: d2scene.JoinRound, MiterLimit: 4},
		}))
		boxReports := []map[string]any{}
		for index, box := range boxes {
			root.Children = append(root.Children, d2scene.NewNode(d2scene.Rect{
				Box: box,
				Fill: d2scene.SolidPaint{Color: fills[index]},
				Stroke: &d2scene.Stroke{Paint: d2scene.SolidPaint{Color: strokes[index]}, Width: 2,
					Cap: d2scene.CapButt, Join: d2scene.JoinRound, MiterLimit: 4, Dashes: []float64{dash, variant.gap}},
			}))
			points := []d2scene.Point{{X: box.X, Y: box.Y}, {X: box.X + box.Width, Y: box.Y},
				{X: box.X + box.Width, Y: box.Y + box.Height}, {X: box.X, Y: box.Y + box.Height}}
			runs, err := makeStrokeRuns(context.Background(), subpath{points: points, closed: true}, []float64{dash, variant.gap}, 0, func() error { return nil })
			if err != nil {
				t.Fatal(err)
			}
			first := []map[string]any{}
			for _, run := range runs[:min(3, len(runs))] {
				bits := []map[string]string{}
				for _, point := range run.points {
					bits = append(bits, map[string]string{"x": fmt.Sprintf("%016x", math.Float64bits(point.X)), "y": fmt.Sprintf("%016x", math.Float64bits(point.Y))})
				}
				first = append(first, map[string]any{"points": run.points, "bits": bits, "closed": run.closed})
			}
			boxReports = append(boxReports, map[string]any{"box": box, "run_count": len(runs), "first_three_runs": first})
		}
		document := d2scene.NewDocument(d2scene.Box{X: -102, Y: -102, Width: 1952, Height: 1415}, root)
		options := testOptions()
		options.Scale, options.MaxWidth, options.MaxHeight, options.MaxPixels = 2, 4000, 3000, 12_000_000
		options.MaxOffscreenBytes = 256 * 1024 * 1024
		options.Background = color.White
		frame, err := Render(context.Background(), document, options)
		if err != nil {
			t.Fatal(err)
		}
		frames[variant.name] = frame
		png, err := EncodePNG(context.Background(), frame)
		if err != nil {
			t.Fatal(err)
		}
		if err := os.WriteFile(filepath.Join(output, variant.name+".png"), png, 0644); err != nil {
			t.Fatal(err)
		}
		variantReports = append(variantReports, map[string]any{"name": variant.name, "dash": dash, "gap": variant.gap,
			"dash_bits": fmt.Sprintf("%016x", math.Float64bits(dash)), "gap_bits": fmt.Sprintf("%016x", math.Float64bits(variant.gap)), "boxes": boxReports})
	}
	comparisons := []map[string]any{}
	for _, name := range []string{"svg-rounded", "usvg-f32"} {
		baseline, candidate := frames["exact"], frames[name]
		changed := []map[string]any{}
		count := 0
		for y := baseline.Bounds().Min.Y; y < baseline.Bounds().Max.Y; y++ {
			for x := baseline.Bounds().Min.X; x < baseline.Bounds().Max.X; x++ {
				before, after := baseline.NRGBAAt(x, y), candidate.NRGBAAt(x, y)
				if before != after {
					count++
					if len(changed) < 256 {
						changed = append(changed, map[string]any{"x": x, "y": y, "exact": before, "candidate": after})
					}
				}
			}
		}
		comparisons = append(comparisons, map[string]any{"variant": name, "different_pixels": count, "first_256_pixels": changed})
	}
	report := map[string]any{"scope": "Actual pinned Go native raster, varying only dash precision on the three Jupyter rectangles",
		"variants": variantReports, "comparisons": comparisons, "width": frames["exact"].Bounds().Dx(), "height": frames["exact"].Bounds().Dy()}
	encoded, err := json.MarshalIndent(report, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(filepath.Join(output, "go-witness.json"), append(encoded, '\n'), 0644); err != nil {
		t.Fatal(err)
	}
	t.Logf("exact gap %.17g (%016x), SVG %.17g, usvg %.17g", gap, math.Float64bits(gap), rounded, float64(float32(rounded)))
}
