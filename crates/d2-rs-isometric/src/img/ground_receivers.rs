// SPDX-License-Identifier: MPL-2.0
//! ground_receivers.go: independent root containment domains and receivers.
use super::mesh::Triangle;
use crate::scene::Board;
use std::collections::HashMap;
pub struct HierarchyShadowSpan {
    pub first: usize,
    pub last: usize,
    pub board: String,
}
pub fn hierarchy_shadow_receivers(
    ts: &mut [Triangle],
    boards: &[Board],
    spans: &[HierarchyShadowSpan],
) {
    let index: HashMap<_, _> = boards
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id.clone(), i))
        .collect();
    let mut domains = HashMap::new();
    let mut state = vec![0; boards.len()];
    fn find(
        i: usize,
        boards: &[Board],
        index: &HashMap<String, usize>,
        domains: &mut HashMap<String, String>,
        state: &mut [u8],
    ) -> String {
        let b = &boards[i];
        if state[i] != 0 {
            return domains.get(&b.id).cloned().unwrap_or_else(|| b.id.clone());
        }
        state[i] = 1;
        let domain = if b.kind != "platform" {
            if let Some(p) = index.get(&b.parent_id) {
                find(*p, boards, index, domains, state)
            } else {
                b.id.clone()
            }
        } else {
            b.id.clone()
        };
        domains.insert(b.id.clone(), domain.clone());
        state[i] = 2;
        domain
    }
    for i in 0..boards.len() {
        find(i, boards, &index, &mut domains, &mut state);
    }
    let mut receivers: HashMap<String, f64> = HashMap::new();
    for span in spans {
        let Some(domain) = domains.get(&span.board) else {
            continue;
        };
        let r = receivers.entry(domain.clone()).or_insert(0.0);
        for t in &ts[span.first..span.last] {
            for v in t.v {
                *r = d2_rs_scene::go_min(*r, v.position.y);
            }
        }
    }
    for r in receivers.values_mut() {
        *r += 0.08;
    }
    for span in spans {
        let Some(r) = domains.get(&span.board).and_then(|d| receivers.get(d)) else {
            continue;
        };
        for t in &mut ts[span.first..span.last] {
            t.shadow_ground = Some(*r);
        }
    }
}
