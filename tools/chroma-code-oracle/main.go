// Oracle: Chroma v2.27.0 token streams, registry names and d2svg styleToSVG maps.
// Run by recipe mathtext-txt-chroma-bash-styles (tools/fixtures/mathtext-txt.toml): cases.py output on stdin.
package main

import (
	"encoding/json"
	"os"

	"github.com/alecthomas/chroma/v2"
	"github.com/alecthomas/chroma/v2/formatters/svg"
	"github.com/alecthomas/chroma/v2/lexers"
	"github.com/alecthomas/chroma/v2/styles"
)

type tok struct {
	Kind  string `json:"kind"`
	Value string `json:"value"`
}

func main() {
	var cases []struct {
		Language string `json:"language"`
		Source   string `json:"source"`
	}
	if err := json.NewDecoder(os.Stdin).Decode(&cases); err != nil {
		panic(err)
	}
	out := map[string]any{}
	rows := []map[string]any{}
	for _, c := range cases {
		lexer := lexers.Get(c.Language)
		name := "fallback"
		if lexer == nil {
			lexer = lexers.Fallback
		} else {
			name = lexer.Config().Name
		}
		it, err := lexer.Tokenise(nil, c.Source)
		if err != nil {
			panic(err)
		}
		toks := []tok{}
		for _, t := range it.Tokens() {
			toks = append(toks, tok{t.Type.String(), t.Value})
		}
		rows = append(rows, map[string]any{"language": c.Language, "lexer": name, "source": c.Source, "tokens": toks})
	}
	out["cases"] = rows
	for _, s := range []string{"github", "catppuccin-mocha"} {
		style := styles.Get(s)
		m := map[string]string{}
		for t := range chroma.StandardTypes {
			e := style.Get(t)
			if e.IsZero() {
				continue
			}
			m[t.String()] = svg.StyleEntryToSVG(e)
		}
		out[s] = map[string]any{"svg": m, "background": style.Get(chroma.Background).Background.String()}
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", " ")
	enc.Encode(out)
}
