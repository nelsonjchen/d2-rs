// Copyright 2008 Google Inc.
// Copyright (c) 2026 The elk-go Authors.
// Rust translation and modifications: Copyright 2026 Nelson Chen.
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
// https://www.apache.org/licenses/LICENSE-2.0
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
// SPDX-License-Identifier: Apache-2.0

//! GWT offer/poll heap semantics, including equal-value sift-down.
use std::cmp::Ordering;

/// GWT bulk insertion appends all items then recursively merges subheaps.
/// Repeated offer changes traversal order for equal priorities.
pub(crate) fn add_all<T: Copy>(heap: &mut Vec<T>, items: &[T], compare: impl Fn(T, T) -> Ordering) {
    fn merge<T: Copy>(heap: &mut [T], mut node: usize, compare: &impl Fn(T, T) -> Ordering) {
        let value = heap[node];
        while node * 2 + 1 < heap.len() {
            let left = node * 2 + 1;
            let right = left + 1;
            let smallest =
                if right < heap.len() && compare(heap[right], heap[left]) == Ordering::Less {
                    right
                } else {
                    left
                };
            if compare(value, heap[smallest]) == Ordering::Less {
                break;
            }
            heap[node] = heap[smallest];
            node = smallest;
        }
        heap[node] = value;
    }
    fn make<T: Copy>(heap: &mut [T], node: usize, compare: &impl Fn(T, T) -> Ordering) {
        if node * 2 + 1 >= heap.len() {
            return;
        }
        make(heap, node * 2 + 1, compare);
        if node * 2 + 2 < heap.len() {
            make(heap, node * 2 + 2, compare);
        }
        merge(heap, node, compare);
    }
    if !items.is_empty() {
        heap.extend_from_slice(items);
        make(heap, 0, &compare);
    }
}

pub(crate) fn offer<T: Copy>(heap: &mut Vec<T>, item: T, compare: impl Fn(T, T) -> Ordering) {
    let mut node = heap.len();
    heap.push(item);
    while node > 0 {
        let child = node;
        node = (node - 1) / 2;
        if compare(heap[node], item) != Ordering::Greater {
            heap[child] = item;
            return;
        }
        heap[child] = heap[node];
    }
    heap[node] = item;
}

pub(crate) fn poll<T: Copy>(heap: &mut Vec<T>, compare: impl Fn(T, T) -> Ordering) -> Option<T> {
    let value = *heap.first()?;
    let last = heap.pop().expect("nonempty heap");
    if !heap.is_empty() {
        let mut node = 0;
        while node * 2 + 1 < heap.len() {
            let left = node * 2 + 1;
            let right = left + 1;
            let smallest =
                if right < heap.len() && compare(heap[right], heap[left]) == Ordering::Less {
                    right
                } else {
                    left
                };
            if compare(last, heap[smallest]) == Ordering::Less {
                break;
            }
            heap[node] = heap[smallest];
            node = smallest;
        }
        heap[node] = last;
    }
    Some(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_priorities_sift_down_on_poll_but_stay_on_offer() {
        // Source GWT queue chooses the left equal child and moves the last
        // value below it. A stable queue or Rust BinaryHeap changes this order.
        let mut heap = Vec::new();
        for value in 0..4 {
            offer(&mut heap, value, |_, _| Ordering::Equal);
        }
        assert_eq!(heap, [0, 1, 2, 3]);
        let output =
            std::iter::from_fn(|| poll(&mut heap, |_, _| Ordering::Equal)).collect::<Vec<_>>();
        assert_eq!(output, [0, 1, 3, 2]);
    }
}
