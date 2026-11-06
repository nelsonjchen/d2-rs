// SPDX-License-Identifier: MPL-2.0
// Oracle for recipe mathtext-img-jpeg-frame (tools/fixtures/mathtext-img.toml):
// Go stdlib image/jpeg.Decode over [{"Name","Data"}] on stdin; writes
// [{"input", "expected": {Width, Height, Error, Pixels (16-bit RGBA)}}].
package main

import (
	"bytes"
	"encoding/json"
	"image/jpeg"
	"os"
)

type input struct {
	Data []int
	Name string
}

type output struct {
	Width, Height int
	Error         string
	Pixels        [][4]uint32
}

type row struct {
	Input    input  `json:"input"`
	Expected output `json:"expected"`
}

func main() {
	var inputs []input
	if err := json.NewDecoder(os.Stdin).Decode(&inputs); err != nil {
		panic(err)
	}
	rows := make([]row, len(inputs))
	for i, in := range inputs {
		data := make([]byte, len(in.Data))
		for k, v := range in.Data {
			data[k] = byte(v)
		}
		o := output{Pixels: [][4]uint32{}}
		if m, err := jpeg.Decode(bytes.NewReader(data)); err != nil {
			o.Error = err.Error()
		} else {
			o.Width, o.Height = m.Bounds().Dx(), m.Bounds().Dy()
			for y := range o.Height {
				for x := range o.Width {
					r, g, b, a := m.At(m.Bounds().Min.X+x, m.Bounds().Min.Y+y).RGBA()
					o.Pixels = append(o.Pixels, [4]uint32{r, g, b, a})
				}
			}
		}
		rows[i] = row{in, o}
	}
	if err := json.NewEncoder(os.Stdout).Encode(rows); err != nil {
		panic(err)
	}
}
