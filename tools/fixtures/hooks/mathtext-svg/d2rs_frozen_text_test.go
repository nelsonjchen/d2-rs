// SPDX-License-Identifier: MPL-2.0
// Frozen MathJax <text> outlines (frozenMathJaxTextCommands) over one-edit
// variants of the micro tree x command limit x context, plus the text style
// gate (recipe mathtext-svg-frozen-text; needs
// tools/patches/mathtext-svgimport-admission.patch).
package d2svgimport

import (
	"context"
	"strings"
	"testing"
)

func TestD2RSSvgFrozenText(t *testing.T) {
	out := d2rsOutput(t)
	base := `<svg xmlns="http://www.w3.org/2000/svg" role="img" focusable="false" style="vertical-align:-0.5ex;" width="2ex" height="1ex" viewBox="0 -100 1000 1000"><g stroke="currentColor" fill="currentColor" stroke-width="0" transform="scale(1,-1)"><g data-mml-node="math"><g class="MathML-Unit" data-mml-node="mi"><text data-variant="normal" transform="scale(1,-1)" font-size="884px" font-family="serif">µ</text></g></g></g></svg>`
	inputs := []string{base}
	changes := [][2]string{{">µ<", ">x<"}, {">µ<", ">µµ<"}, {">µ<", "> µ<"}, {`font-size="884px"`, `font-size="884.0px"`},
		{`font-family="serif"`, `font-family="sans"`}, {`font-family="serif"`, ``}, {`data-variant="normal"`, `data-variant="bold"`},
		{`data-variant="normal"`, `data-variant="normal "`}, {`data-variant="normal"`, ``}, {`<text data-variant`, `<text fill="red" data-variant`},
		{`>µ</text>`, `>µ<title/></text>`}, {`xmlns="http://www.w3.org/2000/svg"`, ``}, {`role="img"`, `role=" img "`},
		{`focusable="false"`, `focusable=" false "`}, {`style="vertical-align:-0.5ex;"`, `style="color:red; vertical-align:-0.5ex;"`},
		{`width="2ex"`, `width="16px"`}, {`viewBox="0 -100 1000 1000"`, `viewBox="1 -100 1000 1000"`}, {`class="MathML-Unit"`, `class="other"`},
		{`class="MathML-Unit"`, `class="MathML-Unit other"`}, {`class="MathML-Unit"`, `class="MathML-Unit MathML-Unit"`}, {`class="MathML-Unit"`, ``},
		{`data-mml-node="mi"`, `data-mml-node="mo"`}, {`data-mml-node="mi"`, `data-mml-node="mi" data-other="x"`},
		{`class="MathML-Unit"`, `class="MathML-Unit" id="u"`}, {`class="MathML-Unit"`, `class="MathML-Unit" transform="translate(2)"`},
		{`class="MathML-Unit"`, `class="MathML-Unit" transform=" translate(2)"`}, {`class="MathML-Unit"`, `class="MathML-Unit" transform="translate(-2)"`},
		{`class="MathML-Unit"`, `class="MathML-Unit" transform="translate(2,1)"`}, {`class="MathML-Unit"`, `class="MathML-Unit" transform="scale(1)"`},
		{`class="MathML-Unit"`, `class="MathML-Unit" transform="translate(2) scale(1)"`}, {`stroke-width="0"`, `stroke-width="1"`},
		{`data-mml-node="math"`, `data-mml-node="math" class="a"`}, {`data-mml-node="math"`, `data-mml-node="mrow"`}}
	for _, change := range changes {
		inputs = append(inputs, strings.Replace(base, change[0], change[1], 1))
	}
	rows := []map[string]any{}
	for _, input := range inputs {
		for _, maximum := range []int{0, 15, 16, 17} {
			for _, mode := range []string{"", "cancelled", "deadline"} {
				i := svgImporter{ctx: context.Background(), source: "math", d2rsElementAdmissionOnly: true, ids: map[string]*svgElement{},
					limits: Limits{MaxBytes: 10000, MaxDepth: 100, MaxElements: 100, MaxAttributes: 100, MaxAttributeBytes: 10000,
						MaxPathCommands: maximum, MaxTransformFunctions: 100, MaxUseDepth: 100, MaxResources: 100}}
				root, err := i.parse([]byte(input))
				var text *svgElement
				if err == nil {
					err = i.validateNonRenderingTree(root)
				}
				if err == nil {
					err = i.validateGradientTree(root)
				}
				if err == nil {
					err = i.validateClipTree(root)
				}
				if err == nil {
					err = i.compileStylesheets(root)
				}
				if err == nil {
					stack := []*svgElement{root}
					for len(stack) > 0 {
						e := stack[len(stack)-1]
						stack = stack[:len(stack)-1]
						if err = i.compileElement(e); err != nil {
							break
						}
						if e.name == "text" {
							text = e
						}
						for j := len(e.children) - 1; j >= 0; j-- {
							stack = append(stack, e.children[j])
						}
					}
				}
				i.ctx = d2rsContext(t, mode)
				var commands any
				if err == nil {
					path, e := i.frozenMathJaxTextCommands(text)
					err = e
					if e == nil {
						commands = d2rsCommands(path)
					}
				}
				rows = append(rows, map[string]any{"operation": "commands", "input": input, "maximum": maximum, "cancel": mode, "commands": commands,
					"error": d2rsError(err)})
			}
		}
	}
	for _, property := range []string{"fill", "stroke-width", "fill-opacity", "opacity", "display", "visibility", "fill-rule", "clip-path"} {
		for _, value := range []string{"none", "currentColor", "red", "0", "0.5", "1", "inline", "hidden", "evenodd"} {
			e := &svgElement{name: "text", declarations: map[string]string{property: value}}
			i := svgImporter{source: "math", ctx: context.Background()}
			parent := defaultSVGStyle()
			parent.strokeWidth = 0
			style, err := i.computeStyle(parent, e)
			var output any
			if err == nil {
				output = d2rsStyle(style)
				err = i.validateFrozenMathJaxTextStyle(e, style)
			}
			rows = append(rows, map[string]any{"operation": "style", "property": property, "value": value, "style": output, "error": d2rsError(err)})
		}
	}
	d2rsWrite(t, out, rows)
}
