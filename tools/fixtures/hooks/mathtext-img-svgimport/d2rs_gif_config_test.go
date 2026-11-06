// SPDX-License-Identifier: MPL-2.0
// Embedded GIF admission x context (recipe mathtext-img-gif-config).
package d2svgimport

import (
	"context"
	"encoding/json"
	"github.com/d2lang/d2/internal/rasterimage"
	"os"
	"testing"
	"time"
)

func TestD2RSGIFConfig(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	b, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var inputs [][]int
	if err = json.Unmarshal(b, &inputs); err != nil {
		t.Fatal(err)
	}
	rows := []map[string]any{}
	for _, input := range inputs {
		d := []byte{}
		for _, v := range input {
			d = append(d, byte(v))
		}
		for _, mode := range []string{"", "cancelled", "deadline"} {
			ctx := context.Background()
			if mode == "cancelled" {
				var c context.CancelFunc
				ctx, c = context.WithCancel(ctx)
				c()
			} else if mode == "deadline" {
				var c context.CancelFunc
				ctx, c = context.WithDeadline(ctx, time.Unix(0, 0))
				c()
			}
			err := inspectStaticEmbeddedRaster(ctx, d, "gif")
			var output any
			if err == nil {
				cfg, _, e := rasterimage.Config(ctx, d, "gif")
				err = e
				if err == nil {
					n, e := embeddedRasterDecodedBytes(cfg)
					err = e
					if err == nil {
						output = map[string]any{"width": cfg.Width, "height": cfg.Height, "decoded_bytes": n}
					}
				}
			}
			msg := ""
			if err != nil {
				msg = err.Error()
			}
			rows = append(rows, map[string]any{"data": input, "cancel": mode, "error": msg, "output": output})
		}
	}
	b, err = json.Marshal(rows)
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), b, 0644); err != nil {
		t.Fatal(err)
	}
}
