// SPDX-License-Identifier: MPL-2.0
// Overlay added to github.com/go-text/typesetting/internal/unicodedata by
// scripts/fixture-gen-go-unicode.py (recipe mathtext-txt-go-unicode).
package unicodedata

// D2RSCompositionRows probes Compose with every stored composition pair and
// the Hangul LV/LVT neighbourhoods: [a, b, composed, ok].
func D2RSCompositionRows() [][4]int64 {
	rows := [][4]int64{}
	add := func(a, b rune) {
		out, ok := rune(0), int64(0)
		if c, valid := Compose(a, b); valid {
			out, ok = c, 1
		}
		rows = append(rows, [4]int64{int64(a), int64(b), int64(out), ok})
	}
	for _, v := range dm2U32Map {
		add(rune((v>>21)&0x7ff), rune(0x300+((v>>14)&0x7f)))
	}
	for _, v := range dm2U64Map {
		add(rune((v>>42)&0x1fffff), rune((v>>21)&0x1fffff))
	}
	for a := rune(HangulSBase - HangulTCount); a <= HangulSBase+HangulSCount; a += HangulTCount {
		for b := rune(HangulTBase - 1); b <= HangulTBase+HangulTCount; b++ {
			add(a, b)
		}
	}
	for a := rune(HangulLBase - 1); a <= HangulLBase+HangulLCount; a++ {
		for b := rune(HangulVBase - 1); b <= HangulVBase+HangulVCount; b++ {
			add(a, b)
		}
	}
	return rows
}

// D2RSStoredCompositionCount is the number of rows from the stored tables.
func D2RSStoredCompositionCount() int { return len(dm2U32Map) + len(dm2U64Map) }
