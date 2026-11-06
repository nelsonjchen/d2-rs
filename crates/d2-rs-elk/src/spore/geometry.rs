// Copyright (c) 2017 Kiel University and others; 2026 The elk-go Authors.
// SPDX-License-Identifier: EPL-2.0
// Pinned internal/spore/{geometry,model}.go.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub(super) struct V {
    pub x: f64,
    pub y: f64,
}
impl V {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }
    pub fn add(self, b: Self) -> Self {
        Self::new(self.x + b.x, self.y + b.y)
    }
    pub fn sub(self, b: Self) -> Self {
        Self::new(self.x - b.x, self.y - b.y)
    }
    pub fn scale(self, k: f64) -> Self {
        Self::new(self.x * k, self.y * k)
    }
    pub fn neg(self) -> Self {
        Self::new(-self.x, -self.y)
    }
    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y).sqrt()
    }
    pub fn go_length(self) -> f64 {
        crate::js_trig::product_sum(self.y, self.y, self.x * self.x).sqrt()
    }
    pub fn distance(self, b: Self) -> f64 {
        self.sub(b).go_length()
    }
    pub fn cross(self, b: Self) -> f64 {
        crate::js_trig::product_sum(-self.y, b.x, self.x * b.y)
    }
    pub fn length_to(self, target: f64) -> Self {
        let len = self.length();
        let v = if len > 0. {
            Self::new(self.x / len, self.y / len)
        } else {
            self
        };
        v.scale(target)
    }
}
#[derive(Clone, Copy, Default, Debug)]
pub(super) struct Rect {
    pub p: V,
    pub size: V,
}
impl Rect {
    pub fn center(self) -> V {
        self.p.add(self.size.scale(0.5))
    }
    pub fn end(self) -> V {
        self.p.add(self.size)
    }
    pub fn moved(self, by: V) -> Self {
        Self {
            p: self.p.add(by),
            ..self
        }
    }
    pub fn edges(self) -> [(V, V); 4] {
        let tl = self.p;
        let tr = V::new(self.p.x + self.size.x, self.p.y);
        let bl = V::new(self.p.x, self.p.y + self.size.y);
        let br = self.end();
        [(tl, tr), (tl, bl), (br, tr), (br, bl)]
    }
}
pub(super) fn fuzzy(a: f64, b: f64, t: f64) -> i32 {
    if a == b || (a - b).abs() <= t || (a.is_nan() && b.is_nan()) {
        0
    } else if a < b {
        -1
    } else {
        1
    }
}
pub(super) fn java_cmp(a: f64, b: f64) -> std::cmp::Ordering {
    if a.is_nan() {
        return if b.is_nan() {
            std::cmp::Ordering::Equal
        } else {
            std::cmp::Ordering::Greater
        };
    }
    if b.is_nan() {
        return std::cmp::Ordering::Less;
    }
    a.total_cmp(&b)
}
pub(super) fn overlap(a: Rect, b: Rect) -> f64 {
    let h = (a.p.x - b.end().x).abs().min((a.end().x - b.p.x).abs());
    let v = (a.p.y - b.end().y).abs().min((a.end().y - b.p.y).abs());
    let dx = (a.center().x - b.center().x).abs();
    if dx > a.size.x / 2. + b.size.x / 2. {
        return 1.;
    }
    let dy = (a.center().y - b.center().y).abs();
    if dy > a.size.y / 2. + b.size.y / 2. {
        return 1.;
    }
    if dx == 0. && dy == 0. {
        return 0.;
    }
    if dx == 0. {
        return v / dy + 1.;
    }
    if dy == 0. {
        return h / dx + 1.;
    }
    (h / dx).min(v / dy) + 1.
}
pub(super) fn shortest(a: Rect, b: Rect) -> f64 {
    let h = (a.p.x - b.end().x).max(b.p.x - a.end().x);
    let v = (a.p.y - b.end().y).max(b.p.y - a.end().y);
    if (fuzzy(h, 0., 0.00001) >= 0) != (fuzzy(v, 0., 0.00001) >= 0) {
        return v.max(h);
    }
    let len = crate::go_math::hypot(v, h);
    if fuzzy(h, 0., 0.00001) > 0 { len } else { -len }
}
pub(super) fn underlap(a: Rect, b: Rect) -> f64 {
    let dx = (a.center().x - b.center().x).abs();
    let dy = (a.center().y - b.center().y).abs();
    let (mut hs, mut vs) = (1., 1.);
    if dx > a.size.x / 2. + b.size.x / 2. {
        let h = (a.p.x - b.end().x).abs().min((a.end().x - b.p.x).abs());
        hs = 1. - h / dx;
    }
    if dy > a.size.y / 2. + b.size.y / 2. {
        let v = (a.p.y - b.end().y).abs().min((a.end().y - b.p.y).abs());
        vs = 1. - v / dy;
    }
    (1. - hs.min(vs)) * V::new(dx, dy).length()
}
pub(super) fn touches(a: Rect, b: Rect) -> bool {
    fuzzy(a.p.x, b.end().x, 0.0001) <= 0
        && fuzzy(b.p.x, a.end().x, 0.0001) <= 0
        && fuzzy(a.p.y, b.end().y, 0.0001) <= 0
        && fuzzy(b.p.y, a.end().y, 0.0001) <= 0
}
pub(super) fn clip(v: V, size: V) -> V {
    let hw = size.x / 2.;
    let hh = size.y / 2.;
    let xs = if v.x.abs() > hw { hw / v.x.abs() } else { 1. };
    let ys = if v.y.abs() > hh { hh / v.y.abs() } else { 1. };
    v.scale(xs.min(ys))
}
fn equal(a: V, b: V) -> bool {
    (a.x - b.x).abs() <= 0.05 && (a.y - b.y).abs() <= 0.05
}
fn intersection(p: V, r: V, q: V, s: V) -> Option<V> {
    let pq = q.sub(p);
    let cross = pq.cross(r);
    let rs = r.cross(s);
    let t = pq.cross(s) / rs;
    let u = cross / rs;
    if rs == 0. {
        if cross != 0. {
            return None;
        }
        let center = q.add(s.scale(0.5));
        let d1 = p.distance(center);
        let d2 = p.add(r).distance(center);
        let len = s.go_length() * 0.5;
        if d1 < d2 && d1 <= len {
            return Some(p);
        }
        if d2 <= len {
            return Some(p.add(r));
        }
        return None;
    }
    if (0. ..=1.).contains(&t) && (0. ..=1.).contains(&u) {
        Some(V::new(
            crate::js_trig::product_sum(r.x, t, p.x),
            crate::js_trig::product_sum(r.y, t, p.y),
        ))
    } else {
        None
    }
}
fn rays(a1: V, a2: V, b1: V, b2: V, direction: V) -> f64 {
    let mut result = f64::INFINITY;
    let edge = intersection(a1, a2.sub(a1), b1.add(direction), b2.sub(b1))
        .is_some_and(|p| !equal(p, a1) && !equal(p, a2));
    let mut endpoint = false;
    if let Some(p) = intersection(a1, a2.sub(a1), b1, direction) {
        if equal(p, a1) == equal(p, a2) || edge {
            let d = p.sub(b1);
            let distance = crate::js_trig::product_sum(d.x, d.x, d.y * d.y).sqrt();
            #[cfg(feature = "diagnostic-traces")]
            if super::trace_active() {
                eprintln!(
                    "{}",
                    serde_json::json!({"trace":"spore-ray","a1":{ "X":a1.x,"Y":a1.y},"a2":{"X":a2.x,"Y":a2.y},"b1":{"X":b1.x,"Y":b1.y},"b2":{"X":b2.x,"Y":b2.y},"direction":{"X":direction.x,"Y":direction.y},"which":1,"intersection":{"X":p.x,"Y":p.y},"distance":distance})
                );
            }
            result = result.min(distance)
        } else {
            endpoint = true
        }
    }
    if let Some(p) = intersection(a1, a2.sub(a1), b2, direction) {
        if endpoint || equal(p, a1) == equal(p, a2) || edge {
            let d = p.sub(b2);
            let distance = crate::js_trig::product_sum(d.x, d.x, d.y * d.y).sqrt();
            #[cfg(feature = "diagnostic-traces")]
            if super::trace_active() {
                eprintln!(
                    "{}",
                    serde_json::json!({"trace":"spore-ray","a1":{ "X":a1.x,"Y":a1.y},"a2":{"X":a2.x,"Y":a2.y},"b1":{"X":b1.x,"Y":b1.y},"b2":{"X":b2.x,"Y":b2.y},"direction":{"X":direction.x,"Y":direction.y},"which":2,"intersection":{"X":p.x,"Y":p.y},"distance":distance})
                );
            }
            result = result.min(distance)
        }
    }
    result
}
pub(super) fn rect_distance(a: Rect, b: Rect, d: V) -> f64 {
    let mut result = f64::INFINITY;
    for (a1, a2) in a.edges() {
        for (b1, b2) in b.edges() {
            result = result.min(rays(a1, a2, b1, b2, d).min(rays(b1, b2, a1, a2, d.neg())))
        }
    }
    result
}
