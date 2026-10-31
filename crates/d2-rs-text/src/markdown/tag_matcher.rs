// SPDX-License-Identifier: BSD-3-Clause
// Copyright 2009 The Go Authors. All rights reserved.
// Rust translation Copyright 2026 Nelson Chen.
// Go 1.27 regexp/exec.go machine.match/step/add, scoped to the pinned anchored
// Goldmark tag programs. Only the overall match is exposed; captures are unused.
pub(super) struct Inst {
    pub op: u8,
    pub out: usize,
    pub arg: usize,
    pub runes: &'static [u32],
}
struct Queue {
    seen: Vec<bool>,
    count: usize,
    threads: Vec<usize>,
}
impl Queue {
    fn new(n: usize) -> Self {
        Self {
            seen: vec![false; n],
            count: 0,
            threads: Vec::new(),
        }
    }
    fn clear(&mut self) {
        self.seen.fill(false);
        self.count = 0;
        self.threads.clear();
    }
    fn add(&mut self, program: &[Inst], pc: usize, pos: usize) {
        if pc == 0 || self.seen[pc] {
            return;
        }
        self.seen[pc] = true;
        self.count += 1;
        let inst = &program[pc];
        match inst.op {
            0 => {
                self.add(program, inst.out, pos);
                self.add(program, inst.arg, pos);
            }
            2 => self.add(program, inst.out, pos),
            3 => {
                assert_eq!(inst.arg, 4);
                if pos == 0 {
                    self.add(program, inst.out, pos);
                }
            }
            5 => {}
            4 | 7 | 8 => self.threads.push(pc),
            _ => panic!("unqualified tag instruction"),
        }
    }
}
/// Execute the verified tag program, preserving Go's two-rune lookahead and
/// priority queues. The callback is the reader's ReadRune, returning None at EOF.
pub fn match_tag_reader(
    closing: bool,
    mut read: impl FnMut() -> Option<(char, usize)>,
) -> Option<usize> {
    let program = if closing {
        super::tag_program::CLOSE
    } else {
        super::tag_program::OPEN
    };
    let mut run = Queue::new(program.len());
    let mut next = Queue::new(program.len());
    let mut current = read();
    let mut lookahead = if current.is_some() { read() } else { None };
    let mut pos = 0;
    let mut matched = None;
    loop {
        if run.count == 0 && (pos != 0 || matched.is_some()) {
            break;
        }
        if matched.is_none() {
            run.add(program, 1, pos);
        }
        let width = current.map_or(0, |(_, n)| n);
        for &pc in &run.threads {
            let inst = &program[pc];
            if inst.op == 4 {
                matched = Some(pos);
                break;
            }
            let accepts = current.is_some_and(|(r, _)| {
                let r = r as u32;
                if inst.op == 8 {
                    r == inst.runes[0]
                } else {
                    inst.runes
                        .as_chunks::<2>()
                        .0
                        .iter()
                        .any(|v| v[0] <= r && r <= v[1])
                }
            });
            if accepts {
                next.add(program, inst.out, pos + width);
            }
        }
        run.clear();
        if width == 0 {
            break;
        }
        pos += width;
        current = lookahead;
        if current.is_some() {
            lookahead = read();
        }
        std::mem::swap(&mut run, &mut next);
    }
    matched
}
