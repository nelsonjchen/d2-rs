// Copyright (c) 2013, 2015, 2017, 2020 Kiel University and others.
// SPDX-License-Identifier: EPL-2.0
// Original mrtree/provider.go: importGraph through computeNeighbors.
use super::*;
impl Arena {
    pub(super) fn import(w: &ElkGraph, cfg: Config) -> (Self, Graph) {
        let mut a = Self {
            nodes: vec![],
            edges: vec![],
        };
        let mut shapes = BTreeMap::new();
        for (i, n) in w.children.iter().enumerate() {
            shapes.insert(n.id.clone(), i);
            for p in &n.ports {
                shapes.insert(p.id.clone(), i);
            }
            a.nodes.push(Node {
                origin: Some(i),
                pos: V::new(n.x + n.width / 2., n.y + n.height / 2.),
                size: V::new(n.width.max(1.), n.height.max(1.)),
                props: n.layout_options.clone().unwrap_or(Value::Null),
                ..Node::default()
            });
        }
        for source in 0..w.children.len() {
            for (i, e) in w.edges.iter().enumerate() {
                let Some((&s, &t)) = e
                    .sources
                    .first()
                    .and_then(|s| shapes.get(s))
                    .zip(e.targets.first().and_then(|s| shapes.get(s)))
                else {
                    continue;
                };
                if s != source || s == t {
                    continue;
                }
                let id = a.edges.len();
                a.nodes[s].out.push(id);
                a.nodes[t].inc.push(id);
                a.edges.push(Edge {
                    origin: Some(i),
                    source: s,
                    target: t,
                    bends: vec![],
                });
            }
        }
        let g = Graph {
            nodes: (0..a.nodes.len()).collect(),
            edges: (0..a.edges.len()).collect(),
            cfg,
            min: V::default(),
            max: V::default(),
        };
        (a, g)
    }
    pub(super) fn split(&self, g: &Graph) -> Vec<Graph> {
        if !g.cfg.separate {
            return vec![g.clone()];
        }
        let mut inc = vec![vec![]; self.nodes.len()];
        for &e in &g.edges {
            inc[self.edges[e].source].push(e);
            inc[self.edges[e].target].push(e)
        }
        let mut visited = vec![false; self.nodes.len()];
        let mut components = vec![];
        fn visit(a: &Arena, n: usize, g: &mut Graph, visited: &mut [bool], inc: &[Vec<usize>]) {
            if visited[n] {
                return;
            }
            visited[n] = true;
            g.nodes.push(n);
            for &e in &inc[n] {
                let edge = &a.edges[e];
                if edge.source != n {
                    visit(a, edge.source, g, visited, inc)
                }
                if edge.target != n {
                    visit(a, edge.target, g, visited, inc)
                }
                if !g.edges.contains(&e) {
                    g.edges.push(e)
                }
            }
        }
        for &n in &g.nodes {
            if !visited[n] {
                let mut c = g.settings();
                visit(self, n, &mut c, &mut visited, &inc);
                components.push(c)
            }
        }
        components
    }
    pub(super) fn treeify(&mut self, g: &Graph) {
        let mut visited = vec![0; self.nodes.len()];
        let mut eliminated = vec![];
        fn dfs(a: &Arena, n: usize, v: &mut [i32], eliminated: &mut Vec<usize>) {
            v[n] = 1;
            for &e in &a.nodes[n].out {
                let t = a.edges[e].target;
                match v[t] {
                    1 => eliminated.push(e),
                    2 => v[t] = 1,
                    _ => dfs(a, t, v, eliminated),
                }
            }
        }
        for &n in &g.nodes {
            if visited[n] != 0 {
                continue;
            }
            if g.cfg.search == "BFS" {
                let mut q = std::collections::VecDeque::from([n]);
                while let Some(node) = q.pop_front() {
                    visited[node] = 1;
                    for &e in &self.nodes[node].out {
                        let t = self.edges[e].target;
                        match visited[t] {
                            1 => eliminated.push(e),
                            2 => visited[t] = 1,
                            _ => q.push_back(t),
                        }
                    }
                }
            } else {
                dfs(self, n, &mut visited, &mut eliminated)
            }
            visited[n] = 2;
        }
        for e in eliminated {
            let (s, t) = (self.edges[e].source, self.edges[e].target);
            if let Some(i) = self.nodes[s].out.iter().position(|&v| v == e) {
                self.nodes[s].out.remove(i);
            }
            if let Some(i) = self.nodes[t].inc.iter().position(|&v| v == e) {
                self.nodes[t].inc.remove(i);
            }
        }
    }
    pub(super) fn root(&mut self, g: &mut Graph) -> usize {
        let mut roots = vec![];
        for &n in &g.nodes {
            self.nodes[n].root = self.nodes[n].inc.is_empty();
            if self.nodes[n].root {
                roots.push(n)
            }
        }
        if roots.len() == 1 {
            return roots[0];
        }
        let root = self.nodes.len();
        self.nodes.push(Node {
            dummy: true,
            root: true,
            ..Node::default()
        });
        for child in roots {
            self.nodes[child].root = false;
            let e = self.edges.len();
            self.edges.push(Edge {
                source: root,
                target: child,
                origin: None,
                bends: vec![],
            });
            self.nodes[root].out.push(e);
            self.nodes[child].inc.push(e);
            g.edges.push(e)
        }
        g.nodes.push(root);
        root
    }
    pub(super) fn fan(&mut self, n: usize) -> Vec<usize> {
        let mut levels = vec![1];
        for child in self.children(n) {
            let sub = self.fan(child);
            for (depth, count) in sub.into_iter().enumerate() {
                if levels.len() <= depth + 1 {
                    levels.resize(depth + 2, 0)
                }
                levels[depth + 1] += count;
            }
        }
        self.nodes[n].desc = levels.iter().sum();
        self.nodes[n].fan = levels.iter().skip(1).copied().max().unwrap_or(0);
        levels
    }
    pub(super) fn levels(&mut self, root: usize) {
        let mut current = vec![root];
        let mut depth = 0;
        while !current.is_empty() {
            let mut next = vec![];
            for n in current {
                self.nodes[n].level = depth;
                next.extend(self.children(n));
            }
            current = next;
            depth += 1
        }
    }
    fn constraint(&self, n: usize) -> i32 {
        integer(&self.nodes[n].props, "mrtree.positionConstraint").unwrap_or(-1)
    }
    fn reorder(&mut self, n: usize, mut children: Vec<usize>, descending: bool) {
        children.sort_by(|&a, &b| {
            if descending {
                self.nodes[b]
                    .position_index
                    .cmp(&self.nodes[a].position_index)
            } else {
                self.nodes[a]
                    .position_index
                    .cmp(&self.nodes[b].position_index)
            }
        });
        let old = self.nodes[n].out.clone();
        let mut reordered = vec![];
        for c in children {
            for &e in &old {
                if self.edges[e].target == c {
                    reordered.push(e)
                }
            }
        }
        self.nodes[n].out = reordered;
    }
    pub(super) fn order(&mut self, n: usize, weight: &str) {
        if weight == "MODEL_ORDER" {
            return;
        }
        if weight == "CONSTRAINT" {
            self.order_constraint(vec![n]);
            return;
        }
        self.order_weight(vec![n], weight == "DESCENDANTS");
    }
    fn order_weight(&mut self, mut current: Vec<usize>, desc: bool) {
        if current.is_empty() {
            return;
        }
        let weight = |a: &Self, n: usize| {
            if desc {
                a.nodes[n].desc
            } else {
                a.nodes[n].fan
            }
        };
        current.sort_by_key(|&n| weight(self, n));
        let mut first_leaf = current.len();
        while first_leaf > 0 && weight(self, current[first_leaf - 1]) == 0 {
            first_leaf -= 1
        }
        let (inners, leaves) = current.split_at(first_leaf);
        let mut leaves = leaves.to_vec();
        let mut pos = 0;
        if inners.is_empty() {
            for n in leaves {
                self.nodes[n].position_index = pos;
                pos += 1
            }
            return;
        }
        for &n in inners {
            self.nodes[n].position_index = pos;
            pos += 1;
            let children = self.children(n);
            self.order_weight(children.clone(), desc);
            self.reorder(n, children, true);
            let mut gap = self.nodes[n].out.len();
            let mut i = leaves.len();
            while i > 0 && gap > 0 {
                i -= 1;
                let n = leaves[i];
                if weight(self, n) != 0 {
                    break;
                }
                self.nodes[n].position_index = pos;
                pos += 1;
                gap -= 1;
                leaves.remove(i);
            }
        }
    }
    fn order_constraint(&mut self, current: Vec<usize>) {
        if current.is_empty() {
            return;
        }
        let (mut undefined, mut inside, mut outside) = (vec![], vec![], vec![]);
        for &n in &current {
            let c = self.constraint(n);
            if c < 0 {
                undefined.push(n)
            } else if (c as usize) < current.len() {
                inside.push(n)
            } else {
                outside.push(n)
            }
        }
        let mut sorted = vec![None; current.len()];
        let mut index = 0;
        while index < inside.len() {
            let n = inside[index];
            let t = self.constraint(n) as usize;
            if t < inside.len() && sorted[t].is_none() {
                sorted[t] = Some(n);
                inside.remove(index);
            } else {
                index += 1
            }
        }
        for n in inside {
            let t = self.constraint(n) as isize;
            let mut d = 0;
            loop {
                let right = t + d;
                if right >= 0 && (right as usize) < sorted.len() && sorted[right as usize].is_none()
                {
                    sorted[right as usize] = Some(n);
                    break;
                }
                let left = t - d;
                if left >= 0 && (left as usize) < sorted.len() && sorted[left as usize].is_none() {
                    sorted[left as usize] = Some(n);
                    break;
                }
                d += 1;
            }
        }
        outside.sort_by_key(|&n| std::cmp::Reverse(self.constraint(n)));
        let mut outside = outside.into_iter();
        for slot in sorted.iter_mut().rev() {
            if slot.is_none() {
                *slot = outside.next();
            }
        }
        let mut undefined = undefined.into_iter();
        for (i, slot) in sorted.iter_mut().enumerate() {
            if slot.is_none() {
                *slot = undefined.next()
            }
            self.nodes[slot.unwrap()].position_index = i;
        }
        for n in current {
            if self.nodes[n].fan == 0 {
                continue;
            }
            let children = self.children(n);
            self.order_constraint(children.clone());
            self.reorder(n, children, false);
        }
    }
    pub(super) fn level_heights(&mut self, root: usize, horizontal: bool) {
        let mut level = vec![root];
        while !level.is_empty() {
            let mut height = 0.0_f64;
            let mut next = vec![];
            for &n in &level {
                height = height.max(if horizontal {
                    self.nodes[n].size.x
                } else {
                    self.nodes[n].size.y
                });
                next.extend(self.children(n));
            }
            for n in level {
                self.nodes[n].level_height = height;
            }
            level = next
        }
    }
    pub(super) fn neighbors(&mut self, root: usize) {
        for n in self.descendants(root) {
            let n = &mut self.nodes[n];
            n.left = None;
            n.right = None;
            n.left_sib = None;
            n.right_sib = None;
        }
        let mut level = self.children(root);
        while !level.is_empty() {
            let mut left: Option<usize> = None;
            let mut next = vec![];
            for n in level {
                next.extend(self.children(n));
                if let Some(l) = left {
                    self.nodes[l].right = Some(n);
                    self.nodes[n].left = Some(l);
                    if self.parent(l) == self.parent(n) {
                        self.nodes[l].right_sib = Some(n);
                        self.nodes[n].left_sib = Some(l)
                    }
                }
                left = Some(n);
            }
            level = next
        }
    }
}
