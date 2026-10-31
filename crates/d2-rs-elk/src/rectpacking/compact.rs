// Copyright (c) 2018, 2020 Kiel University and others.
// Copyright (c) 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// elk-go v0.2.0/internal/rectpacking/second_iteration.go.
use super::{Profile, node_flag};
use crate::ElkNode;

#[derive(Default)]
struct Line {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    nodes: Vec<usize>,
}
struct Block {
    children: Vec<usize>,
    lines: Vec<Line>,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    min_width: f64,
    min_height: f64,
    max_height: f64,
    smallest_width: f64,
    smallest_height: f64,
    average_height: f64,
    parent: usize,
    stack: Option<usize>,
    fixed: bool,
    position_fixed: bool,
}
#[derive(Default)]
struct Row {
    y: f64,
    width: f64,
    height: f64,
    blocks: Vec<usize>,
    stacks: Vec<usize>,
}
#[derive(Default)]
struct Stack {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    blocks: Vec<usize>,
}
struct Arena<'a> {
    nodes: &'a mut [ElkNode],
    blocks: Vec<Block>,
    rows: Vec<Row>,
    stacks: Vec<Stack>,
    spacing: f64,
    reevaluate: bool,
}
impl Line {
    fn add(&mut self, nodes: &mut [ElkNode], id: usize, spacing: f64) {
        let node = &mut nodes[id];
        node.x = self.x + self.width + if self.nodes.is_empty() { 0.0 } else { spacing };
        node.y = self.y;
        self.height = self.height.max(node.height);
        if !self.nodes.is_empty() {
            self.width += spacing;
        }
        self.width += node.width;
        self.nodes.push(id);
    }
    fn update(&mut self, nodes: &mut [ElkNode], spacing: f64) {
        let (mut width, mut height) = (0.0_f64, 0.0_f64);
        for &id in &self.nodes {
            let node = &mut nodes[id];
            node.x = self.x + width;
            node.y = self.y;
            width += node.width + spacing;
            height = height.max(node.height + spacing);
        }
        self.width = width - spacing;
        self.height = height - spacing;
    }
}
impl Arena<'_> {
    fn row(&mut self, y: f64) -> usize {
        let id = self.rows.len();
        self.rows.push(Row {
            y,
            ..Row::default()
        });
        id
    }
    fn block(&mut self, x: f64, y: f64, parent: usize) -> usize {
        let id = self.blocks.len();
        self.blocks.push(Block {
            children: vec![],
            lines: vec![],
            x,
            y,
            width: 0.0,
            height: 0.0,
            min_width: 0.0,
            min_height: 0.0,
            max_height: 0.0,
            smallest_width: f64::INFINITY,
            smallest_height: f64::INFINITY,
            average_height: 0.0,
            parent,
            stack: None,
            fixed: false,
            position_fixed: false,
        });
        id
    }
    fn notify(&mut self, r: usize) {
        let (mut width, mut height) = (0.0_f64, f64::NEG_INFINITY);
        for (i, &b) in self.rows[r].blocks.iter().enumerate() {
            if i > 0 {
                width += self.spacing;
            }
            width += self.blocks[b].width;
            height = height.max(self.blocks[b].height);
        }
        self.rows[r].width = width;
        self.rows[r].height = height;
    }
    fn row_add(&mut self, r: usize, b: usize) {
        self.rows[r].height = self.rows[r].height.max(self.blocks[b].height);
        if !self.rows[r].blocks.is_empty() {
            self.rows[r].width += self.spacing;
        }
        self.rows[r].width += self.blocks[b].width;
        self.rows[r].blocks.push(b);
    }
    fn row_remove(&mut self, r: usize, b: usize) {
        if let Some(i) = self.rows[r].blocks.iter().position(|&id| id == b) {
            self.rows[r].blocks.remove(i);
        }
        self.rows[r].width -= self.blocks[b].width;
        if !self.rows[r].blocks.is_empty() {
            self.rows[r].width -= self.spacing;
        }
        let mut height = f64::from_bits(1);
        for &other in &self.rows[r].blocks {
            height = height.max(self.blocks[other].height);
        }
        self.rows[r].height = height;
    }
    fn add(&mut self, b: usize, id: usize, new_row: bool) {
        if self.blocks[b].lines.is_empty() {
            let (x, y) = (self.blocks[b].x, self.blocks[b].y);
            self.blocks[b].lines.push(Line {
                x,
                y,
                ..Line::default()
            });
        }
        if new_row {
            let last = self.blocks[b].lines.last().unwrap();
            let y = last.y + last.height + self.spacing;
            let x = self.blocks[b].x;
            self.blocks[b].lines.push(Line {
                x,
                y,
                ..Line::default()
            });
        }
        self.blocks[b].children.push(id);
        self.blocks[b]
            .lines
            .last_mut()
            .unwrap()
            .add(self.nodes, id, self.spacing);
        let n = &self.nodes[id];
        let block = &mut self.blocks[b];
        block.smallest_width = block.smallest_width.min(n.width);
        block.width = block.width.max(block.lines.last().unwrap().width);
        block.min_width = block.min_width.max(
            n.width
                + if block.children.len() == 1 {
                    0.0
                } else {
                    self.spacing
                },
        );
        block.smallest_height = block.smallest_height.min(n.height);
        if block.children.len() > 1 {
            block.max_height += self.spacing;
        }
        block.max_height += n.height;
        block.min_height = block.min_height.max(n.height);
        let mut height = block.lines.len().saturating_sub(1) as f64 * self.spacing;
        for line in &block.lines {
            height += line.height;
        }
        block.height = height;
        let count = block.children.len() as f64;
        block.average_height = block.max_height / count - self.spacing * ((count - 1.0) / count);
        let parent = block.parent;
        self.notify(parent);
    }
    fn adjust_remove(&mut self, b: usize) {
        let block = &mut self.blocks[b];
        let (mut w, mut h) = (0.0_f64, 0.0_f64);
        for (i, line) in block.lines.iter().enumerate() {
            w = w.max(line.width);
            h += line.height;
            if i > 0 {
                h += self.spacing;
            }
        }
        block.width = w;
        block.height = h;
        block.min_width = 0.0;
        block.min_height = 0.0;
        block.max_height = 0.0;
        block.smallest_width = f64::INFINITY;
        block.smallest_height = f64::INFINITY;
        for &id in &block.children {
            let n = &self.nodes[id];
            block.smallest_width = block.smallest_width.min(n.width);
            block.min_width = block.min_width.max(n.width);
            block.min_height = block.min_height.max(n.height);
            block.smallest_height = block.smallest_height.min(n.height);
            block.max_height += n.height + self.spacing;
        }
        if !block.children.is_empty() {
            let count = block.children.len() as f64;
            block.average_height =
                block.max_height / count - self.spacing * ((count - 1.0) / count);
        }
        let parent = block.parent;
        self.notify(parent);
    }
    fn remove(&mut self, b: usize, id: usize) {
        if let Some(i) = self.blocks[b].children.iter().position(|&v| v == id) {
            self.blocks[b].children.remove(i);
        }
        for line in 0..self.blocks[b].lines.len() {
            if let Some(i) = self.blocks[b].lines[line]
                .nodes
                .iter()
                .position(|&v| v == id)
            {
                self.blocks[b].lines[line].nodes.remove(i);
                self.blocks[b].lines[line].update(self.nodes, self.spacing);
                if self.blocks[b].lines[line].nodes.is_empty() {
                    self.blocks[b].lines.remove(line);
                }
                break;
            }
        }
        self.adjust_remove(b);
    }
    fn move_block(&mut self, b: usize, x: f64, y: f64) {
        let block = &mut self.blocks[b];
        let (dx, dy) = (x - block.x, y - block.y);
        for &id in &block.children {
            self.nodes[id].x += dx;
            self.nodes[id].y += dy;
        }
        for line in &mut block.lines {
            line.x += dx;
            line.y += dy;
        }
        block.x = x;
        block.y = y;
    }
    fn layout_in(&mut self, b: usize, width: f64, place: bool) -> (f64, f64) {
        let (mut current_x, mut current_y, mut current_w, mut current_h, mut max_h, mut width_row) = (
            0.0_f64,
            self.blocks[b].y,
            0.0_f64,
            0.0_f64,
            0.0_f64,
            0.0_f64,
        );
        let mut index = 0;
        if place {
            let block = &self.blocks[b];
            self.blocks[b].lines = vec![Line {
                x: block.x,
                y: block.y,
                ..Line::default()
            }];
        }
        for id in self.blocks[b].children.clone() {
            let (w, h) = (self.nodes[id].width, self.nodes[id].height);
            let space = if index > 0 { self.spacing } else { 0.0 };
            if current_x + w + space > width && max_h > 0.0 {
                current_x = 0.0;
                current_y += max_h + self.spacing;
                current_w = current_w.max(width_row);
                current_h += max_h + self.spacing;
                max_h = 0.0;
                width_row = 0.0;
                index = 0;
                if place {
                    let x = self.blocks[b].x;
                    self.blocks[b].lines.push(Line {
                        x,
                        y: current_y,
                        ..Line::default()
                    });
                }
            }
            let space = if index > 0 { self.spacing } else { 0.0 };
            width_row += w + space;
            max_h = max_h.max(h);
            if place {
                self.blocks[b]
                    .lines
                    .last_mut()
                    .unwrap()
                    .add(self.nodes, id, self.spacing);
            }
            current_x += w + space;
            index += 1;
        }
        current_w = current_w.max(width_row);
        current_h += max_h;
        if place {
            self.blocks[b].width = current_w;
            self.blocks[b].height = current_h;
            let r = self.blocks[b].parent;
            self.notify(r);
        }
        (current_w, current_h)
    }
    fn place(&mut self, b: usize, width: f64) -> bool {
        let old = (self.blocks[b].width, self.blocks[b].height);
        self.layout_in(b, width, true) != old
    }
    fn height_for(&mut self, b: usize, width: f64) -> f64 {
        self.layout_in(b, width, false).1
    }
    fn width_for(&mut self, b: usize, height: f64) -> f64 {
        if self.blocks[b].max_height <= height {
            return self.blocks[b].min_width;
        }
        let min = self.blocks[b].min_width;
        let (w, h) = self.layout_in(b, min, false);
        if w <= min && h <= height {
            return min;
        }
        let (mut upper, mut lower, mut viable) = (self.blocks[b].width, min, self.blocks[b].width);
        let mut trial = (upper - lower) / 2.0 + lower;
        while lower + 1.0 < upper {
            let (w, h) = self.layout_in(b, trial, false);
            if w <= trial && h <= height {
                viable = trial;
                upper = trial;
            } else {
                lower = trial;
            }
            trial = (upper - lower) / 2.0 + lower;
        }
        viable
    }
    fn stack(&mut self, x: f64, y: f64) -> usize {
        let id = self.stacks.len();
        self.stacks.push(Stack {
            x,
            y,
            ..Stack::default()
        });
        id
    }
    fn stack_add(&mut self, s: usize, b: usize) {
        self.blocks[b].stack = Some(s);
        self.stacks[s].width = self.stacks[s].width.max(self.blocks[b].width);
        if !self.stacks[s].blocks.is_empty() {
            self.stacks[s].height += self.spacing;
        }
        self.stacks[s].height += self.blocks[b].height;
        self.stacks[s].blocks.push(b);
    }
    fn stack_update(&mut self, s: usize) {
        let (mut w, mut h) = (0.0_f64, 0.0_f64);
        for (i, &b) in self.stacks[s].blocks.iter().enumerate() {
            w = w.max(self.blocks[b].width);
            if i > 0 {
                h += self.spacing;
            }
            h += self.blocks[b].height;
        }
        self.stacks[s].width = w;
        self.stacks[s].height = h;
    }
    fn stack_move(&mut self, s: usize, x: f64, y: f64) {
        let (dx, dy) = (x - self.stacks[s].x, y - self.stacks[s].y);
        for b in self.stacks[s].blocks.clone() {
            self.move_block(b, self.blocks[b].x + dx, self.blocks[b].y + dy);
        }
        self.stacks[s].x = x;
        self.stacks[s].y = y;
    }
    fn stack_width_for(&mut self, s: usize, height: f64) -> f64 {
        if self.stacks[s].blocks.len() == 1 {
            return self.width_for(self.stacks[s].blocks[0], height);
        }
        let mut lower = 0.0_f64;
        for &b in &self.stacks[s].blocks {
            lower = lower.max(self.blocks[b].min_width);
        }
        let (mut upper, mut viable) = (self.stacks[s].width, self.stacks[s].width);
        let mut trial = (upper - lower) / 2.0 + lower;
        while lower + 1.0 < upper {
            let mut total = 0.0;
            for b in self.stacks[s].blocks.clone() {
                total += self.height_for(b, trial);
            }
            if total < height {
                viable = trial;
                upper = trial;
            } else {
                lower = trial;
            }
            trial = (upper - lower) / 2.0 + lower;
        }
        viable
    }
    fn stack_place(&mut self, s: usize, width: f64) {
        let (mut y, mut h, mut w) = (self.stacks[s].y, 0.0_f64, 0.0_f64);
        for b in self.stacks[s].blocks.clone() {
            self.move_block(b, self.stacks[s].x, y);
            self.place(b, width);
            w = w.max(self.blocks[b].width);
            y += self.blocks[b].height + self.spacing;
            h = y;
        }
        self.stacks[s].width = w;
        self.stacks[s].height = h;
    }
    fn row_set_y(&mut self, r: usize, y: f64) {
        let diff = y - self.rows[r].y;
        for s in self.rows[r].stacks.clone() {
            self.stack_move(s, self.stacks[s].x, self.stacks[s].y + diff);
        }
        self.rows[r].y = y;
    }
    fn similar(&self, b: usize, id: usize) -> bool {
        let h = self.nodes[id].height;
        let v = &self.blocks[b];
        (h >= v.smallest_height && h <= v.min_height)
            || (v.average_height * 0.5 <= h && v.average_height * 1.5 >= h)
    }
    fn initial(&mut self, width: f64) -> Vec<usize> {
        let mut result = vec![];
        let mut r = self.row(0.0);
        let b = self.block(0.0, 0.0, r);
        self.row_add(r, b);
        let (mut current_w, mut drawing_h) = (0.0, 0.0);
        for id in 0..self.nodes.len() {
            let mut b = *self.rows[r].blocks.last().unwrap();
            let mut potential = current_w + self.nodes[id].width;
            if !self.blocks[self.rows[r].blocks[0]].children.is_empty() {
                potential += self.spacing;
            }
            if potential > width || node_flag(&self.nodes[id], "rectpacking.inNewRow") {
                drawing_h += self.rows[r].height + self.spacing;
                result.push(r);
                r = self.row(drawing_h);
                b = self.block(0.0, self.rows[r].y, r);
                self.row_add(r, b);
            }
            if self.blocks[b].children.is_empty() || (!self.reevaluate && self.similar(b, id)) {
                self.add(b, id, false);
            } else {
                b = self.block(
                    self.blocks[b].x + self.blocks[b].width + self.spacing,
                    self.rows[r].y,
                    r,
                );
                self.row_add(r, b);
                self.add(b, id, false);
            }
            current_w = self.nodes[id].x + self.nodes[id].width;
        }
        result.push(r);
        result
    }
    fn use_height(&mut self, r: usize, b: usize) -> bool {
        let s = self.blocks[b].stack.unwrap();
        let old = self.stacks[s].width;
        if self.blocks[b].height < self.rows[r].height {
            let target = self.stack_width_for(s, self.rows[r].height);
            if self.stacks[s].width > target {
                self.stack_place(s, target);
                return old != self.stacks[s].width;
            }
        }
        false
    }
    fn place_rect(&mut self, r: usize, b: usize, id: usize, width: f64) -> bool {
        if !self.similar(b, id) {
            return false;
        }
        let line = self.blocks[b].lines.last().unwrap();
        let n = &self.nodes[id];
        if line.x + line.width + n.width + self.spacing <= width
            && (line.y - self.rows[r].y + n.height <= self.rows[r].height
                || self.rows[r].blocks.len() == 1)
        {
            self.add(b, id, false);
            return true;
        }
        if self.blocks[b].x + n.width <= width
            && self.blocks[b].y + self.blocks[b].height + n.height + self.spacing
                <= self.rows[r].y + self.rows[r].height
        {
            self.add(b, id, true);
            return true;
        }
        false
    }
    fn absorb(&mut self, r: usize, b: usize, next: usize, width: f64) -> bool {
        let mut changed = false;
        while let Some(&id) = self.blocks[next].children.first() {
            if !self.place_rect(r, b, id, width) {
                break;
            }
            changed = true;
            self.remove(next, id);
        }
        if self.blocks[next].children.is_empty() {
            self.row_remove(self.blocks[next].parent, next);
        }
        if changed {
            self.stack_update(self.blocks[b].stack.unwrap());
        }
        changed
    }
    fn below(
        &mut self,
        order: &mut Vec<usize>,
        r: usize,
        b: usize,
        next: usize,
        from_next: bool,
        width: f64,
        next_index: usize,
    ) -> bool {
        let remaining = width - self.blocks[b].x;
        let current_height = self.blocks[b].y - self.rows[r].y + self.height_for(b, remaining);
        if self.blocks[next].min_width + self.spacing > remaining {
            return false;
        }
        let next_height = self.height_for(next, remaining);
        if current_height + self.spacing + next_height > self.rows[r].height {
            return false;
        }
        self.place(b, remaining);
        self.blocks[b].fixed = true;
        self.place(next, remaining);
        self.move_block(
            next,
            self.blocks[b].x,
            self.blocks[b].y + self.blocks[b].height + self.spacing,
        );
        self.blocks[next].position_fixed = true;
        self.stack_add(self.blocks[b].stack.unwrap(), next);
        if from_next {
            let old_parent = self.blocks[next].parent;
            self.row_add(r, next);
            self.blocks[next].parent = r;
            if next_index < order.len() {
                self.row_remove(old_parent, next);
                if self.rows[order[next_index]].blocks.is_empty() {
                    order.remove(next_index);
                }
            }
        }
        true
    }
    fn beside(
        &mut self,
        order: &mut Vec<usize>,
        r: usize,
        b: usize,
        next: usize,
        width: f64,
        next_index: usize,
    ) -> bool {
        let s = self.blocks[b].stack.unwrap();
        let current_width =
            self.stack_width_for(s, self.rows[r].y + self.rows[r].height - self.stacks[s].y);
        let should_reevaluate =
            self.blocks[next].min_height > self.rows[r].height && self.reevaluate;
        let mut target_next = width - (self.stacks[s].x + current_width - self.spacing);
        let next_height = self.height_for(next, target_next);
        if should_reevaluate && next_height > self.blocks[next].min_height {
            return false;
        }
        if should_reevaluate {
            let mut potential = 0.0;
            for stack in self.rows[r].stacks.clone() {
                potential +=
                    self.stack_width_for(stack, self.blocks[next].min_height) + self.spacing;
            }
            target_next = width - potential;
        }
        if target_next < self.blocks[next].min_width {
            return false;
        }
        let last_row_optimization =
            next_index == order.len() - 1 && target_next >= self.rows[order[next_index]].width;
        if !should_reevaluate && next_height > self.rows[r].height && !last_row_optimization {
            return false;
        }
        if last_row_optimization && next_height > self.rows[r].height {
            self.blocks[b].height = next_height;
            let target = self.width_for(b, next_height);
            self.place(b, target);
        } else {
            self.stack_place(s, current_width);
            self.blocks[b].fixed = true;
        }
        self.place(next, width - (self.blocks[b].x + self.blocks[b].width));
        self.move_block(
            next,
            self.stacks[s].x + self.stacks[s].width,
            self.rows[r].y,
        );
        let old_parent = self.blocks[next].parent;
        self.row_add(r, next);
        self.blocks[next].parent = r;
        if next_index < order.len() {
            self.row_remove(old_parent, next);
            if self.rows[order[next_index]].blocks.is_empty() {
                order.remove(next_index);
            }
        }
        true
    }
    fn compact_row(&mut self, row_index: usize, order: &mut Vec<usize>, width: f64) -> bool {
        let mut changed = false;
        let mut repeat = false;
        let next_index = row_index + 1;
        let r = order[row_index];
        let mut current_stack = None;
        let mut index: isize = -1;
        loop {
            index += 1;
            if index >= self.rows[r].blocks.len() as isize {
                break;
            }
            let b = self.rows[r].blocks[index as usize];
            if self.blocks[b].fixed {
                continue;
            }
            if self.blocks[b].children.is_empty() {
                self.row_remove(r, b);
                index -= 1;
                changed = true;
                continue;
            }
            if !self.blocks[b].position_fixed {
                if let Some(s) = current_stack {
                    self.stack_update(s);
                }
                let x = current_stack
                    .map(|s| self.stacks[s].x + self.stacks[s].width + self.spacing)
                    .unwrap_or(0.0);
                let s = self.stack(x, self.rows[r].y);
                current_stack = Some(s);
                self.move_block(b, self.stacks[s].x + self.stacks[s].width, self.rows[r].y);
                self.rows[r].stacks.push(s);
                self.stack_add(s, b);
                self.blocks[b].position_fixed = true;
            }
            let mut next = if (index as usize) < self.rows[r].blocks.len() - 1 {
                Some(self.rows[r].blocks[index as usize + 1])
            } else {
                order
                    .get(next_index)
                    .and_then(|&r| self.rows[r].blocks.first())
                    .copied()
            };
            let from_next = next.is_some_and(|n| self.blocks[n].parent != r);
            if let Some(n) = next {
                if !self.blocks[n].children.is_empty()
                    && !node_flag(
                        &self.nodes[self.blocks[n].children[0]],
                        "rectpacking.inNewRow",
                    )
                {
                    self.place(b, width - self.blocks[b].x);
                    self.stack_update(self.blocks[b].stack.unwrap());
                    changed = self.absorb(r, b, n, width) || changed;
                } else {
                    self.row_remove(r, n);
                    break;
                }
                if self.blocks[n].children.is_empty() {
                    if next_index < order.len() {
                        self.row_remove(order[next_index], n);
                    }
                    next = None;
                    while order.len() > next_index && self.rows[order[next_index]].blocks.is_empty()
                    {
                        order.remove(next_index);
                    }
                }
                if next.is_none() {
                    index -= 1;
                    continue;
                }
                if self.below(order, r, b, n, from_next, width, next_index) {
                    changed = true;
                    continue;
                }
                if from_next {
                    let old_height = self.rows[r].height;
                    let next_min = self.blocks[n].min_height;
                    if self.beside(order, r, b, n, width, next_index) {
                        changed = true;
                        if old_height < next_min {
                            repeat = true;
                            self.blocks[n].parent = r;
                            break;
                        }
                        continue;
                    }
                    if self.use_height(r, b) {
                        self.blocks[b].fixed = true;
                        changed = true;
                        continue;
                    }
                } else if self.use_height(r, b) {
                    self.blocks[b].fixed = true;
                    changed = true;
                    continue;
                }
                if changed {
                    continue;
                }
            }
            if self.use_height(r, b) {
                self.blocks[b].fixed = true;
                changed = true;
                if let Some(n) = next {
                    self.blocks[n].position_fixed = false;
                }
            } else {
                self.stack_update(self.blocks[b].stack.unwrap());
            }
        }
        repeat
    }
    fn adjust_row(&mut self, r: usize) {
        let (mut width, mut height) = (0.0_f64, 0.0_f64);
        for (i, s) in self.rows[r].stacks.clone().into_iter().enumerate() {
            self.stack_update(s);
            height = height.max(self.stacks[s].height);
            if i > 0 {
                width += self.spacing;
            }
            width += self.stacks[s].width;
        }
        self.rows[r].width = width;
        self.rows[r].height = height;
    }
}

#[derive(Clone, Copy)]
struct Drawing {
    width: f64,
    height: f64,
    additional_height: f64,
    increase: f64,
    decrease: f64,
}
impl Arena<'_> {
    fn dimensions(&self, order: &[usize]) -> (f64, f64) {
        let (mut w, mut h) = (0.0_f64, 0.0);
        for (i, &r) in order.iter().enumerate() {
            w = w.max(self.rows[r].width);
            if i > 0 {
                h += self.spacing;
            }
            h += self.rows[r].height;
        }
        (w, h)
    }
    fn expand(&mut self, order: &[usize], width: f64, height: f64) {
        if order.is_empty() {
            return;
        }
        let per_row = height / order.len() as f64;
        for (i, &r) in order.iter().enumerate() {
            self.row_set_y(r, self.rows[r].y + per_row * i as f64);
            let per_stack = (width - self.rows[r].width) / self.rows[r].stacks.len() as f64;
            for (j, s) in self.rows[r].stacks.clone().into_iter().enumerate() {
                self.stack_move(s, self.stacks[s].x + j as f64 * per_stack, self.stacks[s].y);
                let per_block = (self.rows[r].height - self.stacks[s].height + per_row)
                    / self.stacks[s].blocks.len() as f64;
                for (k, b) in self.stacks[s].blocks.clone().into_iter().enumerate() {
                    self.move_block(b, self.blocks[b].x, self.blocks[b].y + k as f64 * per_block);
                    // Preserve block.expand's two operations; cancelling
                    // value.width algebraically changes source float bits.
                    let additional_width = self.stacks[s].width - self.blocks[b].width + per_stack;
                    let w = self.blocks[b].width + additional_width;
                    self.blocks[b].width = w;
                    self.blocks[b].height += per_block;
                    let per_line = per_block / self.blocks[b].lines.len() as f64;
                    for (l, line) in self.blocks[b].lines.iter_mut().enumerate() {
                        let per_node = (w - line.width) / line.nodes.len() as f64;
                        line.height += per_line;
                        line.width = w;
                        for (m, &id) in line.nodes.iter().enumerate() {
                            let n = &mut self.nodes[id];
                            n.x += m as f64 * per_node;
                            n.y += l as f64 * per_line;
                            n.width += per_node;
                            n.height = line.height;
                        }
                    }
                }
            }
        }
    }
    fn run(
        &mut self,
        width: f64,
        minimum: (f64, f64),
        padding: [f64; 4],
        simple: bool,
    ) -> (Drawing, Vec<usize>) {
        for n in self.nodes.iter_mut() {
            n.x = 0.0;
            n.y = 0.0;
        }
        self.blocks.clear();
        self.rows.clear();
        self.stacks.clear();
        let mut order = self.initial(width);
        let (mut increase, mut decrease, mut max_decrease) =
            (f64::INFINITY, f64::INFINITY, 0.0_f64);
        if simple {
            for &r in &order {
                for b in self.rows[r].blocks.clone() {
                    let s = self.stack(self.blocks[b].x, self.blocks[b].y);
                    self.stack_add(s, b);
                    self.rows[r].stacks.push(s);
                }
            }
        } else {
            let mut index = 0;
            while index < order.len() {
                let r = order[index];
                if index > 0 {
                    let prev = order[index - 1];
                    self.row_set_y(r, self.rows[prev].y + self.rows[prev].height + self.spacing);
                }
                if self.compact_row(index, &mut order, width) {
                    for b in self.rows[r].blocks.clone() {
                        self.blocks[b].fixed = false;
                        self.blocks[b].position_fixed = false;
                        self.adjust_remove(b);
                    }
                    self.rows[r].stacks.clear();
                    self.rows[r].width = width;
                    continue;
                }
                self.adjust_row(r);
                if index + 1 < order.len() {
                    let change = self.rows[r].width
                        + self.spacing
                        + self.blocks[self.rows[order[index + 1]].blocks[0]].width
                        - width;
                    // ELK intentionally cross-references the decrease fields here.
                    increase = change.min(decrease);
                    if let Some(&last) = self.rows[r].stacks.last() {
                        let change = self.stacks[last].width
                            + if self.rows[r].stacks.len() > 1 {
                                self.spacing
                            } else {
                                0.0
                            };
                        max_decrease = max_decrease.max(change);
                        decrease = max_decrease.min(change);
                    }
                }
                index += 1;
            }
            if order.len() == 1 {
                if let Some(&s) = self.rows[order[0]].stacks.last() {
                    if let Some(&b) = self.stacks[s].blocks.last() {
                        for line in &self.blocks[b].lines {
                            decrease = decrease.min(self.blocks[b].width - line.width);
                            increase = increase.min(line.width + self.spacing);
                        }
                    }
                }
            }
        }
        let (w, h) = self.dimensions(&order);
        let height = h.max(minimum.1 - (padding[0] + padding[2]));
        let additional_height = height - h;
        (
            Drawing {
                width: w.max(minimum.0 - (padding[1] + padding[3])),
                height: height + if simple { additional_height } else { 0.0 },
                additional_height,
                increase,
                decrease,
            },
            order,
        )
    }
}
pub(super) fn place(
    nodes: &mut [ElkNode],
    target: f64,
    minimum: (f64, f64),
    padding: [f64; 4],
    spacing: f64,
    aspect: f64,
    profile: &Profile,
) -> Result<(f64, f64), crate::LayeringError> {
    let mut arena = Arena {
        nodes,
        blocks: vec![],
        rows: vec![],
        stacks: vec![],
        spacing,
        reevaluate: profile.reevaluate && profile.packing == "COMPACTION",
    };
    let (mut drawing, order) = if profile.packing == "NONE" {
        let (mut w, mut h) = (0.0_f64, 0.0_f64);
        for n in arena.nodes.iter() {
            w = w.max(n.x + n.width);
            h = h.max(n.y + n.height);
        }
        let height = h.max(minimum.1 - (padding[0] + padding[2]));
        (
            Drawing {
                width: w.max(minimum.0 - (padding[1] + padding[3])),
                height: height + (height - h),
                additional_height: height - h,
                increase: f64::INFINITY,
                decrease: f64::INFINITY,
            },
            None,
        )
    } else {
        let (mut drawing, mut order) =
            arena.run(target, minimum, padding, profile.packing == "SIMPLE");
        let mut iterations = profile.iterations;
        while profile.packing == "COMPACTION" && iterations > 1 {
            let saved_nodes = arena.nodes.to_vec();
            let old_blocks = std::mem::take(&mut arena.blocks);
            let old_rows = std::mem::take(&mut arena.rows);
            let old_stacks = std::mem::take(&mut arena.stacks);
            let ratio = (drawing.width + padding[1] + padding[3])
                / (drawing.height + padding[0] + padding[2]);
            let candidate_width = if arena.nodes.len() > 1
                && drawing.increase != f64::INFINITY
                && ratio < aspect
            {
                target + drawing.increase
            } else if arena.nodes.len() > 1 && drawing.decrease != f64::INFINITY && ratio > aspect {
                minimum.0.max(target - drawing.decrease)
            } else {
                target
            };
            let (candidate, candidate_order) = arena.run(candidate_width, minimum, padding, false);
            let scale = |d: Drawing| {
                if d.width > 0.0 && d.height > 0.0 {
                    (aspect / d.width).min(1.0 / d.height)
                } else {
                    0.0
                }
            };
            if scale(candidate) >= scale(drawing) && !scale(candidate).is_nan() {
                drawing = candidate;
                order = candidate_order;
            } else {
                arena.nodes.clone_from_slice(&saved_nodes);
                arena.blocks = old_blocks;
                arena.rows = old_rows;
                arena.stacks = old_stacks;
            }
            iterations -= 1;
        }
        (drawing, Some(order))
    };
    if profile.whitespace != "NONE" {
        let Some(order) = order else {
            return Err(crate::LayeringError::Import(
                "rectpacking whitespace elimination requires a packing strategy that produces rows"
                    .into(),
            ));
        };
        if profile.whitespace == "TO_ASPECT_RATIO" {
            if drawing.width / drawing.height < aspect {
                drawing.width = drawing.height * aspect;
            } else {
                drawing.additional_height += drawing.width / aspect - drawing.height;
                drawing.height += drawing.additional_height;
            }
        }
        arena.expand(&order, drawing.width, drawing.additional_height);
    }
    Ok((drawing.width, drawing.height))
}
