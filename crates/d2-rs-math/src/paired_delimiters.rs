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
}
fn substitute(source: &str, args: &[String]) -> Result<String, String> {
    let mut result = String::new();
    let mut chars = source.chars();
    while let Some(c) = chars.next() {
        if c != '#' {
            result.push(c);
            continue;
        }
        match chars.next() {
            Some('#') => result.push('#'),
            Some(c @ '1'..='9') if (c as usize - '1' as usize) < args.len() => {
                result.push_str(&args[c as usize - '1' as usize]);
            }
            _ => return Err("Illegal macro parameter reference".into()),
        }
    }
    Ok(result)
}
impl Parser {
    pub(super) fn paired_definition(&self, name: &str) -> Option<Definition> {
        // The source's simple macro map and AMS tag handlers precede mathtools.
        if matches!(
            name,
            "dfrac"
                | "tfrac"
                | "binom"
                | "dbinom"
                | "tbinom"
                | "pmod"
                | "mod"
                | "bmod"
                | "iff"
                | "implies"
                | "impliedby"
                | "not"
                | "overset"
                | "underset"
                | "substack"
                | "Bra"
                | "Ket"
                | "Braket"
                | "Set"
                | "tag"
                | "notag"
                | "nonumber"
                | "label"
                | "ref"
                | "refeq"
                | "eqref"
        ) {
            return None;
        }
        self.ams.borrow().paired_delimiters.get(name).cloned()
    }
    pub(super) fn declare_paired_delimiter(&mut self, name: &str) -> Option<Atom> {
        let read = |p: &mut Self| -> Result<(), String> {
            let raw = p.padded_raw_argument(name)?;
            let raw = raw.trim();
            let missing = || format!("\\{name} must be followed by a control sequence");
            let Some(raw) = raw.strip_prefix('\\') else {
                return Err(missing());
            };
            let mut sub = Parser {
                chars: raw.chars().collect(),
                index: 0,
                ams: p.ams.clone(),
                splice_inferred: false,
                normal_font: p.normal_font,
                calligraphic_font: p.calligraphic_font,
                multi_letter_font: false,
                cd_minimums: p.cd_minimums,
            };
            let cs = sub.capture_control_sequence();
            if sub.index != sub.chars.len() {
                return Err(missing());
            }
            let mut arguments = 1;
            let is_x = name.contains('X');
            let is_xpp = name.contains("XPP");
            if is_x {
                let count = p.padded_brackets()?;
                if !count.is_empty() {
                    arguments = count.trim().parse::<i64>().map_err(|_| {
                        format!(
                            "Invalid number {}",
                            d2_rs_core::quote_go_bytes(count.as_bytes())
                        )
                    })?;
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
                p.padded_brackets()?
            };
            // Like Go make([]string, arguments), a negative count is invalid at
            // invocation, rather than at declaration.
            let count = usize::try_from(definition.arguments)
                .expect("negative paired delimiter argument count");
            let mut args = Vec::with_capacity(count);
            for _ in 0..count {
                args.push(p.padded_raw_argument(name)?);
            }
            let pre = substitute(pre, &args)?;
            let body = substitute(body, &args)?;
            let post = substitute(post, &args)?;
            let (left, right, middle) = if star {
                ("\\left".into(), "\\right".into(), "\\middle".into())
            } else if !size.is_empty() {
                (
                    format!("\\{size}l"),
                    format!("\\{size}r"),
                    format!("\\{size}"),
                )
            } else {
                (String::new(), String::new(), String::new())
            };
            Ok([
                pre.as_str(),
                &left,
                &definition.open,
                &body.replace("\\delimsize", &middle),
                &right,
                &definition.close,
                &post,
            ]
            .concat())
        };
        let expansion = match expand(self) {
            Ok(v) => v,
            Err(e) => return Some(Atom::ErrorDisplay(e)),
        };
        let atom = self.padded_parse_string(&expansion)?;
        if cd::first_error(&atom).is_some() {
            return Some(atom);
        }
        self.splice_inferred = true;
        Some(match atom {
            Atom::Sequence(_) => atom,
            a => Atom::Sequence(vec![a]),
        })
    }
}
