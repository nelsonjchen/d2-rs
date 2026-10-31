// SPDX-License-Identifier: MPL-2.0
// XML document admission over inputs x limits x modes (recipe mathtext-svg-xml-document).
package d2svgimport

import (
	"context"
	"encoding/json"
	"os"
	"testing"
)

func TestD2RSXMLDocument(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	inputs := []string{"<svg/>", "<svg><g id='a'><path id='p' d='M0 0'/></g></svg>", "<svg xmlns='http://www.w3.org/2000/svg'><g/></svg>", "<svg xmlns='http://www.w3.org/2000/svg'><g xmlns=''/></svg>", "<svg><title>A &amp; B</title><text>hé &lt; x</text><style>.a {fill:red}</style></svg>", "<svg><text><![CDATA[a<b]]></text></svg>", "<svg><style><!--x--></style></svg>", "<svg>bad</svg>", "<svg> \t\n </svg>", "<!--x--><svg/>", "<?xml version='1.0'?><svg/>", " <?xml version='1.0'?><svg/>", "<?xml version='1.1'?><svg/>", "<?xml version='1.0' encoding='UTF-8'?><svg/>", "<?xml version = '1.0' encoding = 'other'?><svg/>", "<?xml version='1.0' standalone='yes'?><svg/>", "<?xml version='1.0' standalone='bad'?><svg/>", "<?xml?><svg/>", "<?pi x?><svg/>", "<!DOCTYPE svg PUBLIC '-//W3C//DTD SVG 1.1//EN' 'http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd'><svg/>", "<!DOCTYPE svg SYSTEM 'x'><svg/>", "<svg/><svg/>", "<path/>", "<svg><g></svg>", "<svg>", "", "<svg id='p'><g id='p'/></svg>", "<svg x='1' x='2'/>", "<svg href='#a' xmlns:x='http://www.w3.org/1999/xlink' x:href='#b'/>", "<svg onload='x'/>", "<svg role='img' data-x='y' focusable='false'/>", "<svg xmlns:a='foreign'><a:g/></svg>", "<svg xmlns:a='foreign' a:x='v'/>", "<svg xmlns:xml='bad'/>", "<svg><script/></svg>", "<svg><pattern/></svg>", "<svg><text>&unknown;</text></svg>", "<svg><text>&#xD;</text></svg>", "<svg><text>\r\n</text></svg>", "<svg><text>&#0;</text></svg>", "<svg id='🙂'/>", "<svg><title id='a'/><metadata id='b'/></svg>"}
	inputs = append(inputs, "\ufeff<svg/>", "\ufeff\ufeff<svg/>", "<?xml version='1.0' encoding='ISO-8859-1'?><svg><text>hé</text></svg>", string(append([]byte("<?xml version='1.0' encoding='ISO-8859-1'?><svg><text>"), append([]byte{0xe9}, []byte("</text></svg>")...)...)), "<svg x='&unknown;'/>", "<svg><g x='&unknown;'/></svg>", "<svg/><svg x='&unknown;'/>", "<svg x=1/>", "<svg><text><!--bad--x--></text></svg>", "<svg><text><![CDATA[bad</text></svg>", "<svg><text>&#xFFFE;</text></svg>")
	rows := []map[string]any{}
	for _, input := range inputs {
		for _, mode := range []string{"default", "depth", "elements", "attributes", "attribute-bytes", "resources"} {
			limits := Limits{MaxBytes: 10000, MaxDepth: 100, MaxElements: 100, MaxAttributes: 100, MaxAttributeBytes: 10000, MaxPathCommands: 100, MaxTransformFunctions: 100, MaxUseDepth: 100, MaxResources: 100}
			switch mode {
			case "depth":
				limits.MaxDepth = 1
			case "elements":
				limits.MaxElements = 1
			case "attributes":
				limits.MaxAttributes = 1
			case "attribute-bytes":
				limits.MaxAttributeBytes = 1
			case "resources":
				limits.MaxResources = 1
			}
			i := svgImporter{source: "math", ctx: context.Background(), limits: limits, ids: map[string]*svgElement{}}
			data := []byte(input)
			if len(data) >= 3 && data[0] == 0xef && data[1] == 0xbb && data[2] == 0xbf {
				data = data[3:]
			}
			root, err := i.parse(data)
			message := ""
			if err != nil {
				message = err.Error()
			}
			var output any
			if root != nil {
				elements := []*svgElement{}
				var walk func(*svgElement)
				walk = func(e *svgElement) {
					elements = append(elements, e)
					for _, child := range e.children {
						walk(child)
					}
				}
				walk(root)
				indices := map[*svgElement]int{}
				for index, e := range elements {
					indices[e] = index
				}
				nodes := []map[string]any{}
				for _, e := range elements {
					children := []int{}
					for _, child := range e.children {
						children = append(children, indices[child])
					}
					var parent any
					if e.parent != nil {
						parent = indices[e.parent]
					}
					text := []int{}
					for _, b := range e.text {
						text = append(text, int(b))
					}
					nodes = append(nodes, map[string]any{"name": e.name, "attributes": map[string]any{"values": e.attrs, "order": e.attrOrder, "metadata": e.metadata, "id": e.id}, "children": children, "parent": parent, "text": text})
				}
				ids := map[string]int{}
				for id, e := range i.ids {
					ids[id] = indices[e]
				}
				output = map[string]any{"namespace": i.elementNamespace, "elements": nodes, "ids": ids, "counts": [4]int{i.parsedElements, i.parsedAttributes, i.parsedAttrBytes, i.stylesheetBytes}}
			}
			rawBytes := []int{}
			for _, b := range []byte(input) {
				rawBytes = append(rawBytes, int(b))
			}
			rows = append(rows, map[string]any{"input_bytes": rawBytes, "input": input, "mode": mode, "limits": [9]int{limits.MaxBytes, limits.MaxDepth, limits.MaxElements, limits.MaxAttributes, limits.MaxAttributeBytes, limits.MaxPathCommands, limits.MaxTransformFunctions, limits.MaxUseDepth, limits.MaxResources}, "error": message, "output": output})
		}
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); err != nil {
		t.Fatal(err)
	}
}
