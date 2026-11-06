// SPDX-License-Identifier: MPL-2.0
// Source: D2 d2svgimport/transform.go, d2scene/geometry.go.
type Matrix = [f64; 6];
const IDENTITY: Matrix = [1., 0., 0., 1., 0., 0.];
fn context(cancel: Option<&d2_rs_core::CancellationToken>) -> Result<(), String> {
    match cancel.and_then(d2_rs_core::CancellationToken::error) {
        Some(d2_rs_core::LayoutError::DeadlineExceeded) => {
            Err("context deadline exceeded".to_owned())
        }
        Some(_) => Err("context canceled".to_owned()),
        None => Ok(()),
    }
}
pub fn mul(m: Matrix, r: Matrix) -> Matrix {
    #[cfg(not(target_arch = "aarch64"))]
    return [
        m[0] * r[0] + m[2] * r[1],
        m[1] * r[0] + m[3] * r[1],
        m[0] * r[2] + m[2] * r[3],
        m[1] * r[2] + m[3] * r[3],
        m[0] * r[4] + m[2] * r[5] + m[4],
        m[1] * r[4] + m[3] * r[5] + m[5],
    ];
    #[cfg(target_arch = "aarch64")]
    [
        m[0].mul_add(r[0], m[2] * r[1]),
        m[1].mul_add(r[0], m[3] * r[1]),
        m[0].mul_add(r[2], m[2] * r[3]),
        m[1].mul_add(r[2], m[3] * r[3]),
        m[0].mul_add(r[4], m[2] * r[5]) + m[4],
        m[1].mul_add(r[4], m[3] * r[5]) + m[5],
    ]
}
fn function(name: &str, v: &[f64]) -> Result<Matrix, String> {
    let arity = |expected| {
        format!(
            "transform function {name:?} has {} arguments; expected {expected}",
            v.len()
        )
    };
    let radians = || libm::remainder(v[0], 360.) / 180. * std::f64::consts::PI;
    match name {
        "matrix" => {
            if v.len() != 6 {
                return Err(arity("6"));
            }
            Ok(v.try_into().unwrap())
        }
        "translate" | "scale" => {
            if v.len() != 1 && v.len() != 2 {
                return Err(arity("1 or 2"));
            }
            let mut matrix = IDENTITY;
            if name == "translate" {
                matrix[4] = v[0];
                matrix[5] = v.get(1).copied().unwrap_or(0.);
            } else {
                matrix[0] = v[0];
                matrix[3] = v.get(1).copied().unwrap_or(v[0]);
            }
            Ok(matrix)
        }
        "rotate" => {
            if v.len() != 1 && v.len() != 3 {
                return Err(arity("1 or 3"));
            }
            let (s, c) = super::svg_trig::sincos(radians());
            let rotation = [c, s, -s, c, 0., 0.];
            if v.len() == 3 {
                Ok(mul(
                    mul([1., 0., 0., 1., v[1], v[2]], rotation),
                    [1., 0., 0., 1., -v[1], -v[2]],
                ))
            } else {
                Ok(rotation)
            }
        }
        "skewX" | "skewY" => {
            if v.len() != 1 {
                return Err(arity("1"));
            }
            let mut matrix = IDENTITY;
            matrix[if name == "skewX" { 2 } else { 1 }] = super::svg_trig::tan(radians());
            Ok(matrix)
        }
        _ => Err(format!("unsupported transform function {name:?}")),
    }
}
/// Source bounded transform parser: matrix is A,B,C,D,E,F, with parsed function count.
pub fn parse(
    source: &str,
    value: &str,
    max_bytes: usize,
    max_functions: usize,
    cancel: Option<&d2_rs_core::CancellationToken>,
) -> Result<(Matrix, usize), String> {
    context(cancel)?;
    if max_bytes == 0 || max_functions == 0 {
        return Err("d2svgimport: transform limits must be positive".to_owned());
    }
    let source = if source.is_empty() {
        "SVG asset"
    } else {
        source
    };
    if value.len() > max_bytes {
        return Err(format!(
            "d2svgimport: {source} transform is {} bytes, exceeding limit {max_bytes}",
            value.len()
        ));
    }
    Parser {
        source,
        data: value.as_bytes(),
        offset: 0,
        maximum: max_functions,
        count: 0,
        cancel,
    }
    .run()
}
struct Parser<'a> {
    source: &'a str,
    data: &'a [u8],
    offset: usize,
    maximum: usize,
    count: usize,
    cancel: Option<&'a d2_rs_core::CancellationToken>,
}
impl Parser<'_> {
    fn error_at(&self, offset: usize, reason: impl std::fmt::Display) -> String {
        format!(
            "d2svgimport: {} transform byte {offset}: {reason}",
            self.source
        )
    }
    fn error(&self, reason: impl std::fmt::Display) -> String {
        self.error_at(self.offset, reason)
    }
    fn byte(&self) -> Option<u8> {
        self.data.get(self.offset).copied()
    }
    fn whitespace(&mut self) -> Result<(), String> {
        while self.byte().is_some_and(|b| b" \t\n\r\x0c".contains(&b)) {
            self.offset += 1;
            if self.offset & 4095 == 0 {
                context(self.cancel)?;
            }
        }
        context(self.cancel)
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
            return Err(self.error_at(start, "expected a finite transform number"));
        }
        if matches!(self.byte(), Some(b'e' | b'E')) {
            let exponent = self.offset;
            self.offset += 1;
            if matches!(self.byte(), Some(b'+' | b'-')) {
                self.offset += 1;
            }
            if self.digits()? == 0 {
                return Err(self.error_at(exponent, "transform exponent has no digits"));
            }
        }
        context(self.cancel)?;
        d2_rs_core::parse_d2_float(std::str::from_utf8(&self.data[start..self.offset]).unwrap())
            .filter(|n| n.is_finite())
            .ok_or_else(|| self.error_at(start, "expected a finite transform number"))
    }
    fn arguments(&mut self) -> Result<Vec<f64>, String> {
        let mut values = Vec::with_capacity(6);
        loop {
            let start = self.offset;
            self.whitespace()?;
            let spaced = self.offset != start;
            let Some(byte) = self.byte() else {
                return Err(self.error("unterminated transform function"));
            };
            if byte == b')' {
                self.offset += 1;
                return Ok(values);
            }
            if !values.is_empty() && byte == b',' {
                self.offset += 1;
                self.whitespace()?;
                if matches!(self.byte(), None | Some(b',' | b')')) {
                    return Err(self.error("transform has an empty argument"));
                }
            } else if values.is_empty() && byte == b',' {
                return Err(self.error("transform has an empty first argument"));
            } else if !values.is_empty() && !spaced {
                return Err(
                    self.error("transform arguments must be separated by whitespace or a comma")
                );
            }
            let value = self.number()?;
            if values.len() >= 6 {
                return Err(self.error("transform has more than 6 arguments"));
            }
            values.push(value);
        }
    }
    fn between(&mut self) -> Result<(), String> {
        let start = self.offset;
        self.whitespace()?;
        if self.offset == self.data.len() {
            return Ok(());
        }
        if self.byte() == Some(b',') {
            self.offset += 1;
            self.whitespace()?;
            if matches!(self.byte(), None | Some(b',')) {
                return Err(self.error("transform has an empty function"));
            }
            return Ok(());
        }
        if self.offset == start {
            return Err(
                self.error("transform functions must be separated by whitespace or a comma")
            );
        }
        Ok(())
    }
    fn run(mut self) -> Result<(Matrix, usize), String> {
        let mut matrix = IDENTITY;
        self.whitespace()?;
        if self.offset == self.data.len() {
            return Ok((matrix, 0));
        }
        loop {
            let start = self.offset;
            while self.byte().is_some_and(|b| b.is_ascii_alphabetic()) {
                self.offset += 1;
                if self.offset & 4095 == 0 {
                    context(self.cancel)?;
                }
            }
            if start == self.offset {
                return Err(self.error("expected a transform function"));
            }
            let name = std::str::from_utf8(&self.data[start..self.offset])
                .unwrap()
                .to_owned();
            self.whitespace()?;
            if self.byte() != Some(b'(') {
                return Err(self.error(format!("transform function {name:?} is missing '('")));
            }
            self.offset += 1;
            let values = self.arguments()?;
            self.count += 1;
            if self.count > self.maximum {
                return Err(self.error(format!(
                    "transform function count exceeds limit {}",
                    self.maximum
                )));
            }
            let next = function(&name, &values).map_err(|e| self.error(e))?;
            matrix = mul(matrix, next);
            if !matrix.iter().all(|n| n.is_finite()) {
                return Err(self.error("composed transform is non-finite"));
            }
            self.between()?;
            if self.offset == self.data.len() {
                return Ok((matrix, self.count));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_matrices_function_counts_and_errors() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/svg-transform-v090.json"
        ))
        .unwrap();
        for row in rows.as_array().unwrap() {
            let result = parse(
                "math",
                row["value"].as_str().unwrap(),
                row["max_bytes"].as_u64().unwrap() as usize,
                row["max_functions"].as_u64().unwrap() as usize,
                None,
            );
            assert_eq!(
                result.as_ref().err().map(String::as_str),
                row["error"].as_str(),
                "{row}"
            );
            if let Ok((matrix, count)) = result {
                assert_eq!(count as u64, row["count"].as_u64().unwrap(), "{row}");
                let expected: Matrix = serde_json::from_value(row["matrix"].clone()).unwrap();
                assert_eq!(matrix, expected, "{row}");
                assert_eq!(
                    serde_json::to_value(matrix.map(f64::to_bits)).unwrap(),
                    row["bits"],
                    "{row}"
                );
            }
        }
    }
    #[test]
    fn context_before_limits() {
        let token = d2_rs_core::CancellationToken::new();
        token.cancel();
        assert_eq!(
            parse("", "", 0, 0, Some(&token)).unwrap_err(),
            "context canceled"
        );
        let token = d2_rs_core::CancellationToken::new();
        token.expire();
        assert_eq!(
            parse("", "", 0, 0, Some(&token)).unwrap_err(),
            "context deadline exceeded"
        );
    }
}
