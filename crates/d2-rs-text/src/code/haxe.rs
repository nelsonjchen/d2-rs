// SPDX-License-Identifier: MIT AND BSD-3-Clause
// Chroma v2.27.0 lexers/haxe.go haxePreProcMutator.
// Copyright 2017 Alec Thomas. Rust translation Copyright 2026 Nelson Chen.
use super::xml::StateStack;
use std::{cell::RefCell, rc::Rc};

#[derive(Clone)]
struct Slice<'a> {
    slots: Rc<RefCell<Vec<&'a str>>>,
    length: usize,
}
impl<'a> Slice<'a> {
    fn new(state: &'a str) -> Self {
        Self {
            slots: Rc::new(RefCell::new(vec![state])),
            length: 1,
        }
    }
    fn push(&mut self, state: &'a str) {
        let capacity = self.slots.borrow().len();
        if self.length == capacity {
            let mut slots = self.slots.borrow()[..self.length].to_vec();
            slots.resize(grow_capacity(capacity), "");
            self.slots = Rc::new(RefCell::new(slots));
        }
        self.slots.borrow_mut()[self.length] = state;
        self.length += 1;
    }
}

pub(super) struct Stack<'a> {
    active: Slice<'a>,
    saved: Vec<Slice<'a>>,
}
impl<'a> Stack<'a> {
    pub(super) fn new(state: &'a str) -> Self {
        Self {
            active: Slice::new(state),
            saved: Vec::new(),
        }
    }
}
impl<'a> StateStack<'a> for Stack<'a> {
    fn len(&self) -> usize {
        self.active.length
    }
    fn last(&self) -> Option<&'a str> {
        self.active
            .length
            .checked_sub(1)
            .map(|i| self.active.slots.borrow()[i])
    }
    fn push(&mut self, state: &'a str) {
        self.active.push(state);
    }
    fn truncate(&mut self, length: usize) {
        self.active.length = self.active.length.min(length);
    }
    fn reset(&mut self, state: &'a str) {
        self.active = Slice::new(state);
    }
    fn preproc(&mut self, directive: &str) {
        // Go saves slice headers, not copies of their elements. A branch that
        // pops then pushes can overwrite slots visible to a saved header.
        match directive {
            "if" => self.saved.push(self.active.clone()),
            "else" | "elseif" => {
                if let Some(saved) = self.saved.last() {
                    self.active = saved.clone();
                }
            }
            "end" => {
                self.saved.pop();
            }
            _ => {}
        }
        if matches!(directive, "if" | "elseif") {
            self.active.push("preproc-expr");
        }
        if directive == "error" {
            self.active.push("preproc-error");
        }
    }
}

// Go 1.27 runtime/slice.go nextslicecap and runtime/msize.go roundupsize.
// Copyright 2009 The Go Authors. All rights reserved. See root notices.
// Stack elements are scanned Go strings (16 bytes on amd64, arm64 and Go
// js/wasm). Capacity determines when saved headers stop sharing their slots.
fn grow_capacity(old: usize) -> usize {
    let next = if old < 256 {
        old * 2
    } else {
        old + ((old + 768) >> 2)
    };
    crate::go_runtime::roundupsize(next * 16, false) / 16
}
