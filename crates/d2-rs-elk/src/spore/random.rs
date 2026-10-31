// Copyright 2023 The Go Authors. All rights reserved.
// BSD-3-Clause; see ../GO-MATH-LICENSE.txt and GO-CHACHA8-NOTICE.txt.
// Go 1.27 internal/chacha8rand/{chacha8,chacha8_generic}.go and math/rand/rand.go.
use std::cell::RefCell;
pub(super) struct ChaCha8 {
    key: [u64; 4],
    buffer: [u64; 32],
    index: usize,
    length: usize,
    counter: u32,
}
fn quarter(x: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
    x[a] = x[a].wrapping_add(x[b]);
    x[d] = (x[d] ^ x[a]).rotate_left(16);
    x[c] = x[c].wrapping_add(x[d]);
    x[b] = (x[b] ^ x[c]).rotate_left(12);
    x[a] = x[a].wrapping_add(x[b]);
    x[d] = (x[d] ^ x[a]).rotate_left(8);
    x[c] = x[c].wrapping_add(x[d]);
    x[b] = (x[b] ^ x[c]).rotate_left(7);
}
fn block(key: [u64; 4], counter: u32) -> [u64; 32] {
    let mut words = [0u32; 64];
    for lane in 0..4 {
        let mut x = [0u32; 16];
        x[..4].copy_from_slice(&[0x61707865, 0x3320646e, 0x79622d32, 0x6b206574]);
        for i in 0..4 {
            x[4 + i * 2] = key[i] as u32;
            x[5 + i * 2] = (key[i] >> 32) as u32;
        }
        x[12] = counter + lane as u32;
        let original = x;
        for _ in 0..4 {
            quarter(&mut x, 0, 4, 8, 12);
            quarter(&mut x, 1, 5, 9, 13);
            quarter(&mut x, 2, 6, 10, 14);
            quarter(&mut x, 3, 7, 11, 15);
            quarter(&mut x, 0, 5, 10, 15);
            quarter(&mut x, 1, 6, 11, 12);
            quarter(&mut x, 2, 7, 8, 13);
            quarter(&mut x, 3, 4, 9, 14);
        }
        for i in 4..12 {
            x[i] = x[i].wrapping_add(original[i]);
        }
        for i in 0..16 {
            words[i * 4 + lane] = x[i];
        }
    }
    let mut out = [0; 32];
    for i in 0..32 {
        out[i] = u64::from(words[2 * i]) | u64::from(words[2 * i + 1]) << 32;
    }
    out
}
impl ChaCha8 {
    pub fn new(seed: [u8; 32]) -> Self {
        let key =
            std::array::from_fn(|i| u64::from_le_bytes(seed[8 * i..8 * i + 8].try_into().unwrap()));
        Self {
            key,
            buffer: block(key, 0),
            index: 0,
            length: 32,
            counter: 0,
        }
    }
    pub fn next(&mut self) -> u64 {
        if self.index >= self.length {
            self.counter += 4;
            if self.counter == 16 {
                self.key.copy_from_slice(&self.buffer[28..]);
                self.counter = 0;
            }
            self.buffer = block(self.key, self.counter);
            self.index = 0;
            self.length = if self.counter == 12 { 28 } else { 32 };
        }
        let value = self.buffer[self.index];
        self.index += 1;
        value
    }
    pub fn float64(&mut self) -> f64 {
        loop {
            let f = (self.next() & 0x7fffffffffffffff) as f64 / 9223372036854775808.0;
            if f != 1.0 {
                return f;
            }
        }
    }
}
fn seed() -> [u8; 32] {
    let mut bytes = [0; 32];
    #[cfg(unix)]
    {
        use std::io::Read;
        if std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut bytes))
            .is_ok()
        {
            return bytes;
        }
    }
    // Non-Unix entropy/platform behavior remains separately unqualified. Keep the
    // same ChaCha8 state/Float64 contract rather than substituting another PRNG.
    use std::hash::{BuildHasher, Hasher};
    for chunk in bytes.chunks_mut(8) {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u128(
            web_time::SystemTime::now()
                .duration_since(web_time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
        );
        chunk.copy_from_slice(&h.finish().to_le_bytes());
    }
    bytes
}
thread_local! {static RUNTIME:RefCell<ChaCha8>=RefCell::new(ChaCha8::new(seed()));}
pub(super) fn float64() -> f64 {
    RUNTIME.with(|r| r.borrow_mut().float64())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_go_chacha8_refill_rekey_and_float_bits() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "src/fixtures/original_spore_random.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let seed: Vec<u8> = serde_json::from_value(row["seed"].clone()).unwrap();
            let mut rng = ChaCha8::new(seed.try_into().unwrap());
            for value in row["words"].as_array().unwrap() {
                assert_eq!(rng.next(), value.as_u64().unwrap())
            }
            let mut rng = ChaCha8::new(
                serde_json::from_value::<Vec<u8>>(row["seed"].clone())
                    .unwrap()
                    .try_into()
                    .unwrap(),
            );
            for bits in row["float_bits"].as_array().unwrap() {
                assert_eq!(rng.float64().to_bits(), bits.as_u64().unwrap());
            }
        }
    }
}
