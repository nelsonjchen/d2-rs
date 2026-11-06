// SPDX-License-Identifier: MPL-2.0 AND MIT
// RenderMarkdown HTML and D2's <div>-wrapped encoding/xml validation over the
// seed requests in tools/fixtures/inputs/mathtext-txt/ (recipes
// mathtext-txt-markdown-{writer,list-grammar,bytes,links} and
// mathtext-txt-compiler-markdown-xml), adapted from the v0.9.0 probes.
package textmeasure

import (
	"encoding/hex"
	"encoding/json"
	"encoding/xml"
	"os"
	"testing"

	"github.com/yuin/goldmark/ast"
	gmtext "github.com/yuin/goldmark/text"
)

func d2rsMarkdownIO(t *testing.T, v any) string {
	out := os.Getenv("D2RS_RECIPE_OUTPUT")
	if out == "" {
		t.Skip("run via scripts/regen-fixtures.py")
	}
	data, err := os.ReadFile(os.Getenv("D2RS_RECIPE_INPUT"))
	if err != nil {
		t.Fatal(err)
	}
	if err := json.Unmarshal(data, v); err != nil {
		t.Fatal(err)
	}
	return out
}

func d2rsMarkdownWrite(t *testing.T, out string, data []byte, err error) {
	if err != nil {
		t.Fatal(err)
	}
	if err := os.WriteFile(out, append(data, '\n'), 0o644); err != nil {
		t.Fatal(err)
	}
}

// d2rsXMLError is the encoding/xml message D2 reports for rendered Markdown.
func d2rsXMLError(document string) (string, bool) {
	var parsed interface{}
	err := xml.Unmarshal([]byte(document), &parsed)
	if err == nil {
		return "", false
	}
	if syntax, ok := err.(*xml.SyntaxError); ok {
		return syntax.Msg, true
	}
	return err.Error(), true
}

func TestD2RSMarkdownWriter(t *testing.T) {
	var requests []struct{ Name, Markdown string }
	out := d2rsMarkdownIO(t, &requests)
	rows := []map[string]any{}
	for _, r := range requests {
		html, err := RenderMarkdown(r.Markdown)
		message := ""
		if err != nil {
			message = err.Error()
		}
		rows = append(rows, map[string]any{"request": r, "html": html, "error": message})
	}
	data, err := json.Marshal(rows)
	d2rsMarkdownWrite(t, out, data, err)
}

func TestD2RSMarkdownListGrammar(t *testing.T) {
	var requests []struct {
		Name, Markdown, Family string
		FontSize               int
	}
	out := d2rsMarkdownIO(t, &requests)
	rows := []map[string]any{}
	for _, r := range requests {
		doc := markdownRenderer.Parser().Parse(gmtext.NewReader([]byte(r.Markdown)))
		nodes := []map[string]any{}
		ids := map[ast.Node]int{}
		err := ast.Walk(doc, func(n ast.Node, entering bool) (ast.WalkStatus, error) {
			if !entering {
				return ast.WalkContinue, nil
			}
			id := len(ids)
			ids[n] = id
			parent := -1
			if n.Parent() != nil {
				parent = ids[n.Parent()]
			}
			if n.Type() == ast.TypeBlock {
				v := map[string]any{"id": id, "parent": parent, "kind": n.Kind().String(), "blank_previous": n.HasBlankPreviousLines(),
					"position": n.Pos(), "children": n.ChildCount()}
				if l, ok := n.(*ast.List); ok {
					v["tight"] = l.IsTight
				}
				if item, ok := n.(*ast.ListItem); ok {
					v["offset"] = item.Offset
				}
				nodes = append(nodes, v)
			}
			return ast.WalkContinue, nil
		})
		if err != nil {
			t.Fatal(err)
		}
		html, err := RenderMarkdown(r.Markdown)
		if err != nil {
			t.Fatal(err)
		}
		rows = append(rows, map[string]any{"request": r, "blocks": nodes, "html": html})
	}
	data, err := json.Marshal(rows)
	d2rsMarkdownWrite(t, out, data, err)
}

func TestD2RSMarkdownBytes(t *testing.T) {
	var rows []map[string]any
	out := d2rsMarkdownIO(t, &rows)
	for _, row := range rows {
		var document string
		if value, ok := row["source_hex"]; ok {
			raw, err := hex.DecodeString(value.(string))
			if err != nil {
				t.Fatal(err)
			}
			rendered, err := RenderMarkdown(string(raw))
			if err != nil {
				row["render_error"] = err.Error()
				continue
			}
			row["html_hex"] = hex.EncodeToString([]byte(rendered))
			document = "<div>" + rendered + "</div>"
		} else {
			raw, err := hex.DecodeString(row["xml_hex"].(string))
			if err != nil {
				t.Fatal(err)
			}
			document = string(raw)
		}
		row["xml_error"], _ = d2rsXMLError(document)
	}
	data, err := json.MarshalIndent(rows, "", "  ")
	d2rsMarkdownWrite(t, out, data, err)
}

func TestD2RSMarkdownLinks(t *testing.T) {
	var requests []string
	out := d2rsMarkdownIO(t, &requests)
	type result struct {
		Error    *string `json:"error"`
		HTMLHex  string  `json:"html_hex"`
		XMLError *string `json:"xml_error"`
	}
	value := struct {
		Requests []map[string]string `json:"requests"`
		Source   []result            `json:"source"`
	}{Requests: []map[string]string{}, Source: []result{}}
	for _, request := range requests {
		raw, err := hex.DecodeString(request)
		if err != nil {
			t.Fatal(err)
		}
		value.Requests = append(value.Requests, map[string]string{"hex": request})
		r := result{}
		html, err := RenderMarkdown(string(raw))
		if err != nil {
			message := err.Error()
			r.Error = &message
		} else {
			r.HTMLHex = hex.EncodeToString([]byte(html))
			if message, failed := d2rsXMLError("<div>" + html + "</div>"); failed {
				r.XMLError = &message
			}
		}
		value.Source = append(value.Source, r)
	}
	data, err := json.MarshalIndent(value, "", "  ")
	d2rsMarkdownWrite(t, out, data, err)
}

func TestD2RSCompilerMarkdownXML(t *testing.T) {
	var cases []struct {
		Name     string `json:"name"`
		Source   string `json:"source"`
		Markdown bool   `json:"markdown"`
	}
	out := d2rsMarkdownIO(t, &cases)
	rows := []any{}
	for _, c := range cases {
		html := c.Source
		if c.Markdown {
			var err error
			if html, err = RenderMarkdown(c.Source); err != nil {
				t.Fatal(err)
			}
		}
		message, _ := d2rsXMLError("<div>" + html + "</div>")
		rows = append(rows, map[string]any{"name": c.Name, "source": c.Source, "markdown": c.Markdown, "html": html, "error": message})
	}
	data, err := json.Marshal(rows)
	d2rsMarkdownWrite(t, out, data, err)
}
