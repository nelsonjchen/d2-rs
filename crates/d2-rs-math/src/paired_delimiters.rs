// Copyright (c) 2020-2022 MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Source: tex/mathtools_handlers_commands.go paired delimiter handlers;
// newcommand.go readCSNameArgument/substituteArguments; commands.go parseExpansion.
use super::*;
const SEPARATOR: &str = "\0mathtools-pair\0";
#[derive(Clone)]
pub(super) struct Definition {
    open: String,
    close: String,
    body: String,
    arguments: i64,
    substitute_zero_args: bool,
}
// ParseUtil.addArgs preserves control words across each expansion boundary.
pub(super) fn add_args(left: &mut String, right: &str) -> Result<(), String> {
    if right
        .as_bytes()
        .first()
        .is_some_and(u8::is_ascii_alphabetic)
    {
        let bytes = left.as_bytes();
        let mut i = bytes.len();
        while i > 0 && bytes[i - 1].is_ascii_alphabetic() {
            i -= 1;
        }
        if i < bytes.len() {
            let end = i;
            while i > 0 && bytes[i - 1] == b'\\' {
                i -= 1;
            }
            if (end - i) % 2 == 1 {
                left.push(' ');
            }
        }
    }
    if left.encode_utf16().count() + right.encode_utf16().count() > 5120 {
        return Err(
            "MathJax internal buffer size exceeded; is there a recursive macro call?".into(),
        );
    }
    left.push_str(right);
    Ok(())
}
pub(super) fn substitute(source: &str, args: &[String]) -> Result<String, String> {
    let mut result = String::new();
    let mut text = String::new();
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                text.push(c);
                if let Some(c) = chars.next() {
                    text.push(c);
                }
            }
            '#' => match chars.next() {
                Some('#') => text.push('#'),
                Some(c @ '1'..='9') if (c as usize - '1' as usize) < args.len() => {
                    add_args(&mut result, &text)?;
                    add_args(&mut result, &args[c as usize - '1' as usize])?;
                    text.clear();
                }
                _ => return Err("Illegal macro parameter reference".into()),
            },
            _ => text.push(c),
        }
    }
    add_args(&mut result, &text)?;
    Ok(result)
}
impl Parser {
    pub(super) fn paired_definition(&self, name: &str) -> Option<Definition> {
        self.ams.borrow().paired_delimiters.get(name).cloned()
    }
    pub(super) fn declare_paired_delimiter(&mut self, name: &str) -> Option<Atom> {
        let read = |p: &mut Self| -> Result<(), String> {
            let raw = p.padded_raw_argument(name)?;
            let mut cs = raw.trim().to_owned();
            if cs.ends_with('\\') && raw.ends_with(' ') {
                cs.push(' ');
            }
            let cs = cs.strip_prefix('\\').unwrap_or(&cs);
            let valid = !cs.is_empty()
                && (cs.chars().all(|c| c.is_ascii_alphabetic())
                    || (cs.chars().count() == 1
                        && cs.chars().all(|c| {
                            c as u32 <= 0xffff
                                && !matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
                        })));
            if !valid {
                return Err(format!("Illegal control sequence name for \\{name}"));
            }
            let cs = cs.to_owned();
            let mut arguments = 1;
            let mut substitute_zero_args = false;
            let is_x = name.contains('X');
            let is_xpp = name.contains("XPP");
            if is_x {
                let present = p.chars[p.index..].iter().find(|c| !c.is_whitespace()) == Some(&'[');
                let count = p.padded_brackets()?;
                if present {
                    arguments = 0;
                }
                if !count.is_empty() {
                    let count = count.trim();
                    if count.is_empty() || !count.chars().all(|c| c.is_ascii_digit()) {
                        return Err(format!(
                            "Illegal number of parameters specified in \\{name}"
                        ));
                    }
                    arguments = count.parse::<i64>().map_err(|_| {
                        format!(
                            "Invalid number {}",
                            d2_rs_core::quote_go_bytes(count.as_bytes())
                        )
                    })?;
                    substitute_zero_args = arguments == 0;
                }
            }
            let pre = if is_xpp {
                p.padded_raw_argument(name)?
            } else {
                String::new()
            };
            let open = p.padded_raw_argument(name)?;
            let close = p.padded_raw_argument(name)?;
            let post = if is_xpp {
                p.padded_raw_argument(name)?
            } else {
                String::new()
            };
            let body = if is_x {
                p.padded_raw_argument(name)?
            } else {
                "#1".into()
            };
            p.ams.borrow_mut().paired_delimiters.insert(
                cs,
                Definition {
                    open,
                    close,
                    body: [pre.as_str(), SEPARATOR, &body, SEPARATOR, &post].concat(),
                    arguments,
                    substitute_zero_args,
                },
            );
            Ok(())
        };
        match read(self) {
            Ok(()) => {
                self.splice_inferred = true;
                Some(Atom::Sequence(Vec::new()))
            }
            Err(e) => Some(Atom::ErrorDisplay(e)),
        }
    }
    pub(super) fn paired_delimiter(&mut self, name: &str, definition: Definition) -> Option<Atom> {
        let expand = |p: &mut Self| -> Result<String, String> {
            let parts = definition.body.split(SEPARATOR).collect::<Vec<_>>();
            let (pre, body, post) = if parts.len() == 3 {
                (parts[0], parts[1], parts[2])
            } else {
                ("", definition.body.as_str(), "")
            };
            while p.chars.get(p.index).is_some_and(|c| c.is_whitespace()) {
                p.index += 1;
            }
            let star = p.chars.get(p.index) == Some(&'*');
            if star {
                p.index += 1;
            }
            let size = if star {
                String::new()
            } else {
                p.padded_brackets().map_err(|e| {
                    if e == "Could not find closing ']' for argument" {
                        format!("Could not find closing ']' for argument to \\{name}")
                    } else {
                        e
                    }
                })?
            };
            // Like Go make([]string, arguments), a negative count is invalid at
            // invocation, rather than at declaration.
            let count = usize::try_from(definition.arguments)
                .expect("negative paired delimiter argument count");
            let mut args = Vec::with_capacity(count);
            for _ in 0..count {
                args.push(p.padded_raw_argument(name)?);
            }
            let substitute = |s: &str| {
                if count > 0 || definition.substitute_zero_args {
                    substitute(s, &args)
                } else {
                    Ok(s.to_owned())
                }
            };
            let pre = substitute(pre)?;
            let body = substitute(body)?;
            let post = substitute(post)?;
            let (left, right, middle) = if star {
                ("\\left".into(), "\\right".into(), "\\middle".into())
            } else if !size.is_empty() {
                (format!("{size}l"), format!("{size}r"), size.clone())
            } else {
                (String::new(), String::new(), String::new())
            };
            let mut expansion = String::new();
            for part in [
                pre.as_str(),
                &left,
                &definition.open,
                &body.replace("\\delimsize", &middle),
                &right,
                &definition.close,
                &post,
                &p.chars[p.index..].iter().collect::<String>(),
            ] {
                add_args(&mut expansion, part)?;
            }
            Ok(expansion)
        };
        let expansion = match expand(self) {
            Ok(v) => v,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        self.chars = expansion.chars().collect();
        self.index = 0;
        self.ams.borrow_mut().paired_macro_count += 1;
        if self.ams.borrow().paired_macro_count > 1000 {
            return Some(Atom::ErrorDisplay("MathJax maximum macro substitution count exceeded; is here a recursive macro call?".into()));
        }
        self.splice_inferred = true;
        Some(Atom::Sequence(Vec::new()))
    }
}
