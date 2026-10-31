// SPDX-License-Identifier: BSD-3-Clause
// Development-only comparison against the pinned Go standard library.
package main

import (
	"encoding/hex"
	"encoding/json"
	"html"
	"net/url"
	"os"
	"strings"
)

func main() {
	var requests []string
	if e := json.NewDecoder(os.Stdin).Decode(&requests); e != nil {
		panic(e)
	}
	out := []any{}
	for _, r := range requests {
		b, e := hex.DecodeString(r)
		if e != nil {
			panic(e)
		}
		decoded := html.UnescapeString(string(b))
		u, e := url.Parse(decoded)
		out = append(out, map[string]any{"decoded": hex.EncodeToString([]byte(decoded)), "remote": e == nil && (u.Scheme != "" || strings.HasPrefix(u.Path, "/"))})
	}
	json.NewEncoder(os.Stdout).Encode(out)
}
