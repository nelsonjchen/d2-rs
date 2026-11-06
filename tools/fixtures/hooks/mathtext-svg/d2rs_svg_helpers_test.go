// SPDX-License-Identifier: MPL-2.0
// Shared encoders for the d2svgimport capture hooks (tools/fixtures/mathtext-svg.toml).
// Every mathtext-svg recipe overlays this file next to its own hook.
package d2svgimport

import (
	"context"
	"encoding/json"
	"math"
	"os"
	"testing"
	"time"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

// d2rsOutput returns the staged output path, skipping outside the runner.
func d2rsOutput(t *testing.T) string {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	return out
}

// d2rsReadInput decodes the recipe's generated input into v.
func d2rsReadInput(t *testing.T, v any) {
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, v); err != nil {
		t.Fatal(err)
	}
}

// d2rsWrite writes rows as two-space indented JSON without a trailing newline.
func d2rsWrite(t *testing.T, out string, rows any) {
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, data, 0o644); err != nil {
		t.Fatal(err)
	}
}

// d2rsContext returns a live, cancelled ("cancelled") or expired ("deadline") context.
func d2rsContext(t *testing.T, mode string) context.Context {
	ctx := context.Background()
	switch mode {
	case "":
	case "cancelled":
		var cancel context.CancelFunc
		ctx, cancel = context.WithCancel(ctx)
		cancel()
	case "deadline":
		var cancel context.CancelFunc
		ctx, cancel = context.WithDeadline(ctx, time.Unix(0, 0))
		t.Cleanup(cancel)
	default:
		t.Fatalf("unknown cancel mode %q", mode)
	}
	return ctx
}

func d2rsMatrix(m d2scene.Matrix) [6]float64 {
	return [6]float64{m.A, m.B, m.C, m.D, m.E, m.F}
}

func d2rsPaint(p d2scene.Paint) any {
	switch p := p.(type) {
	case d2scene.SolidPaint:
		return map[string]any{"kind": "solid", "color": [4]uint8{p.Color.R, p.Color.G, p.Color.B, p.Color.A}}
	case d2scene.LinearGradient:
		stops := []map[string]any{}
		for _, s := range p.Stops {
			stops = append(stops, map[string]any{"offset": s.Offset, "color": [4]uint8{s.Color.R, s.Color.G, s.Color.B, s.Color.A}})
		}
		return map[string]any{"kind": "linear", "start": [2]float64{p.Start.X, p.Start.Y}, "end": [2]float64{p.End.X, p.End.Y},
			"stops": stops, "units": p.Units, "transform": d2rsMatrix(p.Transform), "spread": p.Spread}
	}
	return nil
}

func d2rsCommands(commands []d2scene.PathCommand) []map[string]any {
	out := []map[string]any{}
	for _, c := range commands {
		out = append(out, map[string]any{"kind": c.Kind, "points": [3][2]float64{{c.P1.X, c.P1.Y}, {c.P2.X, c.P2.Y}, {c.P3.X, c.P3.Y}},
			"radii": [2]float64{c.RadiusX, c.RadiusY}, "rotation": c.Rotation, "large_arc": c.LargeArc, "sweep": c.Sweep})
	}
	return out
}

func d2rsGeometry(g svgGeometry) any {
	return map[string]any{"kind": g.kind, "bounds": [4]float64{g.box.X, g.box.Y, g.box.Width, g.box.Height},
		"center": [2]float64{g.center.X, g.center.Y}, "radii": [2]float64{g.radiusX, g.radiusY},
		"commands": d2rsCommands(g.commands), "force_no_fill": g.forceNoFill}
}

func d2rsStroke(s *d2scene.Stroke) any {
	if s == nil {
		return nil
	}
	return map[string]any{"paint": d2rsPaint(s.Paint), "width": s.Width, "cap": s.Cap, "join": s.Join,
		"miter_limit": s.MiterLimit, "dashes": s.Dashes, "dash_offset": s.DashOffset}
}

func d2rsStyle(s svgStyle) any {
	dashes := append([]float64{}, s.dashes...)
	return map[string]any{"paint": map[string]any{"visible": s.visible, "fill": d2rsPaint(s.fill), "stroke": d2rsPaint(s.stroke),
		"fill_current": s.fillCurrent, "stroke_current": s.strokeCurrent, "color": [4]uint8{s.color.R, s.color.G, s.color.B, s.color.A},
		"fill_opacity": s.fillOpacity, "stroke_opacity": s.strokeOpacity, "stroke_width": s.strokeWidth, "cap": s.lineCap,
		"join": s.lineJoin, "miter_limit": s.miterLimit, "dashes": dashes, "dash_offset": s.dashOffset, "fill_rule": s.fillRule},
		"opacity": s.opacity, "display": s.display, "clip_rule": s.clipRule, "clip_path": s.clipPathID}
}

func d2rsViewport(e *svgElement) any {
	if e.viewport == nil {
		return nil
	}
	v := e.viewport
	return map[string]any{"view_box": [4]float64{v.viewBox.X, v.viewBox.Y, v.viewBox.Width, v.viewBox.Height}, "width": v.width,
		"height": v.height, "aspect": [2]uint8{uint8(v.aspect.Align), uint8(v.aspect.Fit)}, "transform": d2rsMatrix(v.transform),
		"position": [2]float64{e.viewportX, e.viewportY}}
}

func d2rsGradientStop(s *d2scene.GradientStop) any {
	if s == nil {
		return nil
	}
	return map[string]any{"offset": s.Offset, "color": [4]uint8{s.Color.R, s.Color.G, s.Color.B, s.Color.A}}
}

func d2rsGradient(g *d2scene.LinearGradient) any {
	if g == nil {
		return nil
	}
	return d2rsPaint(*g)
}

func d2rsClip(c *compiledClipPath) any {
	if c == nil {
		return nil
	}
	return map[string]any{"commands": d2rsCommands(c.path.Commands), "fill_rule": c.path.FillRule,
		"transform": d2rsMatrix(c.transform), "uses": c.useInstances}
}

// d2rsCommandBits is the raw IEEE-754 encoding of a command's scalars.
func d2rsCommandBits(values ...float64) [9]uint64 {
	var bits [9]uint64
	for j, v := range values {
		bits[j] = math.Float64bits(v)
	}
	return bits
}

func d2rsError(err error) string {
	if err == nil {
		return ""
	}
	return err.Error()
}
