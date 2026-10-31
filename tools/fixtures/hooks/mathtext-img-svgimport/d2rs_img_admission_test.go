// SPDX-License-Identifier: MPL-2.0
package d2svgimport

import (
	"encoding/base64"
	"math"
	"testing"

	"github.com/d2lang/d2/d2renderers/d2scene"
)

// d2rsAdmitImage is compileEmbeddedImage stopped once the embedded raster is
// admitted (after resolveEmbeddedRaster's content ID, before the asset cache,
// resource charges and raster inspection). It mirrors
// d2renderers/d2svgimport/image.go statement for statement up to that point;
// d2-rs-math's svg_image::prepare is the same stage.
func d2rsAdmitImage(i *svgImporter, element *svgElement) (svgGeometry, string, []byte, error) {
	if len(element.children) != 0 {
		return svgGeometry{}, "", nil, i.errorf("element <image> cannot contain child elements")
	}
	raw, ok := element.attrs["href"]
	if !ok {
		return svgGeometry{}, "", nil, i.errorf("element <image> requires one data: href")
	}
	if _, ok := element.attrs["width"]; !ok {
		return svgGeometry{}, "", nil, i.errorf("element <image> requires explicit width and height")
	}
	if _, ok := element.attrs["height"]; !ok {
		return svgGeometry{}, "", nil, i.errorf("element <image> requires explicit width and height")
	}
	x, err := i.lengthAttribute(element, "x", 0, false)
	if err != nil {
		return svgGeometry{}, "", nil, err
	}
	y, err := i.lengthAttribute(element, "y", 0, false)
	if err != nil {
		return svgGeometry{}, "", nil, err
	}
	width, err := i.lengthAttribute(element, "width", 0, true)
	if err != nil {
		return svgGeometry{}, "", nil, err
	}
	height, err := i.lengthAttribute(element, "height", 0, true)
	if err != nil {
		return svgGeometry{}, "", nil, err
	}
	if width <= 0 || height <= 0 {
		return svgGeometry{}, "", nil, i.errorf("element <image> width and height must be positive")
	}
	if math.IsInf(x+width, 0) || math.IsNaN(x+width) || math.IsInf(y+height, 0) || math.IsNaN(y+height) {
		return svgGeometry{}, "", nil, i.errorf("element <image> geometry exceeds the finite numeric domain")
	}
	aspect, err := i.parseAspectRatioFor(element.attrs["preserveAspectRatio"], "embedded image")
	if err != nil {
		return svgGeometry{}, "", nil, err
	}
	// resolveEmbeddedRaster, up to its content ID.
	mimeType, payload, err := i.parseEmbeddedRasterDataURI(raw)
	if err != nil {
		return svgGeometry{}, "", nil, err
	}
	data, err := decodeCanonicalEmbeddedBase64(i.ctx, payload)
	if err != nil {
		if contextErr := i.ctx.Err(); contextErr != nil {
			return svgGeometry{}, "", nil, contextErr
		}
		return svgGeometry{}, "", nil, i.errorf("element <image> has malformed canonical base64 data")
	}
	if err := i.ctx.Err(); err != nil {
		return svgGeometry{}, "", nil, err
	}
	if len(data) == 0 {
		return svgGeometry{}, "", nil, i.errorf("element <image> data payload is empty")
	}
	if len(data) > i.limits.MaxBytes {
		return svgGeometry{}, "", nil, i.errorf("embedded raster bytes exceed limit %d", i.limits.MaxBytes)
	}
	format := embeddedRasterFormatForMIME(mimeType)
	if got := embeddedRasterSignature(data); got == "" {
		return svgGeometry{}, "", nil, i.errorf("element <image> data has no supported raster signature")
	} else if got != format {
		return svgGeometry{}, "", nil, i.errorf("element <image> MIME type does not match its raster signature")
	}
	assetID, err := i.embeddedRasterContentID(mimeType, data)
	if err != nil {
		return svgGeometry{}, "", nil, err
	}
	return svgGeometry{
		kind: geometryImage, box: d2scene.Box{X: x, Y: y, Width: width, Height: height},
		asset: assetID, aspect: aspect,
	}, mimeType, data, nil
}

// TestD2RSImageAdmission: <image> attribute, data: URI and canonical base64
// admission (svg-image-admission-v090.json). Inputs are built here.
func TestD2RSImageAdmission(t *testing.T) {
	out := d2rsImgEnv(t)
	values := []string{"", "http://secret/x", "#p", "DATA:image/png;base64,AAAA", "data:", "data:image/png",
		"data:image/png,AAAA", "data:image/png;base64,", "data:image/png;BASE64,AAAA",
		"data:image/png;charset=x;base64,AAAA", "data:image/svg+xml;base64,AAAA", "data:image/png;base64,A",
		"data:image/png;base64,AAA=", "data:image/png;base64,AAB=", "data:image/png;base64,AA==",
		"data:image/png;base64,AB==", "data:image/png;base64,AA=A", "data:image/png;base64,AA\n=",
		"data:image/png;base64,AAAA"}
	for _, mime := range []string{"image/png", "image/jpeg", "image/gif", "image/webp"} {
		for _, data := range [][]byte{[]byte("\x89PNG\r\n\x1a\n"), {0xff, 0xd8, 0xff}, []byte("GIF89a"),
			[]byte("RIFF0000WEBP"), make([]byte, 32772)} {
			values = append(values, "data:"+mime+";base64,"+base64.StdEncoding.EncodeToString(data))
		}
	}
	attrs := []map[string]string{{}, {"width": "2"}, {"width": "2", "height": "3"}, {"width": "0", "height": "3"},
		{"width": "-1", "height": "3"}, {"width": "bad", "height": "3"}, {"width": "2", "height": "3", "x": "bad"},
		{"width": "1e308", "height": "3", "x": "1e308"}, {"width": "2", "height": "3", "preserveAspectRatio": "bad"},
		{"width": "2", "height": "3", "preserveAspectRatio": "none slice"}}
	rows := []map[string]any{}
	for _, raw := range values {
		for _, a := range attrs {
			for _, maximum := range []int{1, 100000} {
				for _, mode := range d2rsImgModes {
					element := map[string]string{"href": raw}
					for k, v := range a {
						element[k] = v
					}
					i := svgImporter{ctx: d2rsImgContext(mode), source: "math", limits: Limits{MaxBytes: maximum}}
					geometry, mime, data, err := d2rsAdmitImage(&i, &svgElement{name: "image", attrs: element})
					message := ""
					var output any
					if err != nil {
						message = err.Error()
					} else {
						output = map[string]any{
							"bounds":     [4]float64{geometry.box.X, geometry.box.Y, geometry.box.Width, geometry.box.Height},
							"aspect":     [2]uint8{uint8(geometry.aspect.Align), uint8(geometry.aspect.Fit)},
							"mime":       mime,
							"data":       d2rsImgInts(data),
							"content_id": geometry.asset,
						}
					}
					rows = append(rows, map[string]any{"attrs": element, "maximum": maximum, "cancel": mode,
						"error": message, "output": output})
				}
			}
		}
	}
	d2rsImgWrite(t, out, rows, true)
}
