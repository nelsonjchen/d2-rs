// SPDX-License-Identifier: MPL-2.0
// Development reference only. No production Rust package invokes this program.
//
// font-subset-oracle exercises the pinned D2 v0.9.0 font embedding pipeline
// (d2renderers/d2fonts.Font.GetEncodedSubset -> lib/font.UTF8CutFont ->
// lib/font.Sfnt2Woff) and emits digests that crates/d2-rs-text tests compare
// against.
//
// Usage:
//
//	font-subset-oracle cases <cases.json> [dump-dir]
//	    cases.json: [{"family":"SourceSansPro","style":"regular","corpus":"..."}]
//	    Prints a JSON array with the cut SFNT, WOFF and data-URI digests.
//	    With dump-dir, writes <index>.ttf and <index>.woff for each case.
//	font-subset-oracle encodings
//	    Checks every bundled FontEncodings entry against base64(FontFaces).
//	font-subset-oracle zlib <file>...
//	    Digests of zlib Write+Close versus Write+Flush+Close for each file.
//	font-subset-oracle golden <svg>...
//	    Recovers every embedded subset font's character set from its cmap,
//	    re-runs GetEncodedSubset and prints a fixture line per unique font.
package main

import (
	"bytes"
	"compress/zlib"
	"crypto/sha256"
	"encoding/base64"
	"encoding/binary"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"os"
	"path/filepath"
	"regexp"
	"sort"
	"strings"

	"github.com/d2lang/d2/d2renderers/d2fonts"
	"github.com/d2lang/d2/lib/font"
)

type fontCase struct {
	Family string `json:"family"`
	Style  string `json:"style"`
	Corpus string `json:"corpus"`
}

type caseResult struct {
	Family    string `json:"family"`
	Style     string `json:"style"`
	Corpus    string `json:"corpus"`
	CutSHA256 string `json:"cut_sha256"`
	CutLen    int    `json:"cut_len"`
	WoffSHA   string `json:"woff_sha256"`
	WoffLen   int    `json:"woff_len"`
	URISHA256 string `json:"uri_sha256"`
	Fallback  bool   `json:"fallback"`
}

func digest(data []byte) string {
	sum := sha256.Sum256(data)
	return hex.EncodeToString(sum[:])
}

func uniqueChars(corpus string) string {
	var unique string
	seen := map[rune]bool{}
	for _, char := range corpus {
		if !seen[char] {
			seen[char] = true
			unique += string(char)
		}
	}
	return unique
}

func runCase(c fontCase) (caseResult, []byte, []byte) {
	f := d2fonts.FontFamily(c.Family).Font(0, d2fonts.FontStyle(c.Style))
	face := d2fonts.FontFaces.Get(f)
	buf := make([]byte, len(face))
	copy(buf, face)
	cut := font.UTF8CutFont(buf, uniqueChars(c.Corpus))
	cutCopy := append([]byte{}, cut...)
	woff, err := font.Sfnt2Woff(cut)
	uri := f.GetEncodedSubset(c.Corpus)
	return caseResult{
		Family:    c.Family,
		Style:     c.Style,
		Corpus:    c.Corpus,
		CutSHA256: digest(cutCopy),
		CutLen:    len(cutCopy),
		WoffSHA:   digest(woff),
		WoffLen:   len(woff),
		URISHA256: digest([]byte(uri)),
		Fallback:  err != nil,
	}, cutCopy, woff
}

func cases(path, dumpDir string) error {
	data, err := os.ReadFile(path)
	if err != nil {
		return err
	}
	var input []fontCase
	if err := json.Unmarshal(data, &input); err != nil {
		return err
	}
	var out []caseResult
	for i, c := range input {
		result, cut, woff := runCase(c)
		out = append(out, result)
		if dumpDir != "" {
			if err := os.WriteFile(filepath.Join(dumpDir, fmt.Sprintf("%d.ttf", i)), cut, 0o644); err != nil {
				return err
			}
			if err := os.WriteFile(filepath.Join(dumpDir, fmt.Sprintf("%d.woff", i)), woff, 0o644); err != nil {
				return err
			}
		}
	}
	enc := json.NewEncoder(os.Stdout)
	enc.SetIndent("", " ")
	enc.SetEscapeHTML(false)
	return enc.Encode(out)
}

var families = []d2fonts.FontFamily{d2fonts.SourceSansPro, d2fonts.SourceCodePro, d2fonts.HandDrawn}

func encodings() error {
	for _, family := range families {
		for _, style := range d2fonts.FontStyles {
			f := family.Font(0, style)
			face := d2fonts.FontFaces.Get(f)
			expected := "data:application/font-woff;base64," + base64.StdEncoding.EncodeToString(face)
			encoding := d2fonts.FontEncodings.Get(f)
			fmt.Printf("%s %s ttf=%s encoding=%s raw_base64_match=%v\n", family, style, digest(face), digest([]byte(encoding)), expected == encoding)
		}
	}
	return nil
}

// woffTable decodes one table of a WOFF produced by font.Sfnt2Woff.
func woffTable(woff []byte, tag string) ([]byte, error) {
	numTables := int(binary.BigEndian.Uint16(woff[12:]))
	for i := 0; i < numTables; i++ {
		entry := woff[44+i*20:]
		if string(entry[:4]) != tag {
			continue
		}
		offset := binary.BigEndian.Uint32(entry[4:])
		compLength := binary.BigEndian.Uint32(entry[8:])
		length := binary.BigEndian.Uint32(entry[12:])
		data := woff[offset : offset+compLength]
		if compLength == length {
			return data, nil
		}
		reader, err := zlib.NewReader(bytes.NewReader(data))
		if err != nil {
			return nil, err
		}
		return io.ReadAll(reader)
	}
	return nil, fmt.Errorf("missing table %q", tag)
}

// cmapChars returns every character covered by the subset's format-4 cmap,
// which is exactly the set of corpus characters present in the source cmap.
func cmapChars(cmap []byte) string {
	sub := cmap[12:]
	segCount := int(binary.BigEndian.Uint16(sub[6:])) / 2
	var chars []rune
	for i := 0; i < segCount; i++ {
		end := int(binary.BigEndian.Uint16(sub[14+i*2:]))
		start := int(binary.BigEndian.Uint16(sub[16+segCount*2+i*2:]))
		if start == 0xFFFF && end == 0xFFFF {
			continue
		}
		for c := start; c <= end; c++ {
			chars = append(chars, rune(c))
		}
	}
	return string(chars)
}

var fontFaceRE = regexp.MustCompile(`font-family: [A-Za-z0-9-]*font-(regular|bold|semibold|italic|mono|mono-bold|mono-semibold|mono-italic);\s*src: url\("(data:application/font-woff;base64,[A-Za-z0-9+/=]*)"\)`)

type goldenEntry struct {
	Family    string `json:"family"`
	Style     string `json:"style"`
	Chars     string `json:"chars"`
	CutSHA256 string `json:"cut_sha256"`
	URISHA256 string `json:"uri_sha256"`
	// GoldenZlibDiffers marks a golden whose decompressed WOFF (header,
	// directory and every table) equals the pinned pipeline's output while its
	// zlib payload bytes differ, i.e. the golden predates the current encoder.
	// URISHA256 is then the pinned pipeline's digest, not the golden's.
	GoldenZlibDiffers bool `json:"golden_zlib_differs,omitempty"`
}

// woffDecoded is a WOFF with every table decompressed; compressed lengths and
// table offsets are excluded because they depend on the zlib encoder.
func woffDecoded(woff []byte) (string, error) {
	var out bytes.Buffer
	out.Write(woff[4:8])   // flavor
	out.Write(woff[12:14]) // numTables
	out.Write(woff[16:24]) // totalSfntSize, majorVersion, minorVersion
	numTables := int(binary.BigEndian.Uint16(woff[12:]))
	for i := 0; i < numTables; i++ {
		entry := woff[44+i*20:]
		out.Write(entry[:4])
		out.Write(entry[12:20]) // origLength, origChecksum
		data, err := woffTable(woff, string(entry[:4]))
		if err != nil {
			return "", err
		}
		out.Write(data)
	}
	return digest(out.Bytes()), nil
}

func golden(paths []string) error {
	seen := map[string]bool{}
	var entries []goldenEntry
	unmatched := 0
	zlibDiffers := 0
	fullEncodings := 0
	for _, path := range paths {
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		for _, m := range fontFaceRE.FindAllStringSubmatch(string(data), -1) {
			uri := m[2]
			if seen[uri] {
				continue
			}
			seen[uri] = true
			kind := m[1]
			style := strings.TrimPrefix(strings.TrimPrefix(kind, "mono"), "-")
			if style == "" {
				style = "regular"
			}
			woff, err := base64.StdEncoding.DecodeString(strings.TrimPrefix(uri, "data:application/font-woff;base64,"))
			if err != nil {
				return err
			}
			if binary.BigEndian.Uint32(woff) != 0x774f4646 {
				fullEncodings++
				continue
			}
			cmap, err := woffTable(woff, "cmap")
			if err != nil {
				return fmt.Errorf("%s: %w", path, err)
			}
			chars := cmapChars(cmap)
			matched := false
			for _, family := range families {
				f := family.Font(0, d2fonts.FontStyle(style))
				if f.GetEncodedSubset(chars) == uri {
					result, _, _ := runCase(fontCase{Family: string(family), Style: style, Corpus: chars})
					entries = append(entries, goldenEntry{string(family), style, chars, result.CutSHA256, digest([]byte(uri)), false})
					matched = true
					break
				}
			}
			if !matched {
				goldenDecoded, err := woffDecoded(woff)
				if err != nil {
					return err
				}
				for _, family := range families {
					result, _, ours := runCase(fontCase{Family: string(family), Style: style, Corpus: chars})
					oursDecoded, err := woffDecoded(ours)
					if err == nil && oursDecoded == goldenDecoded {
						entries = append(entries, goldenEntry{string(family), style, chars, result.CutSHA256, result.URISHA256, true})
						zlibDiffers++
						matched = true
						fmt.Fprintf(os.Stderr, "zlib-only difference in %s (%s)\n", path, kind)
						break
					}
				}
			}
			if !matched {
				unmatched++
				fmt.Fprintf(os.Stderr, "unmatched font in %s (%s)\n", path, kind)
			}
		}
	}
	sort.Slice(entries, func(i, j int) bool {
		a, b := entries[i], entries[j]
		if a.Family != b.Family {
			return a.Family < b.Family
		}
		if a.Style != b.Style {
			return a.Style < b.Style
		}
		return a.Chars < b.Chars
	})
	fmt.Fprintf(os.Stderr, "unique subset fonts exact=%d zlib-only-differences=%d unmatched=%d full-encodings-skipped=%d\n", len(entries)-zlibDiffers, zlibDiffers, unmatched, fullEncodings)
	enc := json.NewEncoder(os.Stdout)
	enc.SetEscapeHTML(false)
	for _, e := range entries {
		if err := enc.Encode(e); err != nil {
			return err
		}
	}
	if unmatched != 0 {
		return fmt.Errorf("%d golden fonts were not reproduced", unmatched)
	}
	return nil
}

// zlibVariants contrasts zlib.Writer Write+Close with the Write+Flush+Close
// sequence used by lib/font.woffCompression.compressTable.
func zlibVariants(paths []string) error {
	for _, path := range paths {
		data, err := os.ReadFile(path)
		if err != nil {
			return err
		}
		var closeOnly, flushClose bytes.Buffer
		w := zlib.NewWriter(&closeOnly)
		w.Write(data)
		w.Close()
		w = zlib.NewWriter(&flushClose)
		w.Write(data)
		w.Flush()
		w.Close()
		fmt.Printf("%s write_close=%d:%s write_flush_close=%d:%s\n", path, closeOnly.Len(), digest(closeOnly.Bytes()), flushClose.Len(), digest(flushClose.Bytes()))
	}
	return nil
}

func run() error {
	if len(os.Args) < 2 {
		return fmt.Errorf("usage: font-subset-oracle {cases|encodings|golden} ...")
	}
	switch os.Args[1] {
	case "cases":
		dumpDir := ""
		if len(os.Args) > 3 {
			dumpDir = os.Args[3]
		}
		return cases(os.Args[2], dumpDir)
	case "encodings":
		return encodings()
	case "golden":
		return golden(os.Args[2:])
	case "zlib":
		return zlibVariants(os.Args[2:])
	}
	return fmt.Errorf("unknown command %q", os.Args[1])
}

func main() {
	if err := run(); err != nil {
		fmt.Fprintln(os.Stderr, err)
		os.Exit(1)
	}
}
