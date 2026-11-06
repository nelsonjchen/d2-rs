// SPDX-License-Identifier: BSD-3-Clause
// Development-only comparison against the pinned Go standard library
// (recipe lang-public-url-validation): net/url Parse / ParseRequestURI
// diagnostics and the D2 remote/host predicates.
//go:debug default=go1.27

package main

import (
	"encoding/hex"
	"encoding/json"
	"net/url"
	"os"
	"strings"
)

type request struct {
	Hex     string `json:"hex"`
	Request bool   `json:"request"`
}

func main() {
	var requests []request
	if err := json.NewDecoder(os.Stdin).Decode(&requests); err != nil {
		panic(err)
	}
	out := make([]any, 0, len(requests))
	for _, r := range requests {
		b, err := hex.DecodeString(r.Hex)
		if err != nil {
			panic(err)
		}
		var u *url.URL
		if r.Request {
			u, err = url.ParseRequestURI(string(b))
		} else {
			u, err = url.Parse(string(b))
		}
		if err != nil {
			out = append(out, map[string]any{"error": err.Error()})
		} else {
			out = append(out, map[string]any{"remote": u.Scheme != "" || strings.HasPrefix(u.Path, "/"), "has_host": u.Host != ""})
		}
	}
	json.NewEncoder(os.Stdout).Encode(out)
}
