// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

// grouping/sequences.go:310–375. All candidates are drawn before building
// groups. Remembered IDs select generated slots, then remaining groups retain
// their container order. Candidate collisions advance the ID, not the RNG.
fn sequence_id_slots(
    groups: Vec<Vec<NodeId>>,
    generated: &[i64],
    remembered: &BTreeMap<NodeId, i64>,
    occupied: &mut BTreeSet<i64>,
) -> Vec<(Vec<NodeId>, i64)> {
    assert_eq!(groups.len(), generated.len());
    let slots = generated.iter().enumerate().map(|(i, &id)| (id, i)).collect::<BTreeMap<_, _>>();
    let mut ordered = vec![None; groups.len()];
    let mut used = vec![false; groups.len()];
    for (i, members) in groups.iter().enumerate() {
        if let Some(slot) = remembered.get(&members[0]).and_then(|id| slots.get(id))
            && ordered[*slot].is_none()
        {
            ordered[*slot] = Some(i);
            used[i] = true;
        }
    }
    let mut next = 0;
    for slot in &mut ordered {
        if slot.is_none() {
            while used[next] { next += 1; }
            *slot = Some(next);
            used[next] = true;
        }
    }
    let mut groups = groups.into_iter().map(Some).collect::<Vec<_>>();
    ordered.into_iter().enumerate().map(|(slot, index)| {
        let members = groups[index.unwrap()].take().unwrap();
        let id = if let Some(&id) = remembered.get(&members[0]) {
            id
        } else {
            let id = next_available_sequence_id(generated[slot], occupied);
            occupied.insert(id);
            id
        };
        (members, id)
    }).collect()
}

fn next_available_sequence_id(mut candidate: i64, occupied: &BTreeSet<i64>) -> i64 {
    while occupied.contains(&candidate) {
        candidate = if candidate == i64::MAX { 0 } else { candidate + 1 };
    }
    candidate
}

#[cfg(test)]
mod sequence_id_tests {
    use super::*;

    #[test]
    fn remembered_ids_select_generated_slots_before_fresh_groups() {
        let groups = vec![vec![NodeId(0),NodeId(1)], vec![NodeId(2),NodeId(3)], vec![NodeId(4),NodeId(5)]];
        let remembered = BTreeMap::from([(NodeId(2), 10), (NodeId(4), 20)]);
        let mut occupied = BTreeSet::from([10,20]);
        let result = sequence_id_slots(groups, &[10,20,30], &remembered, &mut occupied);
        assert_eq!(result.iter().map(|(m,id)|(m[0],*id)).collect::<Vec<_>>(), vec![(NodeId(2),10),(NodeId(4),20),(NodeId(0),30)]);
        assert!(occupied.contains(&30));
    }

    #[test]
    fn duplicate_candidates_keep_last_generated_slot_and_reserve_fresh_ids() {
        let groups = vec![vec![NodeId(0)],vec![NodeId(1)],vec![NodeId(2)]];
        let mut occupied = BTreeSet::from([9]);
        let result = sequence_id_slots(groups, &[9,9,9], &BTreeMap::from([(NodeId(1),9)]), &mut occupied);
        assert_eq!(result, vec![(vec![NodeId(0)],10),(vec![NodeId(2)],11),(vec![NodeId(1)],9)]);
    }

    #[test]
    fn collision_search_wraps_at_max_and_skips_reserved_zero() {
        assert_eq!(next_available_sequence_id(i64::MAX,&BTreeSet::from([i64::MAX,0,1])),2);
        assert_eq!(next_available_sequence_id(i64::MAX,&BTreeSet::new()),i64::MAX);
        assert_eq!(next_available_sequence_id(-1,&BTreeSet::from([-1,0])),1);
    }
}
