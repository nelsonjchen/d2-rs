// SPDX-License-Identifier: MPL-2.0
// Overlay hook for internal/rasterimage (tools/fixtures/mathtext-img.toml):
// jpegEXIFOrientation and orientedConfig over byte-array inputs
// ($D2RS_RECIPE_INPUT) under each context mode.
package rasterimage

import (
	"context"
	"encoding/json"
	"image"
	"os"
	"testing"
	"time"
)

func TestD2RSJPEGOrientation(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	raw, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var inputs [][]int
	if err := json.Unmarshal(raw, &inputs); err != nil {
		t.Fatal(err)
	}
	rows := []map[string]any{}
	for _, input := range inputs {
		data := make([]byte, len(input))
		for i, v := range input {
			data[i] = byte(v)
		}
		for _, mode := range []string{"", "cancelled", "deadline"} {
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
			orientation, err := jpegEXIFOrientation(ctx, data)
			message := ""
			if err != nil {
				message = err.Error()
			}
			config := orientedConfig(image.Config{Width: 2, Height: 3}, orientation)
			rows = append(rows, map[string]any{"data": input, "cancel": mode, "orientation": orientation, "error": message,
				"dimensions": [2]int{config.Width, config.Height}})
		}
	}
	data, err := json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, data, 0o644); err != nil {
		t.Fatal(err)
	}
}
