// SPDX-License-Identifier: MPL-2.0
// The ordered compile pipeline (parse, tree validation, stylesheets, per-element
// compilation up to the first <image>, resources, use graph, styles, clips)
// over small SVG documents x limits (recipe mathtext-svg-compile-pipeline).
// Needs tools/patches/mathtext-svgimport-admission.patch for <image> elements.
package d2svgimport

import (
	"context"
	"image/color"
	"testing"
)

func TestD2RSSvgCompilePipeline(t *testing.T) {
	out := d2rsOutput(t)
	bodies := []string{"", "<g><rect width='10' height='20'/><circle r='3'/><ellipse rx='2' ry='4'/><line x2='3' y2='4'/></g>", "<path d='M0 0L2 3'/>",
		"<path d='M0 0'/>", "<path d='bad'/><g unknown='x'/>", "<rect width='bad'/><g unknown='x'/>", "<g unknown='x'/><rect width='bad'/>",
		"<polyline points='0,0 2,3'/><polygon points='1,2 3,4'/>", "<defs><path id='p' d='M0 0L1 1'/></defs><use href='#p' x='2' y='3'/>",
		"<use href='#p' x='bad'/><path id='p' d='M0 0L1 1'/>", "<use href='#p' x='2' y='bad'/><path id='p' d='M0 0L1 1'/>", "<use href='#missing'/>",
		"<defs id='p'/><use href='#p'/>", "<clipPath id='c'><use href='#p' x='2' y='3' overflow='bad'/></clipPath><path id='p'/>",
		"<svg viewBox='0 0 20 30' width='10' height='20'><path d='M0 0L2 3'/></svg>", "<svg width='bad'/><path d='bad'/>",
		"<defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='0' gradientUnits='userSpaceOnUse'><stop offset='0' stop-color='red'/><stop offset='1' stop-color='blue'/></linearGradient></defs><rect width='2' height='3' fill='url(#g)'/>",
		"<image width='2' height='3' href='data:x'/><g unknown='x'/>", "<g transform='translate(2)'><path d='M0 0L1 2'/><use href='#p'/></g><path id='p'/>",
		"<title>x</title><path class='a' d='M0 0L1 1'/>", "<text>µ</text>"}
	bodies = append(bodies, "<use id='a' href='#a'/>", "<use id='a' href='#b'/><use id='b' href='#a'/>",
		"<use href='#a'/><use id='a' href='#b'/><use id='b' href='#p'/><path id='p'/>", "<path d='M0 0L0 0' stroke='red' stroke-linecap='round'/>",
		"<path d='M0 0L0 0Z' stroke='red' stroke-linecap='round'/>", "<path d='M0 0Q1 1 0 0' stroke='red' stroke-linecap='square'/>",
		"<path d='M0 0C0 0 0 0 0 0' stroke='red' stroke-linecap='round'/>", "<path d='M0 0L0 0M1 1L2 2' stroke='red' stroke-linecap='round'/>",
		"<path d='M0 0L0 0' stroke='red' stroke-linecap='round' display='none'/>", "<style>.unused {fill:bad}</style><path/>",
		"<style>.unused {stop-color:red;fill:blue}</style>", "<style>.unused {stop-color:red}</style>",
		"<clipPath id='c'><rect width='10' height='20' clip-rule='evenodd'/></clipPath><rect width='2' height='3' clip-path='url(#c)'/>",
		"<clipPath id='c'><use href='#p' x='3'/></clipPath><path id='p' d='M0 0L1 1'/><rect clip-path='url(#c)'/>",
		"<clipPath id='c'><path d='M0 0L1 1' fill='red'/></clipPath>", "<clipPath id='c'><path d='M0 0L1 1'/></clipPath><path fill='bad'/>",
		"<defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='0' gradientUnits='userSpaceOnUse'><stop offset='0.9'/><stop offset='0.1'/></linearGradient></defs><style>.unused {fill:bad}</style>",
		"<g opacity='0'><path fill='bad'/></g>")
	inputs := []string{}
	for _, body := range bodies {
		inputs = append(inputs, "<svg><style>.a {fill:red}</style>"+body+"</svg>")
	}
	inputs = append(inputs, `<svg xmlns="http://www.w3.org/2000/svg" role="img" focusable="false" style="vertical-align:-0.5ex;" width="2ex" height="1ex" viewBox="0 -100 1000 1000"><g stroke="currentColor" fill="currentColor" stroke-width="0" transform="scale(1,-1)"><g data-mml-node="math"><g class="MathML-Unit" data-mml-node="mi"><text data-variant="normal" transform="scale(1,-1)" font-size="884px" font-family="serif">µ</text></g></g></g></svg>`)
	rows := []map[string]any{}
	for _, input := range inputs {
		for _, maximum := range []int{2, 100} {
			i := svgImporter{ctx: context.Background(), source: "math", rootCurrentColor: color.NRGBA{R: 1, G: 2, B: 3, A: 200}, ids: map[string]*svgElement{},
				limits: Limits{MaxBytes: 10000, MaxDepth: 100, MaxElements: 100, MaxAttributes: 100, MaxAttributeBytes: 10000,
					MaxPathCommands: maximum, MaxTransformFunctions: maximum, MaxUseDepth: maximum, MaxResources: 100}}
			root, err := i.parse([]byte(input))
			elements := []*svgElement{}
			indices := map[*svgElement]int{}
			if root != nil {
				var walk func(*svgElement)
				walk = func(e *svgElement) {
					indices[e] = len(elements)
					elements = append(elements, e)
					for _, c := range e.children {
						walk(c)
					}
				}
				walk(root)
			}
			pending := -1
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
				for index, e := range elements {
					i.d2rsElementAdmissionOnly = e.name == "image"
					if err = i.compileElement(e); err != nil {
						break
					}
					if e.name == "image" {
						pending = index
						break
					}
				}
			}
			for _, stage := range []func(*svgElement) error{i.compileGradientResources, func(*svgElement) error { return i.validateStylesheetRules() },
				i.validateUseGraph, i.validateStyleTree, i.compileClipPathResources} {
				if err == nil && pending < 0 {
					err = stage(root)
				}
			}
			states := []map[string]any{}
			for _, e := range elements {
				classes := e.classes
				if classes == nil {
					classes = []string{}
				}
				decl := e.declarations
				if decl == nil {
					decl = map[string]string{}
				}
				var target any
				if e.href != "" {
					target = indices[i.ids[e.href]]
				}
				states = append(states, map[string]any{"classes": classes, "declarations": decl,
					"state": map[string]any{"transform": d2rsMatrix(e.transform), "viewport": d2rsViewport(e)}, "geometry": d2rsGeometry(e.geometry),
					"target": target, "position": [2]float64{e.useX, e.useY}, "gradient": d2rsGradient(e.gradient), "stop": d2rsGradientStop(e.gradientStop),
					"clip": d2rsClip(e.clipPath)})
			}
			rows = append(rows, map[string]any{"input": input, "maximum": maximum, "error": d2rsError(err), "pending_image": pending, "states": states,
				"commands": i.parsedPathCommand, "transforms": i.parsedTransforms, "mathjax": i.mathJaxRoot != nil})
		}
	}
	d2rsWrite(t, out, rows)
}
