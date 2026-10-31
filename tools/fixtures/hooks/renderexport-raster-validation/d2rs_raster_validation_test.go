// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): validateRasterAsset / prepareRasterAsset
// outcomes (format, decoded-byte charge, errors, prepared 16-bit colors) for
// the generated raster validation controls (scripts/renderexport_inputs.py
// raster-validation). Harvested from the v0.9.0-era bridge
// .reference/raster-jpeg-validation-native-20261014/oracle-bridge.go; inert
// unless run by scripts/regen-fixtures.py. Writes [{"input", "expected"}].
package d2raster

import (
	"context"
	"encoding/json"
	"os"
	"testing"
	"time"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

type d2rsRasterInput struct {
	ID, MIME, Mode, Cancel                 string
	Data                                   []byte
	Width, Height, DecodedBytes, Available int64
}

type d2rsRasterOutput struct {
	Format        string
	DecodedBytes  int64
	Error         string
	Width, Height int
	Pixels        [][4]uint32
}

func d2rsRasterValidation(input d2rsRasterInput) d2rsRasterOutput {
	ctx := context.Background()
	if input.Cancel == "cancelled" {
		var cancel context.CancelFunc
		ctx, cancel = context.WithCancel(ctx)
		cancel()
	} else if input.Cancel == "deadline" {
		var cancel context.CancelFunc
		ctx, cancel = context.WithDeadline(ctx, time.Unix(1, 0))
		defer cancel()
	}
	asset := d2scene.RasterAsset{MIMEType: input.MIME, Data: input.Data, PixelWidth: int(input.Width), PixelHeight: int(input.Height), DecodedBytes: input.DecodedBytes}
	out := d2rsRasterOutput{Pixels: [][4]uint32{}}
	if input.Mode == "prepare" {
		image, charge, err := prepareRasterAsset(ctx, d2scene.AssetID(input.ID), asset, input.Available)
		if err != nil {
			out.Error = err.Error()
		} else {
			out.DecodedBytes = charge
			out.Format = "jpeg"
			out.Width = image.bounds.Dx()
			out.Height = image.bounds.Dy()
			for y := range out.Height {
				for x := range out.Width {
					r, g, b, a := image.image.At(x, y).RGBA()
					out.Pixels = append(out.Pixels, [4]uint32{r, g, b, a})
				}
			}
		}
	} else {
		v, err := validateRasterAsset(ctx, d2scene.AssetID(input.ID), asset, input.Available)
		if err != nil {
			out.Error = err.Error()
		} else {
			out.Format = v.format
			out.DecodedBytes = v.decodedBytes
		}
	}
	return out
}

func TestD2RSRasterValidation(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	// The request list stores Data as a JSON byte array (the reader's
	// contract); decode it explicitly rather than as base64.
	var raw []map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		t.Fatal(err)
	}
	type row struct {
		Input    json.RawMessage  `json:"input"`
		Expected d2rsRasterOutput `json:"expected"`
	}
	rows := make([]row, 0, len(raw))
	for _, fields := range raw {
		var input d2rsRasterInput
		var bytes []int
		if err := json.Unmarshal(fields["Data"], &bytes); err != nil {
			t.Fatal(err)
		}
		for _, b := range bytes {
			input.Data = append(input.Data, byte(b))
		}
		for name, target := range map[string]any{"ID": &input.ID, "MIME": &input.MIME, "Mode": &input.Mode, "Cancel": &input.Cancel,
			"Width": &input.Width, "Height": &input.Height, "DecodedBytes": &input.DecodedBytes, "Available": &input.Available} {
			if err := json.Unmarshal(fields[name], target); err != nil {
				t.Fatal(name, err)
			}
		}
		encoded, err := json.Marshal(struct {
			ID           string
			MIME         string
			Data         []int
			Width        int64
			Height       int64
			DecodedBytes int64
			Available    int64
			Mode         string
			Cancel       string
		}{input.ID, input.MIME, bytes, input.Width, input.Height, input.DecodedBytes, input.Available, input.Mode, input.Cancel})
		if err != nil {
			t.Fatal(err)
		}
		rows = append(rows, row{encoded, d2rsRasterValidation(input)})
	}
	encoded, err := json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(encoded, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
