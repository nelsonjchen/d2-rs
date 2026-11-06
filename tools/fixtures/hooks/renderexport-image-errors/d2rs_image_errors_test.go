// SPDX-License-Identifier: MPL-2.0
// Oracle hook (docs/FIXTURES.md): PNG/JPEG/GIF image-shape icons through the
// d2cli raster pipeline (renderPNGWithEncoder), one diagram per committed
// encoded image. $D2RS_RECIPE_INPUT is a list of rows with "kind" (png, jpg,
// gif) and "data_base64"; every other field is copied. Each output row gets
// "data" (the bytes as a JSON array), "icon" (the inline data: URL, unless
// D2RS_IMAGE_ICON=0) and "error": the complete render error, or with
// D2RS_IMAGE_ERROR=cause its wrapped decoder error; null on success.
// Harvested from the v0.9.0-era caller-error captures
// (.reference/{png,gif,jpeg}-*-native-20261014); retargeted to the manifest pin.
// Inert unless run by scripts/regen-fixtures.py.
package d2cli

import (
	"context"
	"encoding/base64"
	"encoding/json"
	"errors"
	"net/url"
	"os"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2fonts"
	"github.com/d2lang/d2/d2renderers/d2svg"
	"github.com/d2lang/d2/d2target"
)

func TestD2RSImageErrors(t *testing.T) {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	var rows []map[string]any
	if err := json.Unmarshal(data, &rows); err != nil {
		t.Fatal(err)
	}
	font := d2fonts.SourceSansPro
	mono := d2fonts.SourceCodePro
	encoder := &rasterPNGEncoder{}
	defer encoder.close()
	for _, row := range rows {
		raw, err := base64.StdEncoding.DecodeString(row["data_base64"].(string))
		if err != nil {
			t.Fatal(err)
		}
		delete(row, "data_base64")
		ints := make([]int, len(raw))
		for i, b := range raw {
			ints[i] = int(b)
		}
		row["data"] = ints
		mime := map[string]string{"png": "image/png", "jpg": "image/jpeg", "gif": "image/gif"}[row["kind"].(string)]
		icon := "data:" + mime + ";base64," + base64.StdEncoding.EncodeToString(raw)
		if os.Getenv("D2RS_IMAGE_KEEP_KIND") == "" {
			delete(row, "kind")
		}
		u, err := url.Parse(icon)
		if err != nil {
			t.Fatal(err)
		}
		shape := *d2target.BaseShape()
		shape.ID = "x"
		shape.Type = d2target.ShapeImage
		shape.Width = 64
		shape.Height = 64
		shape.Icon = u
		diagram := &d2target.Diagram{Shapes: []d2target.Shape{shape}, FontFamily: &font, MonoFontFamily: &mono}
		_, err = renderPNGWithEncoder(context.Background(), "", false, diagram, d2svg.RenderOpts{}, encoder)
		if os.Getenv("D2RS_IMAGE_ICON") != "0" {
			row["icon"] = icon
		}
		row["error"] = nil
		if err != nil {
			if os.Getenv("D2RS_IMAGE_ERROR") == "cause" {
				if cause := errors.Unwrap(err); cause != nil {
					err = cause
				}
			}
			row["error"] = err.Error()
		}
	}
	data, err = json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}
