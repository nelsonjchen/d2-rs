// SPDX-License-Identifier: MPL-2.0
// compileUse href admission: raw href x target id x target element
// (recipe mathtext-svg-use-reference).
package d2svgimport

import (
	"context"
	"testing"
)

func TestD2RSSvgUseReference(t *testing.T) {
	out := d2rsOutput(t)
	type row struct {
		Raw   *string `json:"raw"`
		ID    string  `json:"id"`
		Name  string  `json:"name"`
		Error string  `json:"error"`
	}
	rows := []row{}
	raws := []string{"", "#", "x", "http://example.com/#x", "#x", "#missing", "##x", "#x y", "#x\t", "#x\u0085", "#x\u00a0", "#x\u200b", "#x\u2028", "#x\ufeff",
		"#x\x7f", "#x\u0080", "#🙂", "#1", "#x:y", "#x/y"}
	for _, name := range []string{"g", "path", "rect", "circle", "ellipse", "line", "polyline", "polygon", "image", "text", "use", "svg", "defs", "title",
		"metadata", "rdf:RDF", "cc:Work", "dc:format", "dc:type", "sodipodi:namedview", "linearGradient", "stop", "clipPath"} {
		for _, id := range []string{"x", "🙂", "1", "x:y", "x/y", "x\u200b", "x\ufeff"} {
			rows = append(rows, row{nil, id, name, ""})
			for _, raw := range raws {
				value := raw
				rows = append(rows, row{&value, id, name, ""})
			}
		}
	}
	for n := range rows {
		r := &rows[n]
		attrs := map[string]string{}
		if r.Raw != nil {
			attrs["href"] = *r.Raw
		}
		i := svgImporter{ctx: context.Background(), source: "math", ids: map[string]*svgElement{r.ID: {name: r.Name}}}
		r.Error = d2rsError(i.compileUse(&svgElement{name: "use", attrs: attrs}))
	}
	d2rsWrite(t, out, rows)
}
