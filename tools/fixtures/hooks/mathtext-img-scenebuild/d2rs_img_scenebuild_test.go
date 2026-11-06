// SPDX-License-Identifier: MPL-2.0
// Overlay hook for d2renderers/d2scenebuild (tools/fixtures/mathtext-img.toml):
// the resolved-SVG asset cache and the cumulative document SVG import budget.
// Input: a JSON list of SVG sources ($D2RS_RECIPE_INPUT).
package d2scenebuild

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"fmt"
	"net/url"
	"os"
	"testing"
	"time"

	"github.com/d2lang/d2/d2renderers/d2scene"
	"github.com/d2lang/d2/d2renderers/d2svgimport"
)

func d2rsImgSetup(t *testing.T) (string, []string) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var inputs []string
	if err := json.Unmarshal(data, &inputs); err != nil {
		t.Fatal(err)
	}
	return out, inputs
}

func d2rsImgWriteRows(t *testing.T, out string, rows any) {
	data, err := json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, data, 0o644); err != nil {
		t.Fatal(err)
	}
}

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

func d2rsImgMatrix(m d2scene.Matrix) [6]float64 { return [6]float64{m.A, m.B, m.C, m.D, m.E, m.F} }

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

func d2rsImgDocumentState(b *builder, maximum [8]int) any {
	resources := map[string]any{}
	for id, v := range b.assets {
		a, ok := v.(d2scene.RasterAsset)
		if !ok {
			resources[string(id)] = nil
			continue
		}
		data := []int{}
		for _, v := range a.Data {
			data = append(data, int(v))
		}
		resources[string(id)] = map[string]any{"mime": a.MIMEType, "data": data, "width": a.PixelWidth,
			"height": a.PixelHeight, "decoded_bytes": a.DecodedBytes}
	}
	used := [8]int{b.svgSourceBytes, b.svgElements, b.svgAttributes, b.svgAttributeBytes, b.svgCommands,
		b.svgTransforms, b.svgDeclaredResources, b.svgExpandedUseInstances}
	return map[string]any{"budget": map[string]any{"maximum": maximum, "used": used}, "resources": resources}
}

func d2rsImgCacheState(b *builder, maximum [8]int) any {
	vectors := map[string]any{}
	for id, v := range b.assets {
		if v, ok := v.(d2scene.VectorAsset); ok {
			vectors[string(id)] = map[string]any{"view_box": [4]float64{v.ViewBox.X, v.ViewBox.Y, v.ViewBox.Width, v.ViewBox.Height},
				"tree": d2rsImgNode(v.Root)}
		}
	}
	digests := map[string]string{}
	for d, id := range b.assetIDs {
		digests[fmt.Sprintf("%x", d[:])] = string(id)
	}
	sources := map[string]string{}
	for k, id := range b.sourceAssetIDs {
		sources[k] = string(id)
	}
	return map[string]any{"document": d2rsImgDocumentState(b, maximum), "vectors": vectors, "by_digest": digests, "by_source": sources}
}

var d2rsImgPerAsset = d2svgimport.Limits{MaxBytes: 100000, MaxDepth: 100, MaxElements: 100, MaxAttributes: 100,
	MaxAttributeBytes: 100000, MaxPathCommands: 100, MaxTransformFunctions: 100, MaxUseDepth: 100, MaxResources: 100}

var d2rsImgPerAssetRow = [9]int{100000, 100, 100, 100, 100000, 100, 100, 100, 100}

func d2rsImgBudget(m [8]int) SVGImportBudget {
	return SVGImportBudget{m[0], m[1], m[2], m[3], m[4], m[5], m[6], m[7]}
}

// TestD2RSAssetCache: resolveImageAsset for base64 and percent-encoded data:
// aliases of one SVG, twice each, under five document budget profiles, with
// the context cancelled or expired before the repeats
// (svg-asset-cache-v090.json).
func TestD2RSAssetCache(t *testing.T) {
	out, inputs := d2rsImgSetup(t)
	rows := []any{}
	for _, input := range inputs {
		for profile := 0; profile < 5; profile++ {
			for _, mode := range []string{"", "cancelled", "deadline"} {
				maximum := [8]int{100000, 100, 100, 100000, 100, 100, 100, 100}
				switch profile {
				case 1:
					maximum[0] = len(input)
				case 2:
					maximum[6] = 1
				case 3:
					maximum[6] = 0
				case 4:
					maximum[1] = 1
				}
				b := builder{ctx: context.Background(), options: Options{Assets: &AssetOptions{Resolver: newTestAssetResolver(t, nil),
					SVGImportBudget: d2rsImgBudget(maximum), SVGImportLimits: d2rsImgPerAsset}},
					assets: map[d2scene.AssetID]d2scene.Asset{}, assetIDs: map[[32]byte]d2scene.AssetID{},
					sourceAssetIDs: map[string]d2scene.AssetID{}}
				source := "data:image/svg+xml;base64," + base64.StdEncoding.EncodeToString([]byte(input))
				alias := "data:image/svg+xml," + url.PathEscape(input)
				before := d2rsImgCacheState(&b, maximum)
				calls := []any{}
				for n, key := range []string{source, alias, source, alias} {
					stepMode := ""
					if n >= 2 && profile <= 1 {
						stepMode = mode
						if mode != "" {
							b.ctx = d2rsImgContext(mode)
						}
					}
					id, err := b.resolveImageAsset("shape \"n\"", key)
					message := ""
					if err != nil {
						message = err.Error()
					}
					calls = append(calls, map[string]any{"source": key, "cancel": stepMode, "error": message, "id": string(id),
						"state": d2rsImgCacheState(&b, maximum)})
				}
				rows = append(rows, map[string]any{"input": input, "limits": d2rsImgPerAssetRow, "before": before, "calls": calls})
			}
		}
	}
	d2rsImgWriteRows(t, out, rows)
}

// TestD2RSDocumentImport: remainingSVGImportLimits, ImportNode,
// reserveSVGImport and mergeSVGImportAssets three times per document under
// twelve budget profiles, four pre-seeded asset states and each context mode
// (svg-document-import-v090.json).
func TestD2RSDocumentImport(t *testing.T) {
	out, inputs := d2rsImgSetup(t)
	rows := []any{}
	for _, input := range inputs {
		for profile := 0; profile < 12; profile++ {
			for _, seed := range []string{"", "equal", "collision", "other"} {
				for _, mode := range []string{"", "cancelled", "deadline"} {
					ctx := d2rsImgContext(mode)
					maximum := [8]int{100000, 100, 100, 100000, 100, 100, 100, 100}
					used := [8]int{}
					switch {
					case profile < 8:
						maximum[profile] = 1
					case profile == 8:
						maximum[0] = len(input)*2 - 1
					case profile == 9:
						used[6] = 99
					case profile == 10:
						used[7] = 100
					default:
						maximum[6] = 0
					}
					b := builder{ctx: ctx, options: Options{Assets: &AssetOptions{SVGImportBudget: d2rsImgBudget(maximum),
						SVGImportLimits: d2rsImgPerAsset}}, assets: map[d2scene.AssetID]d2scene.Asset{},
						svgSourceBytes: used[0], svgElements: used[1], svgAttributes: used[2], svgAttributeBytes: used[3],
						svgCommands: used[4], svgTransforms: used[5], svgDeclaredResources: used[6], svgExpandedUseInstances: used[7]}
					if seed != "" {
						if result, err := d2svgimport.ImportNode(context.Background(), "math", []byte(input), d2rsImgPerAsset); err == nil {
							for id, v := range result.Assets {
								if seed == "other" {
									b.assets[id] = nil
									continue
								}
								a := v.(d2scene.RasterAsset)
								if seed == "collision" {
									a.PixelWidth++
								}
								b.assets[id] = a
							}
						}
					}
					before := d2rsImgDocumentState(&b, maximum)
					calls := []any{}
					for repeat := 0; repeat < 3; repeat++ {
						remaining, err := b.remainingSVGImportLimits("shape \"n\"")
						var result *d2svgimport.Result
						if err == nil {
							result, err = d2svgimport.ImportNode(ctx, "math", []byte(input), remaining)
							if err != nil {
								err = fmt.Errorf("scene: shape \"n\" SVG asset: %w", err)
							}
						}
						if err == nil {
							err = b.reserveSVGImport("shape \"n\"", result.Metrics)
						}
						if err == nil {
							err = b.mergeSVGImportAssets("shape \"n\"", result.Assets)
						}
						message := ""
						if err != nil {
							message = err.Error()
						}
						calls = append(calls, map[string]any{"error": message, "state": d2rsImgDocumentState(&b, maximum)})
					}
					rows = append(rows, map[string]any{"input": input, "limits": d2rsImgPerAssetRow, "cancel": mode,
						"before": before, "calls": calls})
				}
			}
		}
	}
	d2rsImgWriteRows(t, out, rows)
}
