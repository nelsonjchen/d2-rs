// SPDX-License-Identifier: MPL-2.0
// compile admission of small documents (recipe mathtext-svg-compile-admission).
package d2svgimport

import (
	"context"
	"encoding/json"
	"os"
	"testing"
)

func TestD2RSCompileAdmission(t *testing.T) {
	if os.Getenv("D2RS_RECIPE_OUTPUT") == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	inputs := []string{}
	bodies := []string{"", "<title/>", "<metadata/>", "<g><metadata/></g>", "<title><path/></title>", "<s:namedview/>", "<g><s:namedview/></g>", "<metadata><path/></metadata>", "<r:RDF/>", "<c:Work/>", "<d:format>image/svg+xml</d:format>", "<metadata><r:RDF/></metadata>", "<metadata><r:RDF><c:Work/></r:RDF></metadata>", "<metadata><r:RDF><c:Work><d:format>image/svg+xml</d:format><d:type/></c:Work></r:RDF></metadata>", "<metadata><r:RDF><c:Work><d:type/><d:format> image/svg+xml </d:format></c:Work></r:RDF></metadata>", "<metadata><r:RDF><c:Work><d:format>wrong</d:format><d:type/></c:Work></r:RDF></metadata>", "<metadata><r:RDF><c:Work><d:type/><d:type/></c:Work></r:RDF></metadata>", "<metadata><r:RDF><c:Work><path/></c:Work></r:RDF></metadata>", "<metadata><r:RDF/><r:RDF/></metadata>", "<metadata><r:RDF><c:Work/><c:Work/></r:RDF></metadata>", "<metadata/><metadata><r:RDF/></metadata>", "<defs><linearGradient id='g'><stop offset='0'/></linearGradient></defs>", "<linearGradient id='g'><stop/></linearGradient>", "<defs><linearGradient><stop/></linearGradient></defs>", "<defs><linearGradient id='g'/></defs>", "<defs><linearGradient id='g'><path/></linearGradient></defs>", "<stop/>", "<defs><linearGradient id='g'><stop><path/></stop></linearGradient></defs>", "<clipPath id='c'><path/></clipPath>", "<clipPath><path/></clipPath>", "<path><clipPath id='c'><path/></clipPath></path>", "<clipPath id='c'/>", "<clipPath id='c'><g/></clipPath>", "<clipPath id='c'><path/><path/></clipPath>", "<clipPath id='c' clipPathUnits='objectBoundingBox'><path/></clipPath>", "<clipPath id='c'><use href='#p' overflow='visible'/></clipPath><path id='p'/>", "<path overflow='visible'/>", "<g><use overflow='visible'/></g>"}
	root := "<svg xmlns:r='http://www.w3.org/1999/02/22-rdf-syntax-ns#' xmlns:c='http://creativecommons.org/ns#' xmlns:d='http://purl.org/dc/elements/1.1/' xmlns:s='http://sodipodi.sourceforge.net/DTD/sodipodi-0.dtd'>"
	styles := []string{"", "<style>.a { fill:red }</style>", "<style>.a { fill:blue }</style><style>bad</style>", "<style id='x'>.a {fill:red}</style>", "<style type='bad'/>", "<style>.a { unknown:x }</style>", "<style>.a {fill:red;fill:blue}.b {opacity:0.5}</style>", "<style><title/></style>"}
	for _, body := range bodies {
		for _, style := range styles {
			inputs = append(inputs, root+style+body+"</svg>")
		}
	}
	rows := []map[string]any{}
	for _, input := range inputs {
		i := svgImporter{source: "math", ctx: context.Background(), limits: Limits{MaxBytes: 10000, MaxDepth: 100, MaxElements: 100, MaxAttributes: 100, MaxAttributeBytes: 10000, MaxPathCommands: 100, MaxTransformFunctions: 100, MaxUseDepth: 100, MaxResources: 100}, ids: map[string]*svgElement{}}
		root, err := i.parse([]byte(input))
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
		message := ""
		if err != nil {
			message = err.Error()
		}
		rules := []map[string]any{}
		for _, rule := range i.stylesheetRules {
			rules = append(rules, map[string]any{"class": rule.class, "declarations": rule.declarations})
		}
		elements := 0
		if err == nil {
			elements = i.parsedElements
		}
		rows = append(rows, map[string]any{"input": input, "error": message, "rules": rules, "elements": elements})
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	if err != nil {
		t.Fatal(err)
	}
	if err = os.WriteFile(os.Getenv("D2RS_RECIPE_OUTPUT"), data, 0644); err != nil {
		t.Fatal(err)
	}
}
