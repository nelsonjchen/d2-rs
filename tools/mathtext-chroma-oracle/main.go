// SPDX-License-Identifier: MPL-2.0
// Oracle for Chroma (pinned by go.mod), D2's code-block highlighter. Run by
// tools/fixtures/mathtext-txt.toml:
//
//	mathtext-chroma-oracle tokens INPUT.json > OUTPUT.json
//	mathtext-chroma-oracle registry EXTRA.json > OUTPUT.json
//
// tokens: INPUT.json is a list of [language, source] pairs
// (scripts/fixture-gen-chroma.py). Each row holds lexers.Get(language)
// (fallback when unknown) token kinds/values and, for the github and
// catppuccin-mocha styles, each token's resolved colour and bold/italic/
// underline flags.
//
// registry: lexers.Get(input).Config().Name ("" when unknown) for inputs
// derived from every registered lexer's name, aliases and filename globs,
// followed by the extra inputs in EXTRA.json.
package main

import (
	"bytes"
	"encoding/json"
	"fmt"
	"os"
	"regexp"
	"strings"

	"github.com/alecthomas/chroma/v2"
	"github.com/alecthomas/chroma/v2/lexers"
	"github.com/alecthomas/chroma/v2/styles"
)

type styled struct {
	Kind      string `json:"kind"`
	Value     string `json:"value"`
	Color     string `json:"color"`
	Bold      bool   `json:"bold"`
	Italic    bool   `json:"italic"`
	Underline bool   `json:"underline"`
}

func tokens(cases [][2]string) any {
	rows := []map[string]any{}
	for _, c := range cases {
		lexer := lexers.Get(c[0])
		if lexer == nil {
			lexer = lexers.Fallback
		}
		it, err := lexer.Tokenise(nil, c[1])
		if err != nil {
			panic(err)
		}
		toks := it.Tokens()
		plain := []map[string]string{}
		for _, t := range toks {
			plain = append(plain, map[string]string{"kind": t.Type.String(), "value": t.Value})
		}
		themed := map[string][]styled{}
		for _, name := range []string{"github", "catppuccin-mocha"} {
			style := styles.Get(name)
			arr := []styled{}
			for _, t := range toks {
				e := style.Get(t.Type)
				color := ""
				if e.Colour.IsSet() {
					color = e.Colour.String()
				}
				arr = append(arr, styled{t.Type.String(), t.Value, color, e.Bold == chroma.Yes, e.Italic == chroma.Yes, e.Underline == chroma.Yes})
			}
			themed[name] = arr
		}
		rows = append(rows, map[string]any{"language": c[0], "source": c[1], "tokens": plain, "styles": themed})
	}
	return rows
}

var charClass = regexp.MustCompile(`\[(.)[^]]*\]`)

// registryInputs derives lookup inputs from each lexer's configuration, in
// registry order without repeats: the name and each alias in their own, lower
// and upper case, and for each filename glob (a [class] reduced to its first
// character) a sample name (* -> sample) in plain, directory, backup (~,
// .bak), template (.in) and upper-case forms, plus the bare extension of
// "*.ext" globs.
func registryInputs() []string {
	seen := map[string]bool{}
	out := []string{}
	add := func(s string) {
		if !seen[s] {
			seen[s] = true
			out = append(out, s)
		}
	}
	for _, lexer := range lexers.GlobalLexerRegistry.Lexers {
		c := lexer.Config()
		for _, name := range append([]string{c.Name}, c.Aliases...) {
			add(name)
			add(strings.ToLower(name))
			add(strings.ToUpper(name))
		}
		for _, glob := range c.Filenames {
			glob = charClass.ReplaceAllString(glob, "$1")
			f := strings.ReplaceAll(glob, "*", "sample")
			for _, v := range []string{f, "dir/" + f, f + "~", f + ".bak", f + ".in", strings.ToUpper(f)} {
				add(v)
			}
			if ext, ok := strings.CutPrefix(glob, "*."); ok {
				add(ext)
			}
		}
	}
	return out
}

func main() {
	if len(os.Args) != 3 {
		fmt.Fprintln(os.Stderr, "usage: mathtext-chroma-oracle tokens|registry INPUT.json")
		os.Exit(2)
	}
	data, err := os.ReadFile(os.Args[2])
	if err != nil {
		panic(err)
	}
	var value any
	switch os.Args[1] {
	case "tokens":
		var cases [][2]string
		if err := json.Unmarshal(data, &cases); err != nil {
			panic(err)
		}
		value = tokens(cases)
	case "registry":
		var extra []string
		if err := json.Unmarshal(data, &extra); err != nil {
			panic(err)
		}
		rows := []map[string]any{}
		for _, input := range append(registryInputs(), extra...) {
			name := ""
			if lexer := lexers.Get(input); lexer != nil {
				name = lexer.Config().Name
			}
			rows = append(rows, map[string]any{"input": input, "name": name})
		}
		value = rows
	default:
		panic("unknown mode " + os.Args[1])
	}
	// D2RS_CHROMA_JSON=indent writes indented JSON without HTML escaping.
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	if os.Getenv("D2RS_CHROMA_JSON") == "indent" {
		enc.SetEscapeHTML(false)
		enc.SetIndent("", "  ")
	}
	if err := enc.Encode(value); err != nil {
		panic(err)
	}
	if _, err := os.Stdout.Write(buf.Bytes()); err != nil {
		panic(err)
	}
}
