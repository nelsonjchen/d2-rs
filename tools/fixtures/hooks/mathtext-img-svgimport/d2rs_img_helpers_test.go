// SPDX-License-Identifier: MPL-2.0
// Shared helpers for the mathtext-img d2svgimport hooks
// (tools/fixtures/mathtext-img.toml): input/output plumbing, context modes
// and JSON projections of scene nodes and raster asset state.
package d2svgimport

import (
	"context"
	"encoding/json"
	"os"
	"testing"
	"time"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

// d2rsImgEnv returns $D2RS_RECIPE_OUTPUT, skipping outside the runner.
func d2rsImgEnv(t *testing.T) string {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	return out
}

func d2rsImgInput(t *testing.T, v any) {
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, v); err != nil {
		t.Fatal(err)
	}
}

func d2rsImgWrite(t *testing.T, out string, rows any, indent bool) {
	var data []byte
	var err error
	if indent {
		data, err = json.MarshalIndent(rows, "", "  ")
	} else {
		data, err = json.Marshal(rows)
	}
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, data, 0o644); err != nil {
		t.Fatal(err)
	}
}

var d2rsImgModes = []string{"", "cancelled", "deadline"}

// d2rsImgContext is a background, already-cancelled or already-expired context.
func d2rsImgContext(mode string) context.Context {
	ctx := context.Background()
	switch mode {
	case "cancelled":
		var cancel context.CancelFunc
		ctx, cancel = context.WithCancel(ctx)
		cancel()
	case "deadline":
		var cancel context.CancelFunc
		ctx, cancel = context.WithDeadline(ctx, time.Unix(0, 0))
		cancel()
	}
	return ctx
}

func d2rsImgBytes(values []int) []byte {
	out := make([]byte, len(values))
	for i, v := range values {
		out[i] = byte(v)
	}
	return out
}

func d2rsImgInts(data []byte) []int {
	out := []int{}
	for _, b := range data {
		out = append(out, int(b))
	}
	return out
}

func d2rsImgPaint(p d2scene.Paint) any {
	switch p := p.(type) {
	case d2scene.SolidPaint:
		return map[string]any{"kind": "solid", "color": [4]uint8{p.Color.R, p.Color.G, p.Color.B, p.Color.A}}
	case d2scene.LinearGradient:
		stops := []map[string]any{}
		for _, s := range p.Stops {
			stops = append(stops, map[string]any{"offset": s.Offset, "color": [4]uint8{s.Color.R, s.Color.G, s.Color.B, s.Color.A}})
		}
		return map[string]any{"kind": "linear", "start": [2]float64{p.Start.X, p.Start.Y}, "end": [2]float64{p.End.X, p.End.Y},
			"stops": stops, "units": p.Units, "transform": d2rsImgMatrix(p.Transform), "spread": p.Spread}
	}
	return nil
}

func d2rsImgMatrix(m d2scene.Matrix) [6]float64 {
	return [6]float64{m.A, m.B, m.C, m.D, m.E, m.F}
}

func d2rsImgCommands(commands []d2scene.PathCommand) []map[string]any {
	out := []map[string]any{}
	for _, c := range commands {
		out = append(out, map[string]any{"kind": c.Kind, "points": [3][2]float64{{c.P1.X, c.P1.Y}, {c.P2.X, c.P2.Y}, {c.P3.X, c.P3.Y}},
			"radii": [2]float64{c.RadiusX, c.RadiusY}, "rotation": c.Rotation, "large_arc": c.LargeArc, "sweep": c.Sweep})
	}
	return out
}

func d2rsImgPrimitive(p d2scene.Primitive) any {
	if p == nil {
		return nil
	}
	out := map[string]any{}
	var fill d2scene.Paint
	var stroke *d2scene.Stroke
	switch p := p.(type) {
	case d2scene.Image:
		return map[string]any{"kind": "image", "bounds": [4]float64{p.Box.X, p.Box.Y, p.Box.Width, p.Box.Height},
			"asset": string(p.Asset), "aspect": [2]uint8{uint8(p.Aspect.Align), uint8(p.Aspect.Fit)}}
	case d2scene.Rect:
		out["kind"] = "rect"
		out["bounds"] = [4]float64{p.Box.X, p.Box.Y, p.Box.Width, p.Box.Height}
		out["radii"] = [2]float64{p.RadiusX, p.RadiusY}
		fill, stroke = p.Fill, p.Stroke
	case d2scene.Ellipse:
		out["kind"] = "ellipse"
		out["center"] = [2]float64{p.Center.X, p.Center.Y}
		out["radii"] = [2]float64{p.RadiusX, p.RadiusY}
		fill, stroke = p.Fill, p.Stroke
	case d2scene.Path:
		out["kind"] = "path"
		out["commands"] = d2rsImgCommands(p.Commands)
		out["fill_rule"] = p.FillRule
		fill, stroke = p.Fill, p.Stroke
	}
	out["fill"] = d2rsImgPaint(fill)
	var st any
	if stroke != nil {
		st = map[string]any{"paint": d2rsImgPaint(stroke.Paint), "width": stroke.Width, "cap": stroke.Cap, "join": stroke.Join,
			"miter_limit": stroke.MiterLimit, "dashes": stroke.Dashes, "dash_offset": stroke.DashOffset}
	}
	out["stroke"] = st
	return out
}

func d2rsImgNode(n *d2scene.Node) any {
	if n == nil {
		return nil
	}
	children := []any{}
	for _, c := range n.Children {
		children = append(children, d2rsImgNode(c))
	}
	classes := n.Classes
	if classes == nil {
		classes = []string{}
	}
	var clip any
	if n.Clip != nil {
		clip = map[string]any{"commands": d2rsImgCommands(n.Clip.Path.Commands), "fill_rule": n.Clip.Path.FillRule,
			"transform": d2rsImgMatrix(n.Clip.Transform)}
	}
	return map[string]any{"id": n.ID, "classes": classes, "transform": d2rsImgMatrix(n.Transform), "opacity": n.Opacity,
		"clip": clip, "primitive": d2rsImgPrimitive(n.Primitive), "children": children}
}

// d2rsImgAssets projects an importer's raster asset cache and byte charges.
func d2rsImgAssets(i *svgImporter) any {
	entries := map[string]any{}
	for id, v := range i.assets {
		a, ok := v.(d2scene.RasterAsset)
		if !ok {
			entries[string(id)] = nil
			continue
		}
		entries[string(id)] = map[string]any{"mime": a.MIMEType, "data": d2rsImgInts(a.Data), "width": a.PixelWidth,
			"height": a.PixelHeight, "decoded_bytes": a.DecodedBytes}
	}
	return map[string]any{"entries": entries, "encoded_bytes": i.rasterBytes, "decoded_bytes": i.rasterDecodedBytes}
}

// d2rsImgResult projects a public import Result (tree, viewport, metrics and,
// when withAssets, the raster assets).
func d2rsImgResult(result *Result, withAssets bool) any {
	m := result.Metrics
	out := map[string]any{
		"tree": d2rsImgNode(result.Root),
		"viewport": map[string]any{
			"view_box":  [4]float64{result.ViewBox.X, result.ViewBox.Y, result.ViewBox.Width, result.ViewBox.Height},
			"width":     result.Width,
			"height":    result.Height,
			"aspect":    [2]uint8{uint8(result.Aspect.Align), uint8(result.Aspect.Fit)},
			"transform": d2rsImgMatrix(result.ViewportTransform),
			"position":  [2]float64{0, 0},
		},
		"metrics": map[string]any{
			"source_bytes": m.SourceBytes, "parsed_elements": m.ParsedElements, "parsed_attributes": m.ParsedAttributes,
			"parsed_attribute_bytes": m.ParsedAttributeBytes, "parsed_path_commands": m.ParsedPathCommands,
			"parsed_transform_funcs": m.ParsedTransformFuncs, "declared_resources": m.DeclaredResources,
			"expanded_use_instances": m.ExpandedUseInstances, "emitted_elements": m.EmittedElements,
			"emitted_path_commands": m.EmittedPathCommands, "embedded_raster_assets": m.EmbeddedRasterAssets,
			"embedded_raster_bytes": m.EmbeddedRasterBytes, "decoded_raster_bytes": m.DecodedRasterBytes,
		},
	}
	if withAssets {
		out["assets"] = d2rsImgAssets(&svgImporter{assets: result.Assets, rasterBytes: m.EmbeddedRasterBytes,
			rasterDecodedBytes: m.DecodedRasterBytes})
	}
	return out
}

// d2rsImgLimitProfiles are the per-import limit sets swept by the import hooks:
// generous, then each limit at its smallest, then a zero byte limit.
var d2rsImgLimitProfiles = [][9]int{
	{10000, 100, 100, 100, 10000, 100, 100, 100, 100},
	{1, 100, 100, 100, 10000, 100, 100, 100, 100},
	{10000, 100, 1, 100, 10000, 100, 100, 100, 100},
	{10000, 100, 100, 1, 10000, 100, 100, 100, 100},
	{10000, 100, 100, 100, 1, 100, 100, 100, 100},
	{10000, 100, 100, 100, 10000, 1, 100, 100, 100},
	{10000, 100, 100, 100, 10000, 100, 1, 100, 100},
	{10000, 100, 100, 100, 10000, 100, 100, 1, 1},
	{0, 100, 100, 100, 10000, 100, 100, 100, 100},
}

func d2rsImgLimits(l [9]int) Limits {
	return Limits{MaxBytes: l[0], MaxDepth: l[1], MaxElements: l[2], MaxAttributes: l[3], MaxAttributeBytes: l[4],
		MaxPathCommands: l[5], MaxTransformFunctions: l[6], MaxUseDepth: l[7], MaxResources: l[8]}
}
