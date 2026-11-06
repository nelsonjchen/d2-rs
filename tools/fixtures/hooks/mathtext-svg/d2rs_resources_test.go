// SPDX-License-Identifier: MPL-2.0
// Resource registration, paint and clip-path references (recipe mathtext-svg-resources).
package d2svgimport

import (
	"context"
	"encoding/json"
	"github.com/d2lang/d2/d2renderers/d2scene"
	"os"
	"strings"
	"testing"
	"time"
)

func TestD2RSResources(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	rows := []map[string]any{}
	values := []string{"none", "currentColor", "red", "bad!", "url(#g)", "URL(#g)", "url (#g)", "url(\"#g\")", "url(#g) red", "url(http://x)", "url(#)", "url(#missing)", "url(#1g)", "url(#_g)", "url(#g.x:y)", "url(#🙂)", " url(#g) ", "url(#g g)", "url(#g))", "url(#" + strings.Repeat("a", 300) + ")", strings.Repeat("a", 4097)}
	for _, kind := range []string{"linearGradient", "uncompiled", "clipPath", "path"} {
		for _, mode := range []string{"", "cancelled", "deadline"} {
			for _, value := range values {
				for _, operation := range []string{"paint", "clip"} {
					ctx := context.Background()
					if mode == "cancelled" {
						var c context.CancelFunc
						ctx, c = context.WithCancel(ctx)
						c()
					} else if mode == "deadline" {
						var c context.CancelFunc
						ctx, c = context.WithDeadline(ctx, time.Unix(0, 0))
						defer c()
					}
					target := &svgElement{name: kind}
					if kind == "linearGradient" {
						target.gradient = &d2scene.LinearGradient{Transform: d2scene.Identity()}
					}
					if kind == "uncompiled" {
						target.name = "linearGradient"
					}
					ids := map[string]*svgElement{}
					for _, id := range []string{"g", "1g", "_g", "g.x:y", "🙂", "g)", strings.Repeat("a", 300)} {
						ids[id] = target
					}
					i := svgImporter{ctx: ctx, source: "math", ids: ids}
					var p any
					var id string
					var current bool
					var e error
					if operation == "paint" {
						var paint d2scene.Paint
						paint, current, e = i.parseSVGPaint(value)
						p = d2rsPaint(paint)
					} else {
						id, e = i.localClipPathID(value)
					}
					message := ""
					if e != nil {
						message = e.Error()
					}
					rows = append(rows, map[string]any{"kind": kind, "cancel": mode, "value": value, "operation": operation, "paint": p, "current": current, "id": id, "error": message})
				}
			}
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
