// Copyright 2017-2022 The MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Translated from pinned mathjax-go v0.1.0 internal/layout/lengths.go.
#[derive(Clone, Copy, Debug)]
enum DimensionKind {
    Relative,
    Absolute,
    SizeRelative,
}
#[derive(Clone, Copy, Debug)]
pub(super) struct Dimension {
    value: f64,
    kind: DimensionKind,
}
pub(super) type Minimums = (Dimension, Dimension);
impl Dimension {
    pub(super) const fn relative(value: f64) -> Self {
        Self {
            value,
            kind: DimensionKind::Relative,
        }
    }
    pub(super) fn em(self, size: f64, scale: f64) -> f64 {
        match self.kind {
            DimensionKind::Relative => self.value,
            DimensionKind::Absolute => self.value / scale,
            DimensionKind::SizeRelative => self.value * size,
        }
    }
    pub(super) fn minimum(self) -> f64 {
        self.em(1.0, 1.0)
    }
}
// Original Length2Em prefix parsing and default renderer pxPerEm=8/0.442.
pub(super) fn dimension(raw: &str) -> Dimension {
    dimension_with_em(raw, 8.0 / 0.442)
}

// TeX commands may resolve absolute lengths before the renderer exists.
pub(super) fn dimension_with_em(raw: &str, em: f64) -> Dimension {
    match raw {
        "thin" => return Dimension::relative(0.04),
        "medium" => return Dimension::relative(0.06),
        "thick" => return Dimension::relative(0.1),
        "normal" => return Dimension::relative(1.0),
        "big" => return Dimension::relative(2.0),
        "small" => return Dimension::relative(std::f64::consts::FRAC_1_SQRT_2),
        "infinity" => return Dimension::relative(1_000_000.0),
        _ => {}
    }
    for (i, name) in [
        "veryverythinmathspace",
        "verythinmathspace",
        "thinmathspace",
        "mediummathspace",
        "thickmathspace",
        "verythickmathspace",
        "veryverythickmathspace",
    ]
    .iter()
    .enumerate()
    {
        if raw == *name {
            return Dimension::relative((i + 1) as f64 / 18.0);
        }
        if raw.strip_prefix("negative") == Some(*name) {
            return Dimension::relative(-(i as f64 + 1.0) / 18.0);
        }
    }
    let raw = raw.trim_start();
    let chars: Vec<_> = raw.chars().collect();
    let mut i = 0;
    if chars.first().is_some_and(|c| matches!(c, '+' | '-')) {
        i += 1;
    }
    let mut digits = 0;
    while chars.get(i).is_some_and(char::is_ascii_digit) {
        i += 1;
        digits += 1;
    }
    if chars.get(i) == Some(&'.') {
        i += 1;
        while chars.get(i).is_some_and(char::is_ascii_digit) {
            i += 1;
            digits += 1;
        }
    }
    let number = if digits == 0 {
        i = 0;
        1.0
    } else {
        chars[..i]
            .iter()
            .collect::<String>()
            .parse::<f64>()
            .unwrap_or(1.0)
    };
    let rest: String = chars[i..].iter().collect();
    let factor = if rest.starts_with("pt") {
        0.1
    } else if rest.starts_with("ex") {
        0.431
    } else if rest.starts_with("mu") {
        1.0 / 18.0
    } else if rest.starts_with("px") {
        1.0
    } else if rest.starts_with("pc") {
        1.2
    } else if rest.starts_with("in") {
        96.0
    } else if rest.starts_with("mm") {
        96.0 / 25.4
    } else if rest.starts_with("cm") {
        96.0 / 2.54
    } else if rest.starts_with('%') {
        0.01
    } else {
        1.0
    };
    let kind = if ["px", "in", "mm", "cm"]
        .iter()
        .any(|unit| rest.starts_with(unit))
    {
        DimensionKind::Absolute
    } else if ["pt", "ex", "mu", "pc", "em"]
        .iter()
        .any(|unit| rest.starts_with(unit))
    {
        DimensionKind::Relative
    } else {
        DimensionKind::SizeRelative
    };
    let value = match kind {
        DimensionKind::Absolute => number * factor / em,
        DimensionKind::SizeRelative if rest.starts_with('%') => number / 100.0,
        _ => number * factor,
    };
    Dimension { value, kind }
}
