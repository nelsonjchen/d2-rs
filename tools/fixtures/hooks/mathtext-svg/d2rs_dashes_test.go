// SPDX-License-Identifier: MPL-2.0
// stroke-dasharray parsing x context (recipe mathtext-svg-dashes).
package d2svgimport

import (
	"context"
	"encoding/json"
	"math"
	"os"
	"strings"
	"testing"
	"time"
)

func TestD2RSDashes(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	type row struct {
		Value  string    `json:"value"`
		Cancel string    `json:"cancel"`
		Bits   *[]uint64 `json:"bits"`
		Error  string    `json:"error"`
	}
	rows := []row{}
	values := []string{"", " ", "none", "0", "-0", "0 0", "1", "1 2", "1 2 3", "1,2,3", "0 1", "1 0", "-1", "1 -1", "1,,2", ",1", "1,", "1-2", "1+2", "1e-2 3e+2", "1mm 2cm", "1%", "1 px", "auto", "NaN", "1e999", "1e-999", "1e308", "1e308 1e308", "1e307 1e307", "1\u00a02", "\u00a01\u00a0", strings.Repeat("1", 513), strings.Repeat("1 ", 5000)}
	for _, value := range values {
		for _, cancelMode := range []string{"", "cancelled", "deadline"} {
			ctx := context.Background()
			if cancelMode == "cancelled" {
				var c context.CancelFunc
				ctx, c = context.WithCancel(ctx)
				c()
			} else if cancelMode == "deadline" {
				var c context.CancelFunc
				ctx, c = context.WithDeadline(ctx, time.Unix(0, 0))
				defer c()
			}
			dashes, e := parseDashArray(ctx, value)
			r := row{Value: value, Cancel: cancelMode}
			if e != nil {
				r.Error = e.Error()
			} else if dashes != nil {
				bits := []uint64{}
				for _, v := range dashes {
					bits = append(bits, math.Float64bits(v))
				}
				r.Bits = &bits
			}
			rows = append(rows, r)
		}
	}
	data, e := json.MarshalIndent(rows, "", "  ")
	if e != nil {
		t.Fatal(e)
	}
	if e = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); e != nil {
		t.Fatal(e)
	}
}
