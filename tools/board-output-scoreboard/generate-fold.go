// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2009 The Go Authors. All rights reserved.
// Run in the pinned trace module with the absolute shared Go caches.
package main

import (
	"encoding/json"
	"fmt"
	"golang.org/x/text/cases"
	"golang.org/x/text/unicode/norm"
	"os"
	"path/filepath"
	"reflect"
	"strings"
)

func main() {
	if cases.UnicodeVersion != "17.0.0" || norm.Version != "17.0.0" {
		panic("requires pinned Go 1.27 x/text Unicode 17.0.0 tables")
	}
	fold := cases.Fold()
	if len(os.Args) > 1 && os.Args[1] == "--fixtures" {
		inputs := []string{"", ".", "./board", "a/../B", "/a/../../Board", "a//b", "é.svg", "e\u0301.svg", "Straße.svg", "STRASSE.svg", "Σ.svg", "ς.svg", "ﬃ.svg", "İ.svg", "i\u0307.svg", "ᾲ.svg", "x" + strings.Repeat("\u0301", 31), "x" + strings.Repeat("\u0344", 16), "가" + strings.Repeat("\u0301", 30), strings.Repeat("\u1161", 31), "x" + strings.Repeat("\u0f73", 16), "\ua7cb.svg", "\U000105d2\u0307.svg"}
		rows := []map[string]string{}
		for _, input := range inputs {
			rows = append(rows, map[string]string{"input": input, "clean": filepath.Clean(input), "output": norm.NFC.String(fold.String(filepath.Clean(input)))})
		}
		enc := json.NewEncoder(os.Stdout)
		enc.SetIndent("", "  ")
		if err := enc.Encode(rows); err != nil {
			panic(err)
		}
		return
	}
	fmt.Println("// SPDX-License-Identifier: BSD-3-Clause\n// Copyright 2009 The Go Authors. All rights reserved.\n// Generated from pinned golang.org/x/text v0.41.0 (Go 1.27: Unicode 17.0.0).\n// Rebuild: tools/board-output-scoreboard/generate-fold.go\n#[rustfmt::skip]\npub(super) const FOLD: &[(char, &str)] = &[")
	for r := rune(0); r <= 0x10ffff; r++ {
		if r >= 0xd800 && r <= 0xdfff {
			continue
		}
		s := string(r)
		f := fold.String(s)
		if s != f {
			fmt.Printf("('\\u{%x}', \"", r)
			for _, q := range f {
				fmt.Printf("\\u{%x}", q)
			}
			fmt.Println("\"),")
		}
	}
	fmt.Println("];\n#[rustfmt::skip]\npub(super) const STREAM_SAFE: &[(char, u8, u8)] = &[")
	// Read the pinned properties rather than substitute UAX15's different
	// treatment of backward-combining starters. No private source is mutated.
	for r := rune(0); r <= 0x10ffff; r++ {
		if r >= 0xd800 && r <= 0xdfff || r >= 0xac00 && r < 0xd7a4 {
			continue
		}
		p := reflect.ValueOf(norm.NFC.PropertiesString(string(r)))
		lead := p.FieldByName("nLead").Uint()
		trail := p.FieldByName("flags").Uint() & 3
		if lead != 0 || trail != 0 {
			fmt.Printf("('\\u{%x}', %d, %d),\n", r, lead, trail)
		}
	}
	fmt.Println("];")
}
