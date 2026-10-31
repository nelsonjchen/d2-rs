// SPDX-License-Identifier: MPL-2.0
// Source: D2 v0.9.0 d2renderers/d2svgimport/path.go and d2scene/path.go.
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Command {
    pub kind: u8,
    pub points: [[f64; 2]; 3],
    pub radii: [f64; 2],
    pub rotation: f64,
    pub large_arc: bool,
    pub sweep: bool,
}

/// Parse bounded SVG path groups into absolute scene commands, retaining the
/// source count even when an identical-endpoint arc emits no geometry.
pub fn parse(
    source: &str,
    data: &str,
    max_bytes: usize,
    max_commands: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(Vec<Command>, usize), String> {
    context(cancel)?;
    if max_bytes == 0 || max_commands == 0 {
        return Err("d2svgimport: path limits must be positive".to_owned());
    }
    let source = if source.is_empty() {
        "SVG asset"
    } else {
        source
    };
    if data.len() > max_bytes {
        return Err(format!(
            "d2svgimport: {source} path data is {} bytes, exceeding limit {max_bytes}",
            data.len()
        ));
    }
    Parser {
        source,
        data: data.as_bytes(),
        offset: 0,
        maximum: max_commands,
        count: 0,
        current: [0.; 2],
        start: [0.; 2],
        have_current: false,
        previous: 0,
        cubic: [0.; 2],
        quad: [0.; 2],
        explicit: false,
        commands: Vec::new(),
        cancel,
    }
    .run()
}

fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => {
            Err("context deadline exceeded".to_owned())
        }
        Some(_) => Err("context canceled".to_owned()),
        None => Ok(()),
    }
}
fn command(c: u8) -> bool {
    b"MmZzLlHhVvCcSsQqTtAa".contains(&c)
}
fn space(c: u8) -> bool {
    b" \t\n\r\x0c".contains(&c)
}
fn reflect(control: [f64; 2], around: [f64; 2]) -> [f64; 2] {
    [2. * around[0] - control[0], 2. * around[1] - control[1]]
}
fn rotation_radians(value: f64) -> f64 {
    libm::remainder(value, 360.) / 180. * std::f64::consts::PI
}
struct Parser<'a> {
    source: &'a str,
    data: &'a [u8],
    offset: usize,
    maximum: usize,
    count: usize,
    current: [f64; 2],
    start: [f64; 2],
    have_current: bool,
    previous: u8,
    cubic: [f64; 2],
    quad: [f64; 2],
    explicit: bool,
    commands: Vec<Command>,
    cancel: Option<&'a d2_rs_core::CancellationToken>,
}
impl Parser<'_> {
    fn error_at(&self, offset: usize, reason: impl std::fmt::Display) -> String {
        format!("d2svgimport: {} path byte {offset}: {reason}", self.source)
    }
    fn error(&self, reason: impl std::fmt::Display) -> String {
        self.error_at(self.offset, reason)
    }
    fn byte(&self) -> Option<u8> {
        self.data.get(self.offset).copied()
    }
    fn whitespace(&mut self) -> Result<(), String> {
        while self.byte().is_some_and(space) {
            self.offset += 1;
            if self.offset & 4095 == 0 {
                context(self.cancel)?;
            }
        }
        context(self.cancel)
    }
    fn separator(&mut self) -> Result<(), String> {
        self.whitespace()?;
        if self.byte() == Some(b',') {
            self.offset += 1;
            self.whitespace()?;
            if self.byte() == Some(b',') {
                return Err(self.error("repeated comma in path data"));
            }
        }
        Ok(())
    }
    fn digits(&mut self) -> Result<usize, String> {
        let start = self.offset;
        while self.byte().is_some_and(|b| b.is_ascii_digit()) {
            self.offset += 1;
            if self.offset & 4095 == 0 {
                context(self.cancel)?;
            }
        }
        Ok(self.offset - start)
    }
    fn number(&mut self) -> Result<f64, String> {
        if self.explicit {
            self.explicit = false;
            self.whitespace()?;
            if self.byte() == Some(b',') {
                return Err(self.error("comma is not allowed before a command's first argument"));
            }
        } else {
            self.separator()?;
        }
        let start = self.offset;
        if matches!(self.byte(), Some(b'+' | b'-')) {
            self.offset += 1;
        }
        let mut digits = self.digits()?;
        if self.byte() == Some(b'.') {
            self.offset += 1;
            digits += self.digits()?;
        }
        if digits == 0 {
            return Err(self.error_at(start, "expected a finite number"));
        }
        if matches!(self.byte(), Some(b'e' | b'E')) {
            let exponent = self.offset;
            self.offset += 1;
            if matches!(self.byte(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            if self.digits()? == 0 {
                return Err(self.error_at(exponent, "number exponent has no digits"));
            }
        }
        context(self.cancel)?;
        let raw = std::str::from_utf8(&self.data[start..self.offset]).unwrap();
        d2_rs_core::parse_d2_float(raw)
            .filter(|n| n.is_finite())
            .ok_or_else(|| self.error_at(start, "expected a finite number"))
    }
    fn numbers<const N: usize>(&mut self) -> Result<[f64; N], String> {
        let mut values = [0.; N];
        for value in &mut values {
            *value = self.number()?;
        }
        Ok(values)
    }
    fn flag(&mut self) -> Result<bool, String> {
        self.separator()?;
        let value = match self.byte() {
            Some(b'0') => false,
            Some(b'1') => true,
            _ => return Err(self.error("arc flag must be 0 or 1")),
        };
        self.offset += 1;
        Ok(value)
    }
    fn charge(&mut self) -> Result<(), String> {
        if self.count >= self.maximum {
            return Err(self.error(format!("path command count exceeds limit {}", self.maximum)));
        }
        self.count += 1;
        context(self.cancel)
    }
    fn append(&mut self, node: Command) -> Result<(), String> {
        if !node
            .points
            .iter()
            .flatten()
            .chain(node.radii.iter())
            .chain(std::iter::once(&node.rotation))
            .all(|n| n.is_finite())
        {
            return Err(self.error("path command has a non-finite derived coordinate"));
        }
        self.commands.push(node);
        context(self.cancel)
    }
    fn point(&self, pair: [f64; 2], relative: bool) -> [f64; 2] {
        if relative {
            [pair[0] + self.current[0], pair[1] + self.current[1]]
        } else {
            pair
        }
    }
    fn group(&mut self, command: u8) -> Result<(), String> {
        context(self.cancel)?;
        let upper = command.to_ascii_uppercase();
        let relative = command.is_ascii_lowercase();
        if upper != b'M' && !self.have_current {
            return Err(self.error(format!("{} command appears before a move", command as char)));
        }
        let mut node = Command::default();
        match upper {
            b'M' | b'L' | b'T' => {
                let pair = self.numbers()?;
                let end = self.point(pair, relative);
                if upper == b'T' {
                    node.kind = 2;
                    node.points[0] = if matches!(self.previous.to_ascii_uppercase(), b'Q' | b'T') {
                        reflect(self.quad, self.current)
                    } else {
                        self.current
                    };
                    node.points[1] = end;
                    self.append(node)?;
                    self.quad = self.commands.last().unwrap().points[0];
                    self.cubic = [0.; 2];
                } else {
                    node.kind = u8::from(upper == b'L');
                    node.points[0] = end;
                    self.append(node)?;
                    if upper == b'M' {
                        self.start = end;
                        self.have_current = true;
                    }
                    self.quad = [0.; 2];
                    self.cubic = [0.; 2];
                }
                self.current = end;
            }
            b'H' | b'V' => {
                let mut value = self.number()?;
                let index = usize::from(upper == b'V');
                if relative {
                    value += self.current[index];
                }
                let mut end = self.current;
                end[index] = value;
                node.kind = 1;
                node.points[0] = end;
                self.append(node)?;
                self.current = end;
                self.cubic = [0.; 2];
                self.quad = [0.; 2];
            }
            b'C' => {
                let values: [f64; 6] = self.numbers()?;
                node.kind = 3;
                for (point, pair) in node.points.iter_mut().zip(values.chunks_exact(2)) {
                    *point = self.point([pair[0], pair[1]], relative);
                }
                let end = node.points[2];
                let control = node.points[1];
                self.append(node)?;
                self.current = end;
                self.cubic = control;
                self.quad = [0.; 2];
            }
            b'S' | b'Q' => {
                let values: [f64; 4] = self.numbers()?;
                let control = self.point([values[0], values[1]], relative);
                let end = self.point([values[2], values[3]], relative);
                if upper == b'S' {
                    node.kind = 3;
                    node.points[0] = if matches!(self.previous.to_ascii_uppercase(), b'C' | b'S') {
                        reflect(self.cubic, self.current)
                    } else {
                        self.current
                    };
                    node.points[1] = control;
                    node.points[2] = end;
                    self.append(node)?;
                    self.cubic = control;
                    self.quad = [0.; 2];
                } else {
                    node.kind = 2;
                    node.points[0] = control;
                    node.points[1] = end;
                    self.append(node)?;
                    self.quad = control;
                    self.cubic = [0.; 2];
                }
                self.current = end;
            }
            b'A' => {
                let [rx, ry, rotation] = self.numbers()?;
                let large_arc = self.flag()?;
                let sweep = self.flag()?;
                let pair = self.numbers()?;
                let end = self.point(pair, relative);
                if rx < 0. || ry < 0. {
                    return Err(self.error("arc radii must be non-negative"));
                }
                if end != self.current {
                    node.points[0] = end;
                    if rx == 0. || ry == 0. {
                        node.kind = 1;
                    } else {
                        node.kind = 4;
                        node.radii = [rx, ry];
                        node.rotation = rotation_radians(rotation);
                        node.large_arc = large_arc;
                        node.sweep = sweep;
                    }
                    self.append(node)?;
                }
                self.current = end;
                self.cubic = [0.; 2];
                self.quad = [0.; 2];
            }
            _ => return Err(self.error(format!("unsupported path command {:?}", command as char))),
        }
        self.previous = command;
        Ok(())
    }
    fn run(mut self) -> Result<(Vec<Command>, usize), String> {
        let mut active = 0;
        loop {
            self.whitespace()?;
            let Some(byte) = self.byte() else {
                return Ok((self.commands, self.count));
            };
            self.explicit = false;
            if command(byte) {
                active = byte;
                self.offset += 1;
                self.explicit = true;
                if matches!(active, b'Z' | b'z') {
                    if !self.have_current {
                        return Err(self.error("close command appears before a move"));
                    }
                    self.charge()?;
                    self.append(Command {
                        kind: 5,
                        ..Command::default()
                    })?;
                    self.current = self.start;
                    self.previous = active;
                    active = 0;
                    continue;
                }
            } else if active == 0 {
                return Err(self.error("expected a path command"));
            }
            self.charge()?;
            self.group(active)?;
            active = match active {
                b'M' => b'L',
                b'm' => b'l',
                other => other,
            };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_path_commands_counts_and_errors() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-path-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let result = parse(
                "math",
                row["data"].as_str().unwrap(),
                row["max_bytes"].as_u64().unwrap() as usize,
                row["max_commands"].as_u64().unwrap() as usize,
                None,
            );
            assert_eq!(
                result.as_ref().err().map(String::as_str),
                row["error"].as_str(),
                "{row}"
            );
            if let Ok((commands, count)) = result {
                assert_eq!(count as u64, row["count"].as_u64().unwrap(), "{row}");
                let expected = row["commands"].as_array().unwrap();
                assert_eq!(commands.len(), expected.len(), "{row}");
                for (actual, expected) in commands.iter().zip(expected) {
                    let expected_command: Command =
                        serde_json::from_value(expected.clone()).unwrap();
                    assert_eq!(actual, &expected_command, "{row}");
                    let bits: Vec<u64> = actual
                        .points
                        .iter()
                        .flatten()
                        .chain(actual.radii.iter())
                        .chain(std::iter::once(&actual.rotation))
                        .map(|n| n.to_bits())
                        .collect();
                    assert_eq!(
                        serde_json::to_value(bits).unwrap(),
                        expected["bits"],
                        "{row}"
                    );
                }
            }
        }
    }
    #[test]
    fn context_precedes_limits() {
        let token = d2_rs_core::CancellationToken::new();
        token.cancel();
        assert_eq!(
            parse("", "M0 0", 0, 0, Some(&token)).unwrap_err(),
            "context canceled"
        );
        let token = d2_rs_core::CancellationToken::new();
        token.expire();
        assert_eq!(
            parse("", "M0 0", 0, 0, Some(&token)).unwrap_err(),
            "context deadline exceeded"
        );
    }
}
