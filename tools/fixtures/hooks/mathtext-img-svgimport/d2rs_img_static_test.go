// SPDX-License-Identifier: MPL-2.0
package d2svgimport

import (
	"image"
	"os"
	"testing"

	"github.com/d2lang/d2/internal/rasterimage"
)

// TestD2RSImageStatic: inspectStaticEmbeddedRaster over {"format","data"}
// inputs under each context mode (svg-image-static-v090.json).
func TestD2RSImageStatic(t *testing.T) {
	out := d2rsImgEnv(t)
	var inputs []struct {
		Format string `json:"format"`
		Data   []int  `json:"data"`
	}
	d2rsImgInput(t, &inputs)
	rows := []map[string]any{}
	for _, in := range inputs {
		data := d2rsImgBytes(in.Data)
		for _, mode := range d2rsImgModes {
			message := ""
			if err := inspectStaticEmbeddedRaster(d2rsImgContext(mode), data, in.Format); err != nil {
				message = err.Error()
			}
			rows = append(rows, map[string]any{"format": in.Format, "data": in.Data, "cancel": mode, "error": message})
		}
	}
	d2rsImgWrite(t, out, rows, false)
}

// TestD2RSRasterConfig: the embedded raster admission of one format
// ($D2RS_IMG_FORMAT): static inspection, rasterimage.Config and the decoded
// byte charge, over byte-array inputs under each context mode
// ({jpeg,png,webp}-config-v090.json).
func TestD2RSRasterConfig(t *testing.T) {
	out := d2rsImgEnv(t)
	format := os.Getenv("D2RS_IMG_FORMAT")
	if format == "" {
		t.Fatal("D2RS_IMG_FORMAT is unset")
	}
	var inputs [][]int
	d2rsImgInput(t, &inputs)
	rows := []map[string]any{}
	for _, input := range inputs {
		data := d2rsImgBytes(input)
		for _, mode := range d2rsImgModes {
			ctx := d2rsImgContext(mode)
			err := inspectStaticEmbeddedRaster(ctx, data, format)
			var output any
			if err == nil {
				var cfg image.Config
				cfg, _, err = rasterimage.Config(ctx, data, format)
				if err == nil {
					var n int64
					n, err = embeddedRasterDecodedBytes(cfg)
					if err == nil {
						output = map[string]any{"width": cfg.Width, "height": cfg.Height, "decoded_bytes": n}
					}
				}
			}
			message := ""
			if err != nil {
				message = err.Error()
			}
			rows = append(rows, map[string]any{"data": input, "cancel": mode, "error": message, "output": output})
		}
	}
	d2rsImgWrite(t, out, rows, false)
}
