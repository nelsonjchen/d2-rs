// SPDX-License-Identifier: MPL-2.0
//! Source-shaped D2 v0.9.0 ASCII target artist. No compilation or layout.
//! Source: d2ascii/d2ascii.go, asciicanvas, charset and asciishapes.
use d2_rs_render::target::TargetDiagram;
use serde_json::Value;

const FW: f64 = 9.75;
const FH: f64 = 18.0;
fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v[key].as_str().unwrap_or_default()
}
fn number(v: &Value, key: &str) -> f64 {
    v[key].as_f64().unwrap_or_default()
}
fn values(v: &Value) -> &[Value] {
    v.as_array().map(Vec::as_slice).unwrap_or_default()
}
fn round(v: f64) -> i64 {
    v.round() as i64
}

#[derive(Clone, Copy)]
struct Chars {
    standard: bool,
}
impl Chars {
    fn glyph(self, c: char) -> char {
        if !self.standard {
            return c;
        }
        match c {
            '┌' | '┐' | '└' | '┘' | '┬' | '┤' | '├' | '┴' => '+',
            '╭' | '╮' => '.',
            '╰' | '╯' => '\'',
            '─' | '‾' => '-',
            '│' | '▏' | '▕' => '|',
            '╱' => '/',
            '╲' => '\\',
            '╳' => 'X',
            '☁' => '@',
            '●' => 'o',
            '⬭' => 'O',
            '▲' => '^',
            '▶' => '>',
            '▼' => 'v',
            '◀' => '<',
            '`' => '~',
            _ => c,
        }
    }
}

struct Canvas {
    rows: Vec<Vec<char>>,
    chars: Chars,
}
impl Canvas {
    fn new(width: i64, height: i64, chars: Chars) -> Result<Self, String> {
        let width = usize::try_from(width).map_err(|_| "negative ASCII canvas width")?;
        let height = usize::try_from(height).map_err(|_| "negative ASCII canvas height")?;
        let cells = width
            .checked_mul(height)
            .ok_or("ASCII canvas size overflow")?;
        if cells > 16 * 1024 * 1024 {
            return Err("ASCII canvas exceeds native cell limit".into());
        }
        Ok(Self {
            rows: vec![vec![' '; width]; height],
            chars,
        })
    }
    fn inside(&self, x: i64, y: i64) -> bool {
        y >= 0
            && (y as usize) < self.rows.len()
            && x >= 0
            && (x as usize) < self.rows[y as usize].len()
    }
    fn set(&mut self, x: i64, y: i64, c: char) {
        if self.inside(x, y) {
            self.rows[y as usize][x as usize] = c;
        }
    }
    fn paint(&mut self, x: i64, y: i64, c: char) {
        self.set(x, y, self.chars.glyph(c));
    }
    fn get(&self, x: i64, y: i64) -> Option<char> {
        self.inside(x, y).then(|| self.rows[y as usize][x as usize])
    }
    fn label(&mut self, x: i64, y: i64, value: &str) {
        // Go's range index is a UTF-8 byte offset, not a rune count. The start
        // cell must itself be in bounds before any line is painted.
        if !self.inside(x, y) {
            return;
        }
        for (line_no, line) in value.split('\n').enumerate() {
            for (offset, c) in line.char_indices() {
                self.set(x + offset as i64, y + line_no as i64, c);
            }
        }
    }
    fn label_bytes(&mut self, x: i64, y: i64, value: &[u8]) {
        if !self.inside(x, y) {
            return;
        }
        for (line_no, line) in value.split(|b| *b == b'\n').enumerate() {
            let mut offset = 0;
            while offset < line.len() {
                let first = line[offset];
                let width = match first {
                    0..=0x7f => 1,
                    0xc2..=0xdf => 2,
                    0xe0..=0xef => 3,
                    0xf0..=0xf4 => 4,
                    _ => 1,
                };
                let valid = line
                    .get(offset..offset + width)
                    .and_then(|bytes| std::str::from_utf8(bytes).ok())
                    .and_then(|s| s.chars().next());
                self.set(
                    x + offset as i64,
                    y + line_no as i64,
                    valid.unwrap_or('\u{fffd}'),
                );
                offset += if valid.is_some() { width } else { 1 };
            }
        }
    }
    fn finish(&self) -> String {
        if self.rows.is_empty() {
            return String::new();
        }
        let nonempty = |row: &Vec<char>| row.iter().any(|c| !c.is_whitespace());
        let start = self.rows.iter().position(nonempty).unwrap_or(0);
        let end = self
            .rows
            .iter()
            .rposition(nonempty)
            .unwrap_or(self.rows.len() - 1);
        let width = self.rows[0].len();
        let occupied = |x: usize| {
            self.rows[start..=end]
                .iter()
                .any(|row| row.get(x).is_some_and(|c| *c != ' '))
        };
        let left = (0..width).find(|x| occupied(*x)).unwrap_or(0);
        let right = (0..width).rfind(|x| occupied(*x)).unwrap_or(0);
        let mut output = String::new();
        let mut previous: Option<(Vec<usize>, char)> = None;
        for row in &self.rows[start..=end] {
            let line: String = row
                .iter()
                .skip(left)
                .take(right.saturating_sub(left) + 1)
                .collect();
            let mut columns = Vec::new();
            let mut kind = None;
            let mut only_routes = true;
            for (offset, c) in line.char_indices() {
                if c == self.chars.glyph('│') || c == self.chars.glyph('─') {
                    columns.push(offset);
                    if let Some(old) = kind {
                        if old != c {
                            only_routes = false;
                            break;
                        }
                    } else {
                        kind = Some(c);
                    }
                } else if c != ' ' {
                    only_routes = false;
                }
            }
            let pattern = kind
                .filter(|_| only_routes && !columns.is_empty())
                .map(|kind| (columns, kind));
            if pattern.is_some() && pattern == previous {
                continue;
            }
            previous = pattern;
            output.push_str(&line);
            output.push('\n');
        }
        output
    }
}

fn label_y(y: i64, bottom: i64, height: i64, position: &str) -> i64 {
    if position.contains("OUTSIDE") {
        if position.contains("BOTTOM") {
            bottom + 1
        } else if position.contains("TOP") {
            y - 1
        } else {
            -1
        }
    } else if position.contains("TOP") {
        y + 1
    } else if position.contains("MIDDLE") {
        y + height / 2
    } else if position.contains("BOTTOM") {
        bottom - 1
    } else {
        -1
    }
}
fn adjust_width(width: i64, label: &str) -> i64 {
    if label.is_empty() {
        return width;
    }
    let available = width - label.len() as i64;
    if available < 2 {
        label.len() as i64 + 2
    } else if available % 2 == 1 {
        width - 1
    } else {
        width
    }
}
fn shape_label(
    canvas: &mut Canvas,
    x: i64,
    y: i64,
    bottom: i64,
    width: i64,
    height: i64,
    label: &str,
    position: &str,
) {
    if label.is_empty() {
        return;
    }
    let max = label.split('\n').map(str::len).max().unwrap_or(0) as i64;
    canvas.label(
        x + (width - max) / 2,
        label_y(y, bottom, height, position),
        label,
    );
}
fn border(canvas: &mut Canvas, x: i64, y: i64, right: i64, bottom: i64, symbol: Option<char>) {
    // In Go's corner map later insertions win when dimensions collapse.
    let corners = [
        ((x, y), '┌'),
        ((right, y), '┐'),
        ((x, bottom), '└'),
        ((right, bottom), '┘'),
    ];
    for ix in x..=right {
        for iy in y..=bottom {
            if let Some((_, c)) = corners.iter().rev().find(|(p, _)| *p == (ix, iy)) {
                canvas.paint(ix, iy, *c);
            } else if iy == y && ix == x + 1 && symbol.is_some() {
                canvas.paint(ix, iy, symbol.unwrap());
            } else if ix == x || ix == right {
                canvas.paint(ix, iy, '│');
            } else if iy == y || iy == bottom {
                canvas.paint(ix, iy, '─');
            }
        }
    }
}
fn separator(canvas: &mut Canvas, x: i64, right: i64, y: i64) {
    for ix in x..=right {
        canvas.paint(
            ix,
            y,
            if ix == x {
                '├'
            } else if ix == right {
                '┤'
            } else {
                '─'
            },
        );
    }
}
fn field(canvas: &mut Canvas, x: i64, right: i64, y: i64, width: i64, name: &str, kind: &str) {
    canvas.label(x + 1, y, name);
    if !kind.is_empty() {
        let maximum = width - name.len() as i64 - 4;
        let mut bytes = kind.as_bytes().to_vec();
        if bytes.len() as i64 > maximum && maximum > 3 {
            bytes.truncate(maximum as usize - 3);
            bytes.extend_from_slice(b"...");
        }
        canvas.label_bytes(right - bytes.len() as i64 - 1, y, &bytes);
    }
}

#[derive(Clone, Copy, Default)]
struct Boundary {
    left: i64,
    top: i64,
    right: i64,
    bottom: i64,
}
impl Boundary {
    fn contains(self, x: i64, y: i64) -> bool {
        x >= self.left && x <= self.right && y >= self.top && y <= self.bottom
    }
}
fn sign(v: f64) -> i64 {
    if v > 0.0 {
        1
    } else if v < 0.0 {
        -1
    } else {
        0
    }
}
fn preserve_width(shape: &Value, connections: &[Value]) -> bool {
    let (px, py, w, h) = (
        number(&shape["pos"], "x"),
        number(&shape["pos"], "y"),
        number(shape, "width"),
        number(shape, "height"),
    );
    connections.iter().any(|c| {
        let route = values(&c["route"]);
        [route.first(), route.last()]
            .into_iter()
            .flatten()
            .any(|p| {
                (number(p, "x") - (px + w)).abs() < FW / 2.0
                    && number(p, "y") >= py
                    && number(p, "y") <= py + h
            })
    })
}
fn preserve_height(shape: &Value, connections: &[Value]) -> bool {
    let (px, py, w) = (
        number(&shape["pos"], "x"),
        number(&shape["pos"], "y"),
        number(shape, "width"),
    );
    connections.iter().any(|c| {
        values(&c["route"]).windows(2).any(|p| {
            (number(&p[0], "y") - number(&p[1], "y")).abs() < 0.1
                && (number(&p[0], "y") - py).abs() < FH
                && number(&p[0], "x").min(number(&p[1], "x")) <= px + w
                && number(&p[0], "x").max(number(&p[1], "x")) >= px
        })
    })
}
fn boundary(shape: &Value, connections: &[Value]) -> Boundary {
    let (mut px, py, mut width, mut height) = (
        number(&shape["pos"], "x"),
        number(&shape["pos"], "y"),
        number(shape, "width"),
        number(shape, "height"),
    );
    if shape["multiple"] == Value::Bool(true) {
        px -= 10.0;
        width += 10.0;
        height += 10.0;
    }
    let (mut x, mut y, mut w, mut h) = (
        round(px / FW),
        round(py / FH),
        round(width / FW),
        round(height / FH),
    );
    let label = text(shape, "label");
    if preserve_width(shape, connections) && !label.is_empty() {
        let available = w - label.len() as i64;
        if available >= 2 && available % 2 == 1 {
            width += FW.trunc();
            x = round(px / FW);
            y = round(py / FH);
            w = round(width / FW);
            h = round(height / FH);
        }
    }
    if !label.is_empty() && h % 2 == 0 && !preserve_height(shape, connections) {
        if h > 2 {
            h -= 1;
            y += 1;
        } else {
            h += 1;
        }
    }
    w = adjust_width(w, label);
    Boundary {
        left: x,
        top: y,
        right: x + w,
        bottom: y + h,
    }
}
type RoutePoint = (f64, f64);
fn shift_until_empty(canvas: &Canvas, point: &mut RoutePoint, dx: f64, dy: f64) {
    while let Some(c) = canvas.get(round(point.0), round(point.1)) {
        if c == ' ' {
            break;
        }
        point.0 += dx;
        point.1 += dy;
    }
}
fn process_route(
    canvas: &Canvas,
    route: &[RoutePoint],
    from: Boundary,
    to: Boundary,
) -> Vec<RoutePoint> {
    let mut merged = Vec::new();
    if let Some(first) = route.first() {
        merged.push(*first);
    }
    if route.len() >= 2 {
        let mut last = route[0];
        let mut direction = sign(route[0].0 - route[1].0) + 2 * sign(route[0].1 - route[1].1);
        for point in &route[1..] {
            let new = sign(last.0 - point.0) + 2 * sign(last.1 - point.1);
            if direction != new {
                merged.push(last);
                direction = new;
            }
            last = *point;
        }
        if merged.last() != Some(&last) {
            merged.push(last);
        }
    }
    for point in &mut merged {
        point.0 = round(point.0 / FW) as f64;
        point.1 = round(point.1 / FH) as f64;
    }
    let mut points: Vec<RoutePoint> = Vec::new();
    for point in merged {
        if let Some(&(px, py)) = points.last() {
            if (px - point.0).abs() > 0.5 && (py - point.1).abs() > 0.5 {
                points.push((point.0, py));
            }
        }
        points.push(point);
    }
    if points.len() >= 2 {
        let (first, second) = (points[0], points[1]);
        let (dx, dy) = (second.0 - first.0, second.1 - first.1);
        if from.contains(round(first.0), round(first.1)) {
            let length = (dx * dx + dy * dy).sqrt();
            if length > 0.0 {
                while from.contains(round(points[0].0), round(points[0].1)) {
                    points[0].0 += dx / length;
                    points[0].1 += dy / length;
                }
            }
        } else if dy.abs() < 0.1 && dx != 0.0 {
            shift_until_empty(canvas, &mut points[0], sign(dx) as f64, 0.0);
        } else if dx.abs() < 0.1 && dy != 0.0 {
            shift_until_empty(canvas, &mut points[0], 0.0, sign(dy) as f64);
        }
        let last = points.len() - 1;
        let (a, b) = (points[last - 1], points[last]);
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        if to.contains(round(b.0), round(b.1)) {
            let length = (dx * dx + dy * dy).sqrt();
            if length > 0.0 {
                while to.contains(round(points[last].0), round(points[last].1)) {
                    points[last].0 -= dx / length;
                    points[last].1 -= dy / length;
                }
            }
        } else if dy.abs() < 0.1 && dx != 0.0 {
            shift_until_empty(canvas, &mut points[last], -sign(dx) as f64, 0.0);
        } else if dx.abs() < 0.1 && dy != 0.0 {
            shift_until_empty(canvas, &mut points[last], 0.0, -sign(dy) as f64);
        }
    }
    points
}
fn raw_label(canvas: &mut Canvas, x: i64, y: i64, label: &str) {
    for (line_no, line) in label.split('\n').enumerate() {
        for (offset, c) in line.char_indices() {
            canvas.set(x + offset as i64, y + line_no as i64, c);
        }
    }
}
fn arrow(dx: f64, dy: f64) -> Option<char> {
    match (sign(dx), sign(dy)) {
        (0, -1) => Some('▲'),
        (1, 0) => Some('▶'),
        (0, 1) => Some('▼'),
        (-1, 0) => Some('◀'),
        _ => None,
    }
}
fn draw_route(
    canvas: &mut Canvas,
    connection: &Value,
    shapes: &[Value],
    connections: &[Value],
    offset_x: f64,
    offset_y: f64,
) -> Result<(), String> {
    let original = values(&connection["route"])
        .iter()
        .map(|p| (number(p, "x") + offset_x, number(p, "y") + offset_y))
        .collect::<Vec<_>>();
    let (mut from, mut to) = (Boundary::default(), Boundary::default());
    for shape in shapes {
        if text(shape, "id") == text(connection, "src") {
            from = boundary(shape, connections);
        } else if text(shape, "id") == text(connection, "dst") {
            to = boundary(shape, connections);
        }
    }
    let route = process_route(canvas, &original, from, to);
    let mut turns = std::collections::BTreeMap::new();
    for triple in route.windows(3) {
        let (a, b, c) = (triple[0], triple[1], triple[2]);
        let key = (
            sign(b.0 - a.0),
            sign(b.1 - a.1),
            sign(c.0 - b.0),
            sign(c.1 - b.1),
        );
        let corner = match key {
            (-1, 0, 0, -1) | (0, 1, 1, 0) => Some('└'),
            (-1, 0, 0, 1) | (0, -1, 1, 0) => Some('┌'),
            (0, -1, -1, 0) | (1, 0, 0, 1) => Some('┐'),
            (0, 1, -1, 0) | (1, 0, 0, -1) => Some('┘'),
            _ => None,
        };
        if let Some(corner) = corner {
            turns.insert((round(b.0), round(b.1)), corner);
        }
    }
    let label = text(connection, "label");
    let mut label_position = None;
    if !label.trim().is_empty() {
        let (mut largest, mut old_scale) = (0.0, 0.0);
        for (index, pair) in route.windows(2).enumerate() {
            let (a, b) = (pair[0], pair[1]);
            let (dx, dy) = ((a.0 - b.0).abs(), (a.1 - b.1).abs());
            let difference = dx.max(dy);
            let scale = sign(dx).abs() as f64 * FW + sign(dy).abs() as f64 * FH;
            if difference * scale > largest * old_scale {
                largest = difference;
                let best_x = if difference == dx {
                    a.0 + sign(b.0 - a.0) as f64 * difference / 2.0
                } else {
                    a.0
                };
                let length = label.split('\n').map(str::len).max().unwrap_or(0) as i64;
                label_position = Some((index, round(best_x) - length / 2, difference));
            }
            old_scale = scale;
        }
    }
    for (index, pair) in route.windows(2).enumerate() {
        let ((ax, ay), (cx, cy)) = (pair[0], pair[1]);
        let step = (cx - ax).abs().max((cy - ay).abs());
        if step == 0.0 {
            continue;
        }
        let (sx, sy) = ((cx - ax) / step, (cy - ay) / step);
        let (mut fx, mut fy) = (ax, ay);
        let (mut x, mut y) = (round(ax), round(ay));
        let mut attempt = 0;
        loop {
            attempt += 1;
            if (x == round(cx) && y == round(cy)) || attempt == 200 {
                break;
            }
            x = round(fx);
            y = round(fy);
            if !canvas.inside(x, y)
                || canvas
                    .get(x, y)
                    .is_some_and(|c| d2_rs_language::parser::is_go_letter_or_digit(c))
            {
                fx += sx;
                fy += sy;
                continue;
            }
            if let Some(corner) = turns.get(&(x, y)) {
                canvas.paint(x, y, *corner);
            } else if index + 1 == original.len().saturating_sub(1)
                && x == round(cx)
                && y == round(cy)
                && text(connection, "dstArrow") != "none"
            {
                if let Some(glyph) = arrow(sx, sy) {
                    let boundary_char = canvas.get(x, y).is_some_and(|c| {
                        ['─', '│', '┌', '┐', '└', '┘', '╭', '╮', '╰', '╯']
                            .into_iter()
                            .any(|g| canvas.chars.glyph(g) == c)
                    });
                    let (arrow_x, arrow_y) = (x - round(sx), y - round(sy));
                    if boundary_char && canvas.inside(arrow_x, arrow_y) {
                        canvas.paint(arrow_x, arrow_y, glyph);
                    } else {
                        canvas.paint(x, y, glyph);
                    }
                }
                if !connection["dstLabel"].is_null() {
                    let value = text(&connection["dstLabel"], "label");
                    let length = value.split('\n').map(str::len).max().unwrap_or(0) as i64;
                    let (lx, ly) = if sx.abs() > 0.0 {
                        (
                            if sx > 0.0 {
                                cx as i64 - 1 - length
                            } else {
                                cx as i64
                            },
                            (cy - 1.0) as i64,
                        )
                    } else if sy.abs() > 0.0 {
                        ((cx + 1.0) as i64, (cy - 1.0) as i64)
                    } else {
                        (0, 0)
                    };
                    raw_label(canvas, lx + 2, ly, value);
                }
            } else if index == 0
                && x == round(ax)
                && y == round(ay)
                && text(connection, "srcArrow") != "none"
            {
                if let Some(glyph) = arrow(-sx, -sy) {
                    canvas.paint(x, y, glyph);
                }
                if !connection["srcLabel"].is_null() {
                    let value = text(&connection["srcLabel"], "label");
                    let length = value.split('\n').map(str::len).max().unwrap_or(0) as i64;
                    let (lx, ly) = if sx.abs() > 0.0 {
                        (
                            if sx > 0.0 {
                                ax as i64
                            } else {
                                ax as i64 - 1 - length
                            },
                            (cy - 1.0) as i64,
                        )
                    } else if sy.abs() > 0.0 {
                        ((cx + 1.0) as i64, (cy - 1.0) as i64)
                    } else {
                        (0, 0)
                    };
                    raw_label(canvas, lx, ly, value);
                }
            } else {
                let existing = canvas.get(x, y).unwrap_or(' ');
                let overwrite = existing != ' ';
                if sx == 0.0 {
                    let junction = [from, to].into_iter().any(|b| {
                        (y == b.top || y == b.bottom) && existing == canvas.chars.glyph('─')
                    });
                    let skip = (existing == '_' && (y == from.bottom || y == to.bottom))
                        || (existing == canvas.chars.glyph('‾') && (y == from.top || y == to.top));
                    if overwrite && junction {
                        canvas.paint(x, y, if sy > 0.0 { '┬' } else { '┴' });
                    } else if !(overwrite && skip) {
                        canvas.paint(x, y, '│');
                    }
                } else {
                    let junction = [from, to].into_iter().any(|b| {
                        (x == b.right - 1 || x == b.left - 1) && existing == canvas.chars.glyph('│')
                    });
                    canvas.paint(
                        x,
                        y,
                        if overwrite && junction {
                            if sx > 0.0 {
                                '├'
                            } else {
                                '┤'
                            }
                        } else {
                            '─'
                        },
                    );
                }
            }
            if let Some((segment, label_x, difference)) = label_position {
                let middle = if sy != 0.0 {
                    round(ay) + round(difference / 2.0) * sign(sy) == y
                } else if sx != 0.0 {
                    round(ax) + round(difference / 2.0) * sign(sx) == x
                } else {
                    false
                };
                if segment == index && middle {
                    if sy != 0.0 {
                        canvas.set(x, y, ' ');
                        raw_label(canvas, label_x, y, label);
                    } else if sx != 0.0 {
                        let position = text(connection, "labelPosition");
                        let y_factor = if position.contains("TOP") {
                            -1
                        } else if position.contains("BOTTOM") {
                            1
                        } else {
                            0
                        };
                        let mut x_pos = label_x;
                        if position.contains("LEFT") {
                            let point = original
                                .get(segment + ((sign(sx) - 1) / 2).unsigned_abs() as usize)
                                .ok_or("source ASCII route label index out of range")?;
                            x_pos = point.0 as i64;
                        } else if position.contains("RIGHT") {
                            let point = original
                                .get(segment + ((sign(sx) + 1) / 2) as usize)
                                .ok_or("source ASCII route label index out of range")?;
                            x_pos = point.0 as i64
                                - label.split('\n').map(str::len).max().unwrap_or(0) as i64 / 2;
                        }
                        raw_label(canvas, x_pos, y + y_factor, label);
                    }
                }
            }
            fx += sx;
            fy += sy;
        }
    }
    Ok(())
}

fn draw_shape(
    canvas: &mut Canvas,
    shape: &Value,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    preserve_height: bool,
) -> Result<(), String> {
    let kind = text(shape, "type");
    let label = text(shape, "label");
    let position = text(shape, "labelPosition");
    let (x, mut y, mut w, mut h) = (
        round(x / FW),
        round(y / FH),
        round(width / FW),
        round(height / FH),
    );
    let rectangle = !matches!(
        kind,
        "page"
            | "hexagon"
            | "person"
            | "stored_data"
            | "cylinder"
            | "package"
            | "parallelogram"
            | "queue"
            | "step"
            | "callout"
            | "document"
            | "diamond"
            | "class"
            | "sql_table"
    );
    if rectangle || kind == "page" {
        if !label.is_empty() && h % 2 == 0 && !(rectangle && preserve_height) {
            if h > 2 {
                h -= 1;
                y += 1;
            } else {
                h += 1;
            }
        }
        w = adjust_width(w, label);
    }
    if kind == "cylinder" {
        w = adjust_width(w, label);
    }
    if kind == "stored_data" {
        if h < 5 {
            h = 5;
        } else if h % 2 == 0 {
            h += 1;
        }
    }
    if kind == "step" && h % 2 == 1 {
        h += 1;
    }
    if kind == "diamond" {
        if w % 2 == 0 {
            w += 1;
        }
        if h % 2 == 0 {
            h += 1;
        }
    }
    let (right, bottom) = if rectangle || matches!(kind, "class" | "sql_table") {
        (x + w, y + h)
    } else {
        (x + w - 1, y + h - 1)
    };
    if rectangle {
        let symbol = match kind {
            "cloud" => Some('☁'),
            "circle" => Some('●'),
            "oval" => Some('⬭'),
            _ => None,
        };
        border(canvas, x, y, right, bottom, symbol);
    } else if matches!(kind, "class" | "sql_table") {
        border(canvas, x, y, right, bottom, None);
        let fields = values(&shape["fields"]);
        let methods = values(&shape["methods"]);
        let columns = values(&shape["columns"]);
        if (kind == "class" && fields.is_empty() && methods.is_empty())
            || (kind == "sql_table" && columns.is_empty())
        {
            shape_label(canvas, x, y, bottom, w, h, label, position);
            return Ok(());
        }
        let mut current = y + 1;
        if !label.is_empty() {
            for iy in current..=current + 1 {
                for ix in x + 1..right {
                    canvas.set(ix, iy, ' ');
                }
            }
            canvas.label(x + (w - label.len() as i64) / 2, current, label);
            separator(canvas, x, right, current + 2);
            current += 3;
        }
        if kind == "class" {
            for (members, method) in [(fields, false), (methods, true)] {
                for member in members {
                    if current >= bottom {
                        break;
                    }
                    let prefix = match text(member, "visibility") {
                        "protected" => "#",
                        "private" => "-",
                        _ => "+",
                    };
                    let name = format!("{prefix}{}", text(member, "name"));
                    field(
                        canvas,
                        x,
                        right,
                        current,
                        w,
                        &name,
                        text(member, if method { "return" } else { "type" }),
                    );
                    current += 1;
                }
                if !method && !fields.is_empty() && !methods.is_empty() && current < bottom {
                    separator(canvas, x, right, current);
                    current += 1;
                }
            }
        } else {
            for column in columns {
                if current >= bottom {
                    break;
                }
                field(
                    canvas,
                    x,
                    right,
                    current,
                    w,
                    text(&column["name"], "label"),
                    text(&column["type"], "label"),
                );
                current += 1;
            }
        }
        return Ok(());
    } else if kind == "page" {
        border(canvas, x, y, right, bottom, None);
        canvas.set(right, y, ' ');
        canvas.paint(right - 1, y, '┐');
        canvas.paint(right - 1, y + 1, '╲');
        canvas.paint(right, y + 1, '┐');
    } else if kind == "diamond" {
        for ix in x..=right {
            for iy in y..=bottom {
                if (iy == y || iy == bottom) && ix - x == w / 2 {
                    canvas.paint(ix, iy, '`');
                } else if (ix == x || ix == right) && iy - y == h / 2 {
                    canvas.paint(ix, iy, '-');
                }
            }
        }
        let route = [
            (x, y + h / 2),
            (x + w / 2, y),
            (right, y + h / 2),
            (x + w / 2, bottom),
            (x, y + h / 2),
        ];
        for points in route.windows(2) {
            let ((ax, ay), (bx, by)) = (points[0], points[1]);
            let steps = (bx - ax).abs().max((by - ay).abs());
            let (mut fx, mut fy) = (ax as f64, ay as f64);
            for _ in 0..steps {
                fx += (bx - ax) as f64 / steps as f64;
                fy += (by - ay) as f64 / steps as f64;
                canvas.paint(round(fx), round(fy), '*');
            }
        }
    } else {
        let document_curves = if kind == "document" {
            let n = (w - 2) / 2;
            if n < 0 {
                return Err("negative ASCII document curve length".into());
            }
            let curve = (n / 2).min(3) + 1;
            let mut left = Vec::new();
            let mut right = Vec::new();
            for i in 0..n {
                let index = if i < curve {
                    i
                } else if (i - n + 1).abs() < curve {
                    (i - n + 1).abs()
                } else {
                    3
                } as usize;
                left.push(b"`-._"[index] as char);
                right.push(".-`‾".as_bytes()[index] as char);
            }
            Some((n, left, right))
        } else {
            None
        };
        for ix in x..=right {
            for iy in y..=bottom {
                let (rx, ry) = (ix - x, iy - y);
                let mut c = None;
                match kind {
                    "hexagon" => {
                        let offset = (h as f64 / 2.0).ceil() as i64;
                        if iy == y && ix >= x + offset && ix <= right - offset {
                            c = Some('‾');
                        } else if iy == bottom && ix >= x + offset && ix <= right - offset {
                            c = Some('_');
                        } else if offset % 2 == 1
                            && (ix == x || ix == right)
                            && iy == y + offset - 1
                        {
                            c = Some('╳');
                        } else if ry + rx + 1 == offset
                            || (bottom - iy) + (right - ix) + 1 == offset
                        {
                            c = Some('╱');
                        } else if ry + (right - ix) + 1 == offset
                            || (bottom - iy) + rx + 1 == offset
                        {
                            c = Some('╲');
                        }
                    }
                    "parallelogram" => {
                        if rx + ry == h - 1 || rx + ry == w - 1 {
                            c = Some('╱');
                        } else if iy == y && ix >= x + h && ix < right {
                            c = Some('‾');
                        } else if iy == bottom && ix > x && ix <= right - h {
                            c = Some('_');
                        }
                    }
                    "package" => {
                        let corners = [
                            ((x, y), '┌'),
                            ((x + w / 2, y), '┐'),
                            ((right, y + 1), '┐'),
                            ((x + w / 2, y + 1), '└'),
                            ((x, bottom), '└'),
                            ((right, bottom), '┘'),
                        ];
                        if let Some((_, v)) = corners.iter().rev().find(|(p, _)| *p == (ix, iy)) {
                            c = Some(*v);
                        } else if (iy == y && ix > x && ix < x + w / 2)
                            || (iy == bottom && ix > x && ix < right)
                            || (iy == y + 1 && ix > x + w / 2 && ix < right)
                        {
                            c = Some('─');
                        } else if (ix == x && iy > y && iy < bottom)
                            || (ix == right && iy > y + 1 && iy < bottom)
                        {
                            c = Some('│');
                        }
                    }
                    "cylinder" => {
                        if iy != y && iy != bottom && (ix == x || ix == right) {
                            c = Some('│');
                        } else if iy == y {
                            if ix == x + 1 || ix == right - 1 {
                                c = Some('.');
                            } else if ix == x + 2 || ix == right - 2 {
                                c = Some('-');
                            } else if ix > x + 2 && ix < right - 2 {
                                c = Some('‾');
                            }
                        } else if iy == bottom || iy == y + 1 {
                            if ix == x + 1 {
                                c = Some('╲');
                            } else if ix == right - 1 {
                                c = Some('╱');
                            } else if ix == x + 2 || ix == right - 2 {
                                c = Some('-');
                            } else if ix > x + 2 && ix < right - 2 {
                                c = Some('_');
                            }
                        }
                    }
                    "queue" => {
                        if (iy == y && (ix == x + 1 || ix == right - 2))
                            || (iy == bottom && ix == right - 1)
                        {
                            c = Some('╱');
                        } else if (iy == y && ix == right - 1)
                            || (iy == bottom && (ix == x + 1 || ix == right - 2))
                        {
                            c = Some('╲');
                        } else if (ix == x || ix == right || ix == right - 3)
                            && iy > y
                            && iy < bottom
                        {
                            c = Some('│');
                        } else if iy == y && ix > x + 1 && ix < right - 1 {
                            c = Some('‾');
                        } else if iy == bottom && ix > x + 1 && ix < right - 3 {
                            c = Some('_');
                        }
                    }
                    "step" => {
                        if (ix < x + h / 2 && rx - ry == 0)
                            || (ix > right - h / 2 && (rx - ry).abs() == w - h / 2)
                        {
                            c = Some('╲');
                        } else if (ix < x + h / 2 && rx + ry == h - 1)
                            || (ix > right - h / 2 && rx + ry == w - 1 + h / 2)
                        {
                            c = Some('╱');
                        } else if iy == y && ix > x && ix < right - h / 2 {
                            c = Some('‾');
                        } else if iy == bottom && ix > x && ix < right - h / 2 {
                            c = Some('_');
                        }
                    }
                    "callout" => {
                        let tail = h / 2;
                        let corners = [
                            ((x, y), '┌'),
                            ((right, y), '┐'),
                            ((x, bottom - tail), '└'),
                            ((right, bottom - tail), '┘'),
                        ];
                        if let Some((_, v)) = corners.iter().rev().find(|(p, _)| *p == (ix, iy)) {
                            c = Some(*v);
                        } else if (iy == y || iy == bottom - tail) && ix > x && ix < right {
                            c = Some('─');
                        } else if (ix == x || ix == right) && iy > y && iy < bottom - tail {
                            c = Some('│');
                        } else if ix == right - (tail + 2) && iy > bottom - tail {
                            c = Some('│');
                        } else if iy > bottom - tail && rx + ry == w {
                            c = Some('╱');
                        }
                    }
                    "stored_data" => {
                        let offset = (h + 1) / 2;
                        if iy == y + offset - 1 && ix == x {
                            c = Some('│');
                        } else if ix < x + offset {
                            if iy < y + offset && rx + ry == offset - 1 {
                                c = Some('╱');
                            } else if iy >= y + offset && (rx - ry).abs() == offset - 1 {
                                c = Some('╲');
                            }
                        } else if iy == y && ix < right {
                            c = Some('‾');
                        } else if iy == bottom && ix < right {
                            c = Some('_');
                        } else if ix > right - offset {
                            if iy == y + offset - 1 && ix == right - (offset - 1) {
                                c = Some('│');
                            } else if rx + ry == w - 1 {
                                c = Some('╱');
                            } else if (rx - ry).abs() == (w - h).abs() {
                                c = Some('╲');
                            }
                        }
                    }
                    "person" => {
                        let body = h - 2;
                        let hw = if w % 2 == 1 { 3 } else { 2 };
                        let offset = (w - hw) / 2;
                        if iy == bottom {
                            c = Some('‾');
                        } else if iy >= y + 2 && iy < bottom {
                            if rx + ry == body {
                                c = Some('╱');
                            } else if rx - (ry - 2) - 1 == (w - (h - 2)).abs() {
                                c = Some('╲');
                            } else if iy == y + 2 && ix >= x + body - 1 && ix <= right - (body - 1)
                            {
                                c = Some('‾');
                            }
                        } else if iy < y + 2 {
                            if iy == y && ix >= x + offset && ix <= right - offset {
                                c = Some('‾');
                            }
                            if iy == y + 1 && ix >= x + offset && ix <= right - offset {
                                c = Some('_');
                            }
                            if (iy == y && ix == x + offset - 1)
                                || (iy == y + 1 && ix == right - offset + 1)
                            {
                                c = Some('╱');
                            }
                            if (iy == y + 1 && ix == x + offset - 1)
                                || (iy == y && ix == right - offset + 1)
                            {
                                c = Some('╲');
                            }
                        }
                    }
                    "document" => {
                        let (n, left, right_curve) = document_curves.as_ref().unwrap();
                        if (ix == x || ix == right) && iy == y {
                            c = Some(if ix == right { '┐' } else { '┌' });
                        } else if iy == y && ix > x && ix < right {
                            c = Some('─');
                        } else if (ix == x || ix == right) && iy > y && iy < bottom {
                            c = Some('│');
                        } else if iy == bottom
                            && ix > x
                            && rx <= *n
                            && rx - 1 >= 0
                            && (rx - 1) < left.len() as i64
                        {
                            c = Some(left[(rx - 1) as usize]);
                        } else if iy == bottom - 1
                            && rx > *n
                            && ix < right
                            && rx - w / 2 < right_curve.len() as i64
                        {
                            c = right_curve.get((rx - w / 2) as usize).copied();
                        }
                    }
                    _ => unreachable!(),
                }
                if let Some(c) = c {
                    if kind == "document" && matches!(c, 'â' | '`') {
                        canvas.set(ix, iy, c);
                    } else {
                        canvas.paint(ix, iy, c);
                    }
                }
            }
        }
    }
    if kind == "cylinder" {
        if !label.is_empty() {
            canvas.label(
                x + (w - label.len() as i64) / 2,
                label_y(y + 1, bottom, h, position),
                label,
            );
        }
    } else if matches!(kind, "diamond" | "step" | "callout" | "document") {
        if !label.is_empty() {
            let height = match kind {
                "callout" => (h + 1) / 2,
                "document" => h - 2,
                _ => h,
            };
            canvas.label(
                x + (w - label.len() as i64) / 2,
                label_y(y, bottom, height, position),
                label,
            );
        }
    } else {
        shape_label(canvas, x, y, bottom, w, h, label, position);
    }
    Ok(())
}

/// Render a decoded public target using source ASCII artist semantics.
/// The source RenderOpts.Scale is unused; its artist retains SCALE=1.
pub fn render(diagram: &TargetDiagram, mode: &str, _scale: Option<f64>) -> Result<String, String> {
    let data = serde_json::to_value(diagram).map_err(|e| e.to_string())?;
    let shapes = values(&data["shapes"]);
    let connections = values(&data["connections"]);
    let mut bounds = diagram.nested_bounding_box()?;
    for connection in connections {
        let route = values(&connection["route"]);
        let label = text(connection, "label");
        let mut include = |left: f64, top: f64, right: f64, bottom: f64| {
            bounds[0] = bounds[0].min(left.trunc());
            bounds[1] = bounds[1].min(top.trunc());
            bounds[2] = bounds[2].max(right.trunc());
            bounds[3] = bounds[3].max(bottom.trunc());
        };
        if !label.is_empty() && route.len() > 1 {
            let (mut largest, mut best_x) = (0.0, 0.0);
            for points in route.windows(2) {
                let dx = (number(&points[0], "x") - number(&points[1], "x")).abs();
                let dy = (number(&points[0], "y") - number(&points[1], "y")).abs();
                let diff = dx.max(dy);
                if diff > largest {
                    largest = diff;
                    best_x = number(&points[0], "x");
                    if diff == dx {
                        best_x += (number(&points[1], "x") - number(&points[0], "x")).signum()
                            * diff
                            / 2.0;
                    }
                }
            }
            let mid_y = (number(&route[0], "y") + number(route.last().unwrap(), "y")) / 2.0;
            include(
                best_x - label.len() as f64 / 2.0 * FW,
                mid_y - FH,
                best_x + label.len() as f64 / 2.0 * FW,
                mid_y + FH,
            );
        }
        for (key, point) in [("srcLabel", route.first()), ("dstLabel", route.last())] {
            if !connection[key].is_null() {
                if let Some(point) = point {
                    let x = number(point, "x");
                    let y = number(point, "y");
                    let length = text(&connection[key], "label").len() as f64 * FW;
                    include(x - length, y - FH, x + length, y + FH);
                }
            }
        }
    }
    let offset_x = (-bounds[0]).max(0.0);
    let offset_y = (-bounds[1]).max(0.0);
    let padding = shapes
        .iter()
        .map(|s| text(s, "label").len())
        .max()
        .unwrap_or(0) as i64
        + 2;
    let mut canvas = Canvas::new(
        round((bounds[2] - bounds[0]).ceil() / FW) + padding + 1,
        round((bounds[3] - bounds[1]).ceil() / FH) + padding + 1,
        Chars {
            standard: mode == "standard",
        },
    )?;
    for shape in shapes {
        let (px, py, w, h) = (
            number(&shape["pos"], "x"),
            number(&shape["pos"], "y"),
            number(shape, "width"),
            number(shape, "height"),
        );
        let preserve_width = connections.iter().any(|c| {
            let route = values(&c["route"]);
            [route.first(), route.last()]
                .into_iter()
                .flatten()
                .any(|p| {
                    (number(p, "x") - (px + w)).abs() < FW / 2.0
                        && number(p, "y") >= py
                        && number(p, "y") <= py + h
                })
        });
        let preserve_height = connections.iter().any(|c| {
            values(&c["route"]).windows(2).any(|p| {
                (number(&p[0], "y") - number(&p[1], "y")).abs() < 0.1
                    && (number(&p[0], "y") - py).abs() < FH
                    && number(&p[0], "x").min(number(&p[1], "x")) <= px + w
                    && number(&p[0], "x").max(number(&p[1], "x")) >= px
            })
        });
        let mut width = w;
        let label = text(shape, "label");
        let available = round(w / FW) - label.len() as i64;
        if preserve_width && !label.is_empty() && available >= 2 && available % 2 == 1 {
            width += FW.trunc();
        }
        let multiple = shape["multiple"] == Value::Bool(true);
        let offset = if multiple { 10.0 } else { 0.0 };
        draw_shape(
            &mut canvas,
            shape,
            px + offset_x - offset,
            py + offset_y,
            width + offset,
            h + offset,
            preserve_height,
        )?;
    }
    for with_arrows in [false, true] {
        for connection in connections {
            let arrows =
                text(connection, "srcArrow") != "none" || text(connection, "dstArrow") != "none";
            if arrows == with_arrows {
                draw_route(
                    &mut canvas,
                    connection,
                    shapes,
                    connections,
                    offset_x,
                    offset_y,
                )?;
            }
        }
    }
    Ok(canvas.finish())
}

#[cfg(test)]
mod source_contract {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn complete_original_go_target_ascii_outcomes() {
        let rows: Vec<Value> = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/target-ascii-source-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.len(), 1142);
        for row in rows {
            let target: TargetDiagram = serde_json::from_value(row["diagram"].clone()).unwrap();
            let output = render(
                &target.without_children(),
                row["options"]["asciiMode"].as_str().unwrap_or("extended"),
                row["options"]["scale"].as_f64(),
            )
            .unwrap();
            assert_eq!(output, row["output"].as_str().unwrap(), "{}", row["name"]);
        }
    }
}
