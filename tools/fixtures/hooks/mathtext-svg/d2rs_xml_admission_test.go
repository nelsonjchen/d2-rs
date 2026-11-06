// SPDX-License-Identifier: MPL-2.0
// XML namespace/element admission (elementName) and attribute admission
// with count/byte charges (attributes) (recipe mathtext-svg-xml-admission).
package d2svgimport

import (
	"context"
	"encoding/xml"
	"strings"
	"testing"
)

func TestD2RSSvgXMLAdmission(t *testing.T) {
	out := d2rsOutput(t)
	rows := []map[string]any{}
	spaces := []string{"", svgNamespace, rdfNamespace, creativeCommonsNS, dublinCoreNamespace, sodipodiNamespace, inkscapeNamespace, "foreign"}
	names := []string{"svg", "g", "defs", "path", "image", "text", "clipPath", "linearGradient", "stop", "title", "metadata", "style", "script", "foreignObject",
		"radialGradient", "pattern", "unknown", "RDF", "Work", "format", "type", "namedview", strings.Repeat("x", 65)}
	for _, space := range spaces {
		for _, name := range names {
			for _, prior := range []string{"unset", "", svgNamespace} {
				i := svgImporter{source: "math", ctx: context.Background(), elementNamespace: prior, namespaceSet: prior != "unset"}
				output, err := i.elementName(xml.Name{Space: space, Local: name})
				var after any
				if i.namespaceSet {
					after = i.elementNamespace
				}
				rows = append(rows, map[string]any{"operation": "element", "namespace": space, "name": name, "prior": prior, "after": after,
					"output": output, "error": d2rsError(err)})
			}
		}
	}
	attr := func(space, local, value string) xml.Attr {
		return xml.Attr{Name: xml.Name{Space: space, Local: local}, Value: value}
	}
	attrs := [][]xml.Attr{{}, {attr("", "id", "p")}, {attr("", "id", "bad id")}, {attr("", "id", "🙂")}, {attr("", "id", "#")}, {attr("", "id", "")},
		{attr("", "href", "#a"), attr(xlinkNamespace, "href", "#b")}, {attr("", "x", "1"), attr("", "x", "2")}}
	for _, name := range []string{"data-", "data-x", "data-X_2", "data-é", "role", "focusable", "onload", "ONLOAD", "once", "ordinary"} {
		for _, value := range []string{"", " img ", "false", "presentation", "none", "true"} {
			attrs = append(attrs, []xml.Attr{attr("", name, value)})
		}
	}
	for _, ns := range []string{xmlNamespace, inkscapeNamespace, sodipodiNamespace, rdfNamespace, xlinkNamespace, "foreign"} {
		for _, name := range []string{"space", "label", "docname", "nodetypes", "about", "resource", "href", "other"} {
			value := "preserve"
			if name == "about" {
				value = ""
			}
			if name == "resource" {
				value = stillImageResource
			}
			attrs = append(attrs, []xml.Attr{attr(ns, name, value)})
		}
	}
	for _, prefix := range []string{"", "xml", "XML", "xmlns", "xmlfoo", "a"} {
		for _, value := range []string{"", svgNamespace, xmlNamespace, xmlnsURI} {
			ns, name := "xmlns", prefix
			if prefix == "" {
				ns, name = "", "xmlns"
			}
			attrs = append(attrs, []xml.Attr{attr(ns, name, value)})
		}
	}
	for _, element := range []string{"svg", "g", "path", "cc:Work", "dc:type", "sodipodi:namedview"} {
		for _, input := range attrs {
			for _, countLimit := range []int{1, 10} {
				for _, byteLimit := range []int{1, 1000} {
					i := svgImporter{source: "math", ctx: context.Background(), limits: Limits{MaxAttributes: countLimit, MaxAttributeBytes: byteLimit}}
					values, order, metadata, id, err := i.attributes(element, input)
					var output any
					if err == nil {
						output = map[string]any{"values": values, "order": order, "metadata": metadata, "id": id}
					}
					inputs := []map[string]string{}
					for _, a := range input {
						inputs = append(inputs, map[string]string{"namespace": a.Name.Space, "name": a.Name.Local, "value": a.Value})
					}
					rows = append(rows, map[string]any{"operation": "attributes", "element": element, "input": inputs, "maximum_attributes": countLimit,
						"maximum_bytes": byteLimit, "after_attributes": i.parsedAttributes, "after_bytes": i.parsedAttrBytes, "output": output, "error": d2rsError(err)})
				}
			}
		}
	}
	d2rsWrite(t, out, rows)
}
