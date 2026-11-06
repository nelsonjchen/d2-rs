// Command goflate-oracle is the Go 1.27.0 compress/zlib + compress/flate
// test oracle for crates/d2-rs-goflate. It reads a case-spec file and prints,
// for every case and level, the SHA-256 of Go's zlib output.
//
// Usage:
//
//	go run . -root <d2-rs checkout> -specs <specs.txt>             > expected.tsv
//	go run . -root <d2-rs checkout> -specs <specs.txt> -hex <name> -level <n>
//
// The fixtures were generated on darwin/arm64; an amd64 run produced
// identical expected.tsv bytes.
//
// Spec lines are "name<TAB>levels<TAB>mode<TAB>input". See generate for the
// input grammar; the Rust test (crates/d2-rs-goflate/tests/go_oracle.rs)
// implements the identical generators.
package main

import (
	"bufio"
	"bytes"
	"compress/zlib"
	"crypto/sha256"
	"encoding/binary"
	"encoding/hex"
	"flag"
	"fmt"
	"os"
	"path/filepath"
	"runtime"
	"strconv"
	"strings"
)

var allLevels = []int{-2, -1, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9}

type spec struct {
	name, levels, mode, input string
}

func main() {
	root := flag.String("root", ".", "d2-rs checkout root (for file inputs)")
	specsPath := flag.String("specs", "", "spec file")
	hexName := flag.String("hex", "", "dump hex output of this case instead")
	hexLevel := flag.Int("level", -1, "level for -hex")
	flag.Parse()

	// compress/flate was rewritten in Go 1.27 (and its output is explicitly
	// not covered by the Go 1 promise), so only the pinned toolchain is a
	// valid oracle. go.mod's "go 1.27.0" makes GOTOOLCHAIN=auto select it.
	if runtime.Version() != "go1.27.0" {
		check(fmt.Errorf("oracle must run on go1.27.0, not %s", runtime.Version()))
	}

	specs, err := readSpecs(*specsPath)
	check(err)
	out := bufio.NewWriter(os.Stdout)
	defer out.Flush()
	if *hexName == "" {
		fmt.Fprintln(out, "# name\tlevel\tinput_len\toutput_len\tsha256(go zlib output)")
	}
	for _, s := range specs {
		if *hexName != "" && s.name != *hexName {
			continue
		}
		data, err := generate(*root, s.input)
		check(err)
		levels, err := parseLevels(s.levels)
		check(err)
		for _, level := range levels {
			if *hexName != "" && level != *hexLevel {
				continue
			}
			z, err := compress(data, level, s.mode)
			check(err)
			if *hexName != "" {
				fmt.Fprintln(out, hex.EncodeToString(z))
				continue
			}
			sum := sha256.Sum256(z)
			fmt.Fprintf(out, "%s\t%d\t%d\t%d\t%s\n", s.name, level, len(data), len(z), hex.EncodeToString(sum[:]))
		}
	}
}

func check(err error) {
	if err != nil {
		fmt.Fprintln(os.Stderr, "goflate-oracle:", err)
		os.Exit(1)
	}
}

func readSpecs(path string) ([]spec, error) {
	b, err := os.ReadFile(path)
	if err != nil {
		return nil, err
	}
	var specs []spec
	for _, line := range strings.Split(string(b), "\n") {
		if line == "" || strings.HasPrefix(line, "#") {
			continue
		}
		f := strings.Split(line, "\t")
		if len(f) != 4 {
			return nil, fmt.Errorf("bad spec line %q", line)
		}
		specs = append(specs, spec{f[0], f[1], f[2], f[3]})
	}
	return specs, nil
}

func parseLevels(s string) ([]int, error) {
	if s == "all" {
		return allLevels, nil
	}
	var levels []int
	for _, f := range strings.Split(s, ",") {
		n, err := strconv.Atoi(f)
		if err != nil {
			return nil, err
		}
		levels = append(levels, n)
	}
	return levels, nil
}

// compress runs zlib.NewWriterLevel + Write(s) + [Flush] + Close.
// mode: "single" (one Write), "chunk:N" (Writes of N bytes),
// "flush:N" (Writes of N bytes, each followed by Flush).
func compress(data []byte, level int, mode string) ([]byte, error) {
	var buf bytes.Buffer
	w, err := zlib.NewWriterLevel(&buf, level)
	if err != nil {
		return nil, err
	}
	chunk, flush := len(data), false
	switch {
	case mode == "single":
	case strings.HasPrefix(mode, "chunk:"):
		chunk, err = strconv.Atoi(mode[len("chunk:"):])
	case strings.HasPrefix(mode, "flush:"):
		chunk, err = strconv.Atoi(mode[len("flush:"):])
		flush = true
	default:
		err = fmt.Errorf("bad mode %q", mode)
	}
	if err != nil {
		return nil, err
	}
	if len(data) == 0 {
		if _, err := w.Write(data); err != nil {
			return nil, err
		}
	}
	for off := 0; off < len(data); off += chunk {
		end := min(off+chunk, len(data))
		if _, err := w.Write(data[off:end]); err != nil {
			return nil, err
		}
		if flush {
			if err := w.Flush(); err != nil {
				return nil, err
			}
		}
	}
	if err := w.Close(); err != nil {
		return nil, err
	}
	return buf.Bytes(), nil
}

// splitmix64 is the deterministic generator shared with the Rust test.
type splitmix64 struct{ state uint64 }

func (r *splitmix64) next() uint64 {
	r.state += 0x9e3779b97f4a7c15
	z := r.state
	z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9
	z = (z ^ (z >> 27)) * 0x94d049bb133111eb
	return z ^ (z >> 31)
}

var words = strings.Fields(`the of and to in is that for it as with was on be by this are or
from at an which have not has but were all their can one more also its other
diagram shape connection label style container layout font glyph table`)

func atoi(s string) int {
	n, err := strconv.Atoi(s)
	check(err)
	return n
}

// generate builds an input from its spec:
//
//	hex:<hex>                  literal bytes
//	repeat:<hex>:<n>           pattern repeated, truncated to n bytes
//	ramp:<n>                   byte i%256
//	rand:<seed>:<n>            low bytes of splitmix64
//	alpha:<seed>:<k>:<n>       'a'+next%k
//	text:<seed>:<n>            words joined by ' ' or '\n' (next%8==0)
//	mix:<seed>:<n>             random / run / back-reference segments
//	file:<path>[:<off>:<len>]  repo file (or slice)
//	ttftable:<path>:<tag>      one table of a TrueType file
func generate(root, s string) ([]byte, error) {
	kind, rest, _ := strings.Cut(s, ":")
	switch kind {
	case "hex":
		return hex.DecodeString(rest)
	case "repeat":
		p, n, _ := strings.Cut(rest, ":")
		pat, err := hex.DecodeString(p)
		if err != nil {
			return nil, err
		}
		size := atoi(n)
		out := make([]byte, 0, size)
		for len(out) < size {
			out = append(out, pat[:min(len(pat), size-len(out))]...)
		}
		return out, nil
	case "ramp":
		n := atoi(rest)
		out := make([]byte, n)
		for i := range out {
			out[i] = byte(i)
		}
		return out, nil
	case "rand", "alpha", "text", "mix":
		f := strings.Split(rest, ":")
		r := &splitmix64{uint64(atoi(f[0]))}
		n := atoi(f[len(f)-1])
		out := make([]byte, 0, n)
		switch kind {
		case "rand":
			for len(out) < n {
				out = append(out, byte(r.next()))
			}
		case "alpha":
			k := uint64(atoi(f[1]))
			for len(out) < n {
				out = append(out, 'a'+byte(r.next()%k))
			}
		case "text":
			for len(out) < n {
				out = append(out, words[r.next()%uint64(len(words))]...)
				if r.next()%8 == 0 {
					out = append(out, '\n')
				} else {
					out = append(out, ' ')
				}
			}
			out = out[:n]
		case "mix":
			for len(out) < n {
				seg := int(1 + r.next()%2000)
				switch r.next() % 3 {
				case 0:
					for i := 0; i < seg; i++ {
						out = append(out, byte(r.next()))
					}
				case 1:
					b := byte(r.next())
					for i := 0; i < seg; i++ {
						out = append(out, b)
					}
				default:
					if len(out) == 0 {
						continue
					}
					dist := int(1 + r.next()%uint64(min(len(out), 40000)))
					for i := 0; i < seg; i++ {
						out = append(out, out[len(out)-dist])
					}
				}
			}
			out = out[:n]
		}
		return out, nil
	case "file":
		f := strings.Split(rest, ":")
		b, err := os.ReadFile(filepath.Join(root, f[0]))
		if err != nil {
			return nil, err
		}
		if len(f) == 3 {
			off, n := atoi(f[1]), atoi(f[2])
			b = b[off : off+n]
		}
		return b, nil
	case "ttftable":
		path, tag, _ := strings.Cut(rest, ":")
		b, err := os.ReadFile(filepath.Join(root, path))
		if err != nil {
			return nil, err
		}
		numTables := int(binary.BigEndian.Uint16(b[4:]))
		for i := 0; i < numTables; i++ {
			rec := b[12+16*i:]
			if string(rec[:4]) == tag {
				off := binary.BigEndian.Uint32(rec[8:])
				n := binary.BigEndian.Uint32(rec[12:])
				return b[off : off+n], nil
			}
		}
		return nil, fmt.Errorf("%s: no table %q", path, tag)
	}
	return nil, fmt.Errorf("bad input spec %q", s)
}
