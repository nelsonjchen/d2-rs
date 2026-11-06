// SPDX-License-Identifier: MPL-2.0
// Source witnesses for the static-PNG adapter required by patterns/real-lines
// and real_world/queue_workers. Installed only in the trace checkout.
package d2isometricimg

import (
	"bytes"
	"context"
	"encoding/json"
	"github.com/d2lang/d2/internal/rasterimage"
	"image"
	"image/color"
	"image/png"
	"os"
	"testing"
)

func TestD2rsSurfacePNGContracts(t *testing.T) {
	path := os.Getenv("D2RS_SURFACE_PNG_CONTRACTS")
	if path == "" {
		t.Skip("D2RS_SURFACE_PNG_CONTRACTS not set")
	}
	type witness struct {
		Name            string
		Input, Expected []byte
	}
	var records []witness
	add := func(name string, img image.Image) {
		var input, expected bytes.Buffer
		if err := (&png.Encoder{CompressionLevel: png.BestSpeed}).Encode(&input, img); err != nil {
			t.Fatal(err)
		}
		decoded, err := rasterimage.DecodeFirst(context.Background(), input.Bytes(), "png")
		if err != nil {
			t.Fatal(err)
		}
		if err := png.Encode(&expected, decoded); err != nil {
			t.Fatal(err)
		}
		records = append(records, witness{name, input.Bytes(), expected.Bytes()})
	}
	for _, count := range []int{2, 4, 16, 17} {
		palette := make(color.Palette, count)
		for i := range palette {
			palette[i] = color.NRGBA{uint8(i * 13), uint8(i * 7), uint8(255 - i), 255}
		}
		palette[count-1] = color.NRGBA{70, 90, 110, 80}
		p := image.NewPaletted(image.Rect(0, 0, 7, 3), palette)
		for i := range p.Pix {
			p.Pix[i] = uint8(i % count)
		}
		add(string(rune('a'+count)), p)
	}
	for _, alpha := range []uint8{255, 80} {
		p := image.NewNRGBA(image.Rect(0, 0, 7, 3))
		for y := 0; y < 3; y++ {
			for x := 0; x < 7; x++ {
				p.SetNRGBA(x, y, color.NRGBA{uint8(x * 20), uint8(y * 30), 123, alpha})
			}
		}
		add(string(rune(alpha)), p)
	}
	gray := image.NewGray16(image.Rect(0, 0, 7, 3))
	for y := 0; y < 3; y++ {
		for x := 0; x < 7; x++ {
			gray.SetGray16(x, y, color.Gray16{uint16(x*9000 + y)})
		}
	}
	add("gray16", gray)
	data, err := json.MarshalIndent(records, "", " ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(path, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
