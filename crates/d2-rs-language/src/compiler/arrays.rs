// SPDX-License-Identifier: MPL-2.0
//! Observable Go []Value append aliasing during substitution (64-bit Go 1.27).
//! Growth/size classes follow Go runtime slice.go, msize.go and sizeclasses.go;
//! Go Authors' BSD notice is retained in the distributed third-party notices.
use super::Compiler;
use d2_rs_core::{ast::Shared, ir::*};
#[derive(Clone)]
pub(super) struct ArrayView {
    buffer: Shared<Vec<NodeId>>,
    len: usize,
    capacity: usize,
}
impl ArrayView {
    pub(super) fn len(&self) -> usize {
        self.len
    }
    pub(super) fn get(&self, index: usize) -> NodeId {
        self.buffer.read()[index]
    }
    fn append(mut self, values: &[NodeId]) -> Self {
        let new_len = self.len + values.len();
        if new_len > self.capacity {
            self.capacity = grow_capacity(self.capacity, new_len);
            let copied = self.buffer.read()[..self.len].to_vec();
            self.buffer = Shared::new(copied);
        }
        {
            let mut buffer = self.buffer.write();
            for (i, value) in values.iter().enumerate() {
                if self.len + i < buffer.len() {
                    buffer[self.len + i] = *value
                } else {
                    buffer.push(*value)
                }
            }
        }
        self.len = new_len;
        self
    }
}
// Value is a two-word, pointer-containing Go interface. Heap header and size
// class rounding affect whether nested append reuses the original backing data.
fn grow_capacity(old: usize, new_len: usize) -> usize {
    let mut cap = old;
    if new_len > old * 2 {
        cap = new_len
    } else if old < 256 {
        cap = old * 2
    } else {
        while cap < new_len {
            cap += (cap + 3 * 256) >> 2;
        }
    }
    const CLASSES: &[usize] = &[
        0, 8, 16, 24, 32, 48, 64, 80, 96, 112, 128, 144, 160, 176, 192, 208, 224, 240, 256, 288,
        320, 352, 384, 416, 448, 480, 512, 576, 640, 704, 768, 896, 1024, 1152, 1280, 1408, 1536,
        1792, 2048, 2304, 2688, 3072, 3200, 3456, 4096, 4864, 5376, 6144, 6528, 6784, 6912, 8192,
        9472, 9728, 10240, 10880, 12288, 13568, 14336, 16384, 18432, 19072, 20480, 21760, 24576,
        27264, 28672, 32768,
    ];
    let bytes = cap * 16;
    let rounded = if bytes <= 32768 - 8 {
        let header = if bytes > 512 { 8 } else { 0 };
        CLASSES
            .iter()
            .copied()
            .find(|size| *size >= bytes + header)
            .unwrap()
            - header
    } else {
        bytes.div_ceil(8192) * 8192
    };
    rounded / 16
}
impl Compiler {
    pub(super) fn append_compiled_array(&mut self, array: NodeId, values: &[NodeId]) {
        if values.is_empty() {
            return;
        }
        let view = self.array_view(array).append(values);
        let Node::Array(a) = self.arena.node_mut(array) else {
            unreachable!()
        };
        a.values = Some(view.buffer.read()[..view.len].to_vec());
        self.array_views.insert(array, view);
    }
    pub(super) fn register_array(&mut self, array: NodeId) {
        let Node::Array(a) = self.arena.node(array) else {
            unreachable!()
        };
        let values = a.values.clone().unwrap_or_default();
        let mut capacity = 0;
        for len in 1..=values.len() {
            if len > capacity {
                capacity = grow_capacity(capacity, len);
            }
        }
        self.array_views.insert(
            array,
            ArrayView {
                len: values.len(),
                buffer: Shared::new(values),
                capacity,
            },
        );
    }
    pub(super) fn array_view(&mut self, array: NodeId) -> ArrayView {
        self.array_views
            .entry(array)
            .or_insert_with(|| {
                let Node::Array(a) = self.arena.node(array) else {
                    unreachable!()
                };
                let values = a.values.clone().unwrap_or_default();
                // Unregistered arrays come from IR Copy: append([]Value(nil), old...).
                ArrayView {
                    len: values.len(),
                    capacity: grow_capacity(0, values.len()),
                    buffer: Shared::new(values),
                }
            })
            .clone()
    }
    pub(super) fn spread_array(&mut self, array: NodeId, node: NodeId, resolved: NodeId) {
        let original = self.array_view(array);
        let insert = self.array_view(resolved);
        let values = insert.buffer.read()[..insert.len].to_vec();
        let Some(index) = original.buffer.read()[..original.len]
            .iter()
            .position(|v| *v == node)
        else {
            return;
        };
        let mut prefix = original.clone();
        prefix.len = index;
        let prefix = prefix.append(&values);
        // Read this tail AFTER the inner append, as Go's nested expression does.
        let tail = original.buffer.read()[index + 1..original.len].to_vec();
        let result = prefix.append(&tail);
        let Node::Array(a) = self.arena.node_mut(array) else {
            unreachable!()
        };
        a.values = Some(result.buffer.read()[..result.len].to_vec());
        self.array_views.insert(array, result);
    }
}

#[cfg(test)]
mod tests {
    #[test]
    #[cfg(feature = "oracle-fixtures")]
    fn go_interface_slice_capacities_match_pinned_runtime() {
        let cases: Vec<(usize, usize, usize)> = serde_json::from_str(
            d2_rs_testdata::oracle_fixture!("tests/fixtures/array-capacities.json"),
        )
        .unwrap();
        for (old, new, expected) in cases {
            assert_eq!(
                super::grow_capacity(old, new),
                expected,
                "old={old}, new={new}"
            );
        }
    }
}
