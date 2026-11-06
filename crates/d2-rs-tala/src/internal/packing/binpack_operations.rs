// SPDX-License-Identifier: MPL-2.0
//! transaction.go::Ops, AddOp, Clear and the retained range header in Commit.
use super::*;
use std::cell::RefCell;
use std::rc::{Rc, Weak};

pub(in crate::internal) type PackingOperation<'op> = Rc<
    dyn for<'context> Fn(
            &mut ArenaGraph,
            &mut limits::WorkGuard<'context>,
        ) -> Result<(), limits::Error>
        + 'op,
>;
#[derive(Clone)]
struct Header<'op> {
    backing: Rc<RefCell<Vec<Option<PackingOperation<'op>>>>>,
    offset: usize,
    len: usize,
    cap: usize,
    nil: bool,
}
impl Header<'_> {
    fn empty() -> Self {
        Self {
            backing: Rc::new(RefCell::new(Vec::new())),
            offset: 0,
            len: 0,
            cap: 0,
            nil: false,
        }
    }
}
/// A handle to the actual Ops field. Range copies its header, not its elements;
/// Clear replaces that field without invalidating an already-running range.
#[derive(Clone)]
pub(in crate::internal) struct PackingOperations<'op>(Rc<RefCell<Header<'op>>>);
pub(in crate::internal) struct WeakPackingOperations<'op>(Weak<RefCell<Header<'op>>>);
impl<'op> WeakPackingOperations<'op> {
    pub(in crate::internal) fn upgrade(&self) -> PackingOperations<'op> {
        PackingOperations(self.0.upgrade().expect("live transaction Ops field"))
    }
}
impl<'op> PackingOperations<'op> {
    pub(in crate::internal) fn new() -> Self {
        Self(Rc::new(RefCell::new(Header::empty())))
    }
    pub(in crate::internal) fn downgrade(&self) -> WeakPackingOperations<'op> {
        WeakPackingOperations(Rc::downgrade(&self.0))
    }
    pub(in crate::internal) fn clear(&self) {
        *self.0.borrow_mut() = Header::empty();
    }
    /// Assigning nil replaces the public field, not any retained slice header.
    pub(in crate::internal) fn set_nil(&self) {
        *self.0.borrow_mut() = Header {
            nil: true,
            ..Header::empty()
        };
    }
    pub(in crate::internal) fn is_nil(&self) -> bool {
        self.0.borrow().nil
    }
    /// Go slice assignment copies a header while retaining its backing array.
    /// Ordinary Clone remains a handle to the same mutable transaction field.
    pub(in crate::internal) fn copy_header(&self) -> Self {
        Self(Rc::new(RefCell::new(self.0.borrow().clone())))
    }
    pub(in crate::internal) fn assign(&self, source: &Self) {
        let header = source.0.borrow().clone();
        *self.0.borrow_mut() = header;
    }
    /// Full slice expression, including extension into initialized spare slots.
    pub(in crate::internal) fn slice(&self, low: usize, high: usize, max: usize) -> Self {
        let header = self.0.borrow();
        assert!(
            low <= high && high <= max && max <= header.cap,
            "transaction Ops slice bounds out of range"
        );
        Self(Rc::new(RefCell::new(Header {
            backing: Rc::clone(&header.backing),
            offset: header.offset + low,
            len: high - low,
            cap: max - low,
            nil: header.nil,
        })))
    }
    /// transaction.go::cloneSlice copies the visible function values into
    /// independent len-sized backing. Captured closure state stays shared.
    pub(in crate::internal) fn copy_visible(&self) -> Self {
        let header = self.0.borrow();
        let values = header.backing.borrow()[header.offset..header.offset + header.len].to_vec();
        Self(Rc::new(RefCell::new(Header {
            backing: Rc::new(RefCell::new(values)),
            offset: 0,
            len: header.len,
            cap: header.len,
            nil: false,
        })))
    }
    pub(in crate::internal) fn add(&self, operation: Option<PackingOperation<'op>>) {
        let mut header = self.0.borrow_mut();
        let next_len = header
            .len
            .checked_add(1)
            .expect("transaction Ops length overflow");
        if next_len > header.cap {
            let cap = container_slices::go127_pointer_growth_capacity(header.cap, next_len)
                .expect("transaction Ops capacity overflow");
            let mut values =
                header.backing.borrow()[header.offset..header.offset + header.len].to_vec();
            values.resize_with(cap, || None);
            header.backing = Rc::new(RefCell::new(values));
            header.offset = 0;
            header.cap = cap;
        }
        header.backing.borrow_mut()[header.offset + header.len] = operation;
        header.len = next_len;
        header.nil = false;
    }
    pub(in crate::internal) fn replace(
        &self,
        index: usize,
        operation: Option<PackingOperation<'op>>,
    ) {
        let header = self.0.borrow();
        assert!(index < header.len, "transaction Ops index out of range");
        header.backing.borrow_mut()[header.offset + index] = operation;
    }
    pub(in crate::internal) fn range(&self) -> OperationRange<'op> {
        OperationRange {
            header: self.0.borrow().clone(),
            index: 0,
        }
    }
    #[cfg(test)]
    pub(in crate::internal) fn shares_slot(
        &self,
        index: usize,
        other: &Self,
        other_index: usize,
    ) -> bool {
        let header = self.0.borrow();
        let other = other.0.borrow();
        assert!(index < header.cap && other_index < other.cap);
        Rc::ptr_eq(&header.backing, &other.backing)
            && header.offset + index == other.offset + other_index
    }
    #[cfg(test)]
    pub(in crate::internal) fn dimensions(&self) -> (usize, usize, Vec<bool>) {
        let header = self.0.borrow();
        (
            header.len,
            header.cap,
            header.backing.borrow()[header.offset..header.offset + header.len]
                .iter()
                .map(Option::is_none)
                .collect(),
        )
    }
}
pub(in crate::internal) struct OperationRange<'op> {
    header: Header<'op>,
    index: usize,
}
impl<'op> Iterator for OperationRange<'op> {
    type Item = Option<PackingOperation<'op>>;
    fn next(&mut self) -> Option<Self::Item> {
        if self.index == self.header.len {
            return None;
        }
        // Go loads the function value before Step/ctx.Err. Release the backing
        // borrow before invoking callbacks, which may replace a future slot.
        let operation = self.header.backing.borrow()[self.header.offset + self.index].clone();
        self.index += 1;
        Some(operation)
    }
}
