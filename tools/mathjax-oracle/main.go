// SPDX-License-Identifier: MPL-2.0
// Oracle for github.com/d2lang/mathjax-go mathjax.Render at the version D2's
// pinned go.mod requires (the call behind d2latex.Render).
//
// Without arguments it reads a JSON array of TeX strings on stdin and writes
// a JSON array of {"svg": mathjax.Render(tex), "error": err} to stdout
// (scripts/mathjax-exact-fixtures.py).
//
// With a ledger path (recipe mathtext-latex-mathjax-exact,
// tools/fixtures/mathtext-latex.toml) it reads [[tex, outcome], ...], the
// committed render_exact outcome classes, and writes the
// crates/d2-rs-math/tests/fixtures/mathjax-exact-v090.json fixture:
// {oracle, counts, cases: [{tex, sha256 of the Go SVG, outcome}]}, skipping
// expressions that mathjax-go rejects.
package main

import (
	"bytes"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"runtime/debug"

	mathjax "github.com/d2lang/mathjax-go"
)

type result struct {
	SVG   string `json:"svg"`
	Error string `json:"error,omitempty"`
}

type counts struct {
	Match    int `json:"match"`
	Refused  int `json:"refused"`
	Mismatch int `json:"mismatch"`
}

type exactCase struct {
	Tex     string `json:"tex"`
	SHA256  string `json:"sha256"`
	Outcome string `json:"outcome"`
}

type fixture struct {
	Oracle string      `json:"oracle"`
	Counts counts      `json:"counts"`
	Cases  []exactCase `json:"cases"`
}

func mathjaxVersion() string {
	if info, ok := debug.ReadBuildInfo(); ok {
		for _, dep := range info.Deps {
			if dep.Path == "github.com/d2lang/mathjax-go" {
				return dep.Version
			}
		}
	}
	panic("mathjax-go version not in build info")
}

func ledger(path string) {
	data, err := os.ReadFile(path)
	if err != nil {
		panic(err)
	}
	var entries [][2]string
	if err := json.Unmarshal(data, &entries); err != nil {
		panic(err)
	}
	out := fixture{
		Oracle: fmt.Sprintf("github.com/d2lang/mathjax-go %s mathjax.Render (tools/mathjax-oracle)", mathjaxVersion()),
		Cases:  []exactCase{},
	}
	for _, entry := range entries {
		tex, outcome := entry[0], entry[1]
		svg, err := mathjax.Render(tex)
		if err != nil {
			continue
		}
		sum := sha256.Sum256([]byte(svg))
		out.Cases = append(out.Cases, exactCase{tex, hex.EncodeToString(sum[:]), outcome})
		switch outcome {
		case "match":
			out.Counts.Match++
		case "refused":
			out.Counts.Refused++
		case "mismatch":
			out.Counts.Mismatch++
		default:
			panic("unknown outcome " + outcome)
		}
	}
	var buf bytes.Buffer
	enc := json.NewEncoder(&buf)
	enc.SetEscapeHTML(false)
	enc.SetIndent("", "")
	if err := enc.Encode(out); err != nil {
		panic(err)
	}
	if _, err := os.Stdout.Write(buf.Bytes()); err != nil {
		panic(err)
	}
}

func main() {
	if len(os.Args) == 2 {
		ledger(os.Args[1])
		return
	}
	var inputs []string
	if err := json.NewDecoder(os.Stdin).Decode(&inputs); err != nil {
		panic(err)
	}
	out := make([]result, len(inputs))
	for i, tex := range inputs {
		svg, err := mathjax.Render(tex)
		out[i].SVG = svg
		if err != nil {
			out[i].Error = err.Error()
		}
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(out); err != nil {
		panic(err)
	}
}
