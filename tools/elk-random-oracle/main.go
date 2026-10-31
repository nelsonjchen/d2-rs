// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// Development-only Go math/rand/v2 ChaCha8 oracle for
// crates/d2-rs-elk/src/spore/random.rs (recipe elk-spore-random). For 16 fixed
// seeds it prints 600 Uint64 words and the first 600 Float64-style values
// (63-bit mantissa / 2^63, excluding 1) as bit patterns. Standard library only.
package main

import (
	"encoding/json"
	"math"
	"math/rand/v2"
	"os"
)

func main() {
	rows := []map[string]any{}
	for k := 0; k < 16; k++ {
		var seed [32]byte
		values := []int{}
		for i := range seed {
			seed[i] = byte((k*31 + i*17) ^ (k * i))
			values = append(values, int(seed[i]))
		}
		rng := rand.NewChaCha8(seed)
		words := []uint64{}
		for i := 0; i < 600; i++ {
			words = append(words, rng.Uint64())
		}
		rng = rand.NewChaCha8(seed)
		bits := []uint64{}
		for len(bits) < 600 {
			f := float64(rng.Uint64()&0x7fffffffffffffff) / 9223372036854775808.0
			if f != 1 {
				bits = append(bits, math.Float64bits(f))
			}
		}
		rows = append(rows, map[string]any{"seed": values, "words": words, "float_bits": bits})
	}
	if err := json.NewEncoder(os.Stdout).Encode(rows); err != nil {
		panic(err)
	}
}
