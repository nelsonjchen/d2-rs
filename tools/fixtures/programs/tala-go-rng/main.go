// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
// Development reference only; never shipped as part of d2-rs.

// Recipe `tala-go-rng-int63n` (tools/fixtures/tala-oracle.toml): Go
// math/rand Int63n on a fresh seeded source at the edges of its 31-bit and
// 63-bit rejection loops, counting source draws, followed by one Float64.
// Read by crates/d2-rs-tala/src/internal/runtime/go_rng.rs (GoRng), which
// mirrors the math/rand source TALA seeds per layout.
package main

import (
	"encoding/json"
	"fmt"
	"math"
	"math/rand"
	"os"
)

// counting wraps the seeded source and counts every value it yields.
type counting struct {
	src   rand.Source64
	draws uint64
}

func (c *counting) Int63() int64    { c.draws++; return c.src.Int63() }
func (c *counting) Uint64() uint64  { c.draws++; return c.src.Uint64() }
func (c *counting) Seed(seed int64) { c.src.Seed(seed) }

type record struct {
	Seed            int64  `json:"seed"`
	Bound           int64  `json:"bound"`
	Result          int64  `json:"result"`
	Draws           uint64 `json:"draws"`
	NextFloatBits   string `json:"next_float_bits"`
	DrawsAfterFloat uint64 `json:"draws_after_float"`
}

func main() {
	seeds := []int64{0, 1, 2, 17, 42, math.MaxInt32, -1, math.MinInt64}
	bounds := []int64{1, 2, math.MaxInt32, math.MaxInt32 + 1, math.MaxInt32 + 2, 1 << 62, 1<<62 + 3, math.MaxInt64}
	records := []record{}
	for _, seed := range seeds {
		for _, bound := range bounds {
			src := &counting{src: rand.NewSource(seed).(rand.Source64)}
			r := rand.New(src)
			result := r.Int63n(bound)
			draws := src.draws
			bits := fmt.Sprintf("%016x", math.Float64bits(r.Float64()))
			records = append(records, record{seed, bound, result, draws, bits, src.draws})
		}
	}
	data, err := json.Marshal(records)
	if err != nil {
		panic(err)
	}
	if _, err := os.Stdout.Write(append(data, '\n')); err != nil {
		panic(err)
	}
}
