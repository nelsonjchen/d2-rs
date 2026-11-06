// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.
//! D2 v0.9.0 adapter.go identity allocation. Cancellation integration is pending.
use std::collections::{BTreeMap, BTreeSet};

pub fn allocate(identities: &[&[u8]]) -> Result<Vec<i64>, usize> {
    let mut seen = BTreeSet::new();
    let mut buckets = BTreeMap::<u32, usize>::new();
    let mut hashed = Vec::with_capacity(identities.len());
    for (i, identity) in identities.iter().enumerate() {
        if !seen.insert(*identity) {
            return Err(i);
        }
        let hash = identity.iter().fold(2_166_136_261u32, |h, b| {
            (h ^ u32::from(*b)).wrapping_mul(16_777_619)
        });
        *buckets.entry(hash).or_default() += 1;
        hashed.push(hash);
    }
    let mut allocated = vec![0; identities.len()];
    let mut ambiguous = Vec::new();
    for (i, hash) in hashed.into_iter().enumerate() {
        if hash != 0 && buckets[&hash] == 1 {
            allocated[i] = i64::from(hash);
        } else {
            ambiguous.push((hash, identities[i], i));
        }
    }
    ambiguous.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
    for (offset, (_, _, index)) in ambiguous.into_iter().enumerate() {
        allocated[index] = (1i64 << 32) + offset as i64;
    }
    Ok(allocated)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn upstream_collision_and_zero_hash_vectors() {
        let first = allocate(&[b"costarring", b"liquid", b"a"]).unwrap();
        assert_eq!(first, [(1i64 << 32), (1i64 << 32) + 1, 3_826_002_220]);
        let second = allocate(&[b"a", b"liquid", b"costarring"]).unwrap();
        assert_eq!(second, [first[2], first[1], first[0]]);
        assert_eq!(
            allocate(&[&[0xcc, 0x24, 0x31, 0xc4]]).unwrap(),
            [1i64 << 32]
        );
        assert_eq!(allocate(&[b"duplicate", b"duplicate"]), Err(1));
    }
}
