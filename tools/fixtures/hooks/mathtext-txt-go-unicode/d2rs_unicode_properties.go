// SPDX-License-Identifier: MPL-2.0
// Overlay added to github.com/go-text/typesetting/harfbuzz by
// scripts/fixture-gen-go-unicode.py (recipe mathtext-txt-go-unicode).
package harfbuzz

import (
	"encoding/json"
	"io"

	"github.com/go-text/typesetting/internal/unicodedata"
	"github.com/go-text/typesetting/language"
)

// D2RSUnicodeProperties writes, for every Unicode scalar value, run-length
// ranges of [first, last, script, general category, combining class, mirror
// delta, modified combining class], every canonical decomposition with its
// recomposition, and the reverse composition table probes.
func D2RSUnicodeProperties(w io.Writer) error {
	ranges := [][7]int64{}
	decomposition := [][4]int64{}
	var previous [7]int64
	have := false
	for ch := rune(0); ch <= 0x10ffff; ch++ {
		if ch >= 0xd800 && ch <= 0xdfff {
			continue
		}
		row := [7]int64{int64(ch), int64(ch), int64(language.LookupScript(ch)), int64(unicodedata.LookupGeneralCategory(ch)),
			int64(unicodedata.LookupCombiningClass(ch)), int64(unicodedata.LookupMirrorChar(ch) - ch), int64(uniModifiedCombiningClass(ch))}
		if have && previous[1]+1 == row[0] && previous[2] == row[2] && previous[3] == row[3] && previous[4] == row[4] && previous[5] == row[5] && previous[6] == row[6] {
			previous[1] = row[1]
		} else {
			if have {
				ranges = append(ranges, previous)
			}
			previous = row
			have = true
		}
		if a, b, ok := unicodedata.Decompose(ch); ok {
			composed := rune(0)
			if c, ok := unicodedata.Compose(a, b); ok {
				composed = c
			}
			decomposition = append(decomposition, [4]int64{int64(ch), int64(a), int64(b), int64(composed)})
		}
	}
	ranges = append(ranges, previous)
	return json.NewEncoder(w).Encode(map[string]any{
		"ranges":                   ranges,
		"composition":              unicodedata.D2RSCompositionRows(),
		"stored_composition_count": unicodedata.D2RSStoredCompositionCount(),
		"decomposition":            decomposition,
		"scalar_count":             0x110000 - 0x800,
		"contract":                 "all Unicode scalar values;Source script/category/canonical combining class/mirror/decomposition and reverse composition",
	})
}
