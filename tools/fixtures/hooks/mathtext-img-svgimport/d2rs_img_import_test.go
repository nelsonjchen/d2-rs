// SPDX-License-Identifier: MPL-2.0
package d2svgimport

import (
	"context"
	"image/color"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

// TestD2RSImageAssets: compileEmbeddedImage twice against a pre-seeded raster
// asset cache (reuse, MIME/data/length collisions, foreign entries, spent
// byte charges) under byte and resource limits and each context mode
// (svg-image-assets-v090.json). Input: a JSON list of href values.
func TestD2RSImageAssets(t *testing.T) {
	out := d2rsImgEnv(t)
	var inputs []string
	d2rsImgInput(t, &inputs)
	rows := []map[string]any{}
	for _, raw := range inputs {
		for _, seed := range []string{"", "reuse", "wrongmime", "wrongdata", "wronglength", "other", "bytes", "decoded", "unrelated"} {
			for _, maximum := range []int{16, 64, 1000} {
				for _, resources := range []int{0, 1, 3} {
					for _, mode := range d2rsImgModes {
						i := svgImporter{ctx: d2rsImgContext(mode), source: "math", assets: map[d2scene.AssetID]d2scene.Asset{},
							ids: map[string]*svgElement{}, limits: Limits{MaxBytes: maximum, MaxResources: resources}}
						probe := svgImporter{ctx: context.Background(), source: "math"}
						if mime, payload, err := probe.parseEmbeddedRasterDataURI(raw); err == nil {
							if data, err := decodeCanonicalEmbeddedBase64(context.Background(), payload); err == nil {
								id, err := probe.embeddedRasterContentID(mime, data)
								if err != nil {
									t.Fatal(err)
								}
								asset := d2scene.RasterAsset{MIMEType: mime, Data: data, PixelWidth: 9, PixelHeight: 8, DecodedBytes: 288}
								switch seed {
								case "reuse":
									i.assets[id] = asset
									i.rasterBytes = len(data)
									i.rasterDecodedBytes = 288
								case "wrongmime":
									asset.MIMEType = "other"
									i.assets[id] = asset
								case "wrongdata":
									asset.Data = append([]byte{}, data...)
									if len(data) > 0 {
										asset.Data[0] ^= 1
									}
									i.assets[id] = asset
								case "wronglength":
									asset.Data = append(data, 0)
									i.assets[id] = asset
								case "other":
									i.assets[id] = nil
								}
							}
						}
						switch seed {
						case "bytes":
							i.rasterBytes = maximum
						case "decoded":
							i.rasterDecodedBytes = int64(maximum)
						case "unrelated":
							i.assets["other"] = nil
							i.ids["declared"] = &svgElement{}
						}
						attrs := map[string]string{"href": raw, "width": "2", "height": "3", "x": "4", "preserveAspectRatio": "xMinYMin slice"}
						before := d2rsImgAssets(&i)
						calls := []any{}
						for repeat := 0; repeat < 2; repeat++ {
							g, err := i.compileEmbeddedImage(&svgElement{name: "image", attrs: attrs})
							message := ""
							var output any
							if err != nil {
								message = err.Error()
							} else {
								output = map[string]any{"bounds": [4]float64{g.box.X, g.box.Y, g.box.Width, g.box.Height},
									"aspect": [2]uint8{uint8(g.aspect.Align), uint8(g.aspect.Fit)}, "content_id": string(g.asset)}
							}
							calls = append(calls, map[string]any{"error": message, "output": output, "assets": d2rsImgAssets(&i)})
						}
						rows = append(rows, map[string]any{"attrs": attrs, "maximum": maximum, "resources": resources,
							"declared": len(i.ids), "cancel": mode, "before": before, "calls": calls})
					}
				}
			}
		}
	}
	d2rsImgWrite(t, out, rows, false)
}

// d2rsImportRows runs ImportNodeWithOptions over sources x limit profiles x
// currentColor x context modes.
func d2rsImportRows(inputs []string, withAssets bool) []map[string]any {
	rows := []map[string]any{}
	for _, input := range inputs {
		for _, l := range d2rsImgLimitProfiles {
			for _, custom := range []bool{false, true} {
				for _, mode := range d2rsImgModes {
					options := ImportOptions{}
					if custom {
						options.CurrentColor = &color.NRGBA{R: 1, G: 2, B: 3, A: 200}
					}
					result, err := ImportNodeWithOptions(d2rsImgContext(mode), "math", []byte(input), d2rsImgLimits(l), options)
					message := ""
					if err != nil {
						message = err.Error()
					}
					var output any
					if result != nil {
						output = d2rsImgResult(result, withAssets)
					}
					rows = append(rows, map[string]any{"input": input, "limits": l, "custom": custom, "cancel": mode,
						"error": message, "output": output})
				}
			}
		}
	}
	return rows
}

// TestD2RSImageImport: public ImportNodeWithOptions results, including raster
// assets, for SVG documents with embedded images (svg-image-import-v090.json).
// Input: a JSON list of SVG sources.
func TestD2RSImageImport(t *testing.T) {
	out := d2rsImgEnv(t)
	var inputs []string
	d2rsImgInput(t, &inputs)
	d2rsImgWrite(t, out, d2rsImportRows(inputs, true), true)
}

// TestD2RSImportResult: public ImportNodeWithOptions results for small vector
// documents (svg-import-result-v090.json). Inputs are listed here.
func TestD2RSImportResult(t *testing.T) {
	out := d2rsImgEnv(t)
	inputs := []string{"<svg width='10' height='20'/>", "<svg viewBox='1 2 10 20'/>",
		"<svg viewBox='1 2 10 20' width='30'/>", "<svg viewBox='1 2 10 20' height='30'/>",
		"<svg id='root' width='10' height='20' display='none'><rect width='2' height='3'/></svg>",
		"<svg width='10' height='20'><rect width='2' height='3' fill='currentColor'/></svg>",
		"<svg width='10' height='20'><g opacity='0.5'><path d='M0 0L2 3' stroke='red'/></g></svg>",
		"<svg width='10' height='20'><defs><path id='p' d='M0 0L1 1'/></defs><use href='#p'/><use href='#p'/></svg>",
		"<svg width='10' height='20'><clipPath id='c'><rect width='2' height='3'/></clipPath><rect width='2' height='3' clip-path='url(#c)'/></svg>",
		"<svg width='10' height='20'><svg width='2' height='3' viewBox='0 0 2 3'><path d='M0 0L1 1'/></svg></svg>",
		"<svg width='10' height='20'><rect x='1e308' width='1e308' height='3'/></svg>",
		"<svg width='10' height='20'><g transform='scale(1e308)'><g transform='scale(1e308)'/></g></svg>",
		"<svg width='10' height='20'><defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='0' gradientUnits='userSpaceOnUse'><stop offset='0' stop-color='red'/><stop offset='1' stop-color='blue'/></linearGradient></defs><rect width='2' height='3' fill='url(#g)'/></svg>",
		"<svg width='10' height='20'><title>Hello</title></svg>", "<svg/>", "<svg width='bad'/>",
		"<svg width='10' height='20'><text>bad</text></svg>", "<svg width='10' height='20'><path d='bad'/></svg>",
		"\ufeff<svg width='10' height='20'/>", "\ufeff\ufeff<svg width='10' height='20'/>",
		"<?xml version='1.0' encoding='ISO-8859-1'?><svg width='10' height='20'><title>h\u00e9</title></svg>"}
	d2rsImgWrite(t, out, d2rsImportRows(inputs, false), true)
}
