// SPDX-License-Identifier: MPL-2.0
// D2 v0.9.0 code_shape.go collectCodeTokens/countCodeTokenPieces.
use d2_rs_core::{CancellationToken, LayoutError};
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CodeWorkError {
    RawTokens(usize),
    ExpandedPieces(usize),
    Cancelled,
    DeadlineExceeded,
}
impl std::fmt::Display for CodeWorkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::RawTokens(n) => write!(f, "raw lexer token count exceeds limit {n}"),
            Self::ExpandedPieces(n) => {
                write!(f, "newline-expanded token piece count exceeds limit {n}")
            }
            Self::Cancelled => f.write_str("context canceled"),
            Self::DeadlineExceeded => f.write_str("context deadline exceeded"),
        }
    }
}
impl std::error::Error for CodeWorkError {}
#[derive(Clone, Debug)]
pub struct CodeBudget {
    max_raw: usize,
    max_pieces: usize,
    raw: usize,
    pieces: usize,
}
impl Default for CodeBudget {
    fn default() -> Self {
        Self::for_max_nodes(0)
    }
}
impl CodeBudget {
    /// Mirrors newCodeTokenBudget; this does not perform the separate scene-node admission check.
    pub fn for_max_nodes(max_nodes: usize) -> Self {
        let limit = if max_nodes == 0 {
            100_000
        } else {
            max_nodes.min(100_000)
        };
        Self {
            max_raw: limit,
            max_pieces: limit,
            raw: 0,
            pieces: 0,
        }
    }
    pub fn counts(&self) -> (usize, usize) {
        (self.raw, self.pieces)
    }
}
pub(super) struct Control<'a> {
    budget: Option<&'a mut CodeBudget>,
    cancel: Option<&'a CancellationToken>,
    pub error: Option<CodeWorkError>,
}
impl<'a> Control<'a> {
    pub fn unlimited() -> Self {
        Self {
            budget: None,
            cancel: None,
            error: None,
        }
    }
    pub fn bounded(budget: &'a mut CodeBudget, cancel: Option<&'a CancellationToken>) -> Self {
        Self {
            budget: Some(budget),
            cancel,
            error: None,
        }
    }
    // Chroma's DelegatingLexer builds intermediate streams before the caller
    // collects final tokens. Preserve cancellation, charge only the final stream.
    pub(super) fn unmetered(&self) -> Self {
        Self {
            budget: None,
            cancel: self.cancel,
            error: None,
        }
    }
    pub fn check(&mut self) -> bool {
        if self.error.is_some() {
            return false;
        }
        if let Some(error) = self.cancel.and_then(CancellationToken::error) {
            self.error = Some(if matches!(error, LayoutError::DeadlineExceeded) {
                CodeWorkError::DeadlineExceeded
            } else {
                CodeWorkError::Cancelled
            });
            return false;
        }
        true
    }
    pub fn accept(&mut self, value: &str) -> bool {
        if !self.check() {
            return false;
        }
        let Some(budget) = self.budget.as_ref() else {
            return true;
        };
        if budget.raw >= budget.max_raw {
            self.error = Some(CodeWorkError::RawTokens(budget.max_raw));
            return false;
        }
        let remaining = budget.max_pieces - budget.pieces;
        let limit = budget.max_pieces;
        let mut pieces = 1;
        if remaining < pieces {
            self.error = Some(CodeWorkError::ExpandedPieces(limit));
            return false;
        }
        for (i, byte) in value.bytes().enumerate() {
            if i & 4095 == 0 && !self.check() {
                return false;
            }
            if byte == b'\n' {
                pieces += 1;
                if pieces > remaining {
                    self.error = Some(CodeWorkError::ExpandedPieces(limit));
                    return false;
                }
            }
        }
        if !self.check() {
            return false;
        }
        let budget = self.budget.as_mut().unwrap();
        budget.raw += 1;
        budget.pieces += pieces;
        true
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "oracle-fixtures")]
    use super::*;
    #[test]
    fn custom_delegation_charges_only_the_complete_final_stream() {
        use super::{CodeBudget, CodeWorkError};
        for (language, source) in [
            ("svelte", "<!-- a comment containing {braces} -->"),
            (
                "http",
                "GET / HTTP/1.1\nContent-Type: application/json\n\n{\"n\": 42}",
            ),
            ("rst", ".. code:: python\n\n    n = 42\n    print(n)\n\n"),
            ("haxe", "#if js\nvar x = 1;\n#else\nvar x = 2;\n#end"),
            ("markdown", "```haxe\nvar x = 42;\n```"),
            ("raku", "say qq{hello $x {1+2}};"),
            ("markdown", "```raku\nmy $x = 42;\n```"),
        ] {
            let expected = crate::code::tokenize(language, source).unwrap();
            let pieces = expected
                .iter()
                .map(|t| 1 + t.value.bytes().filter(|&b| b == b'\n').count())
                .sum();
            let mut budget = CodeBudget::for_max_nodes(pieces);
            let actual = crate::code::tokenize_bounded(language, source, &mut budget, None)
                .unwrap()
                .unwrap();
            assert_eq!(actual, expected, "{language}");
            assert_eq!(budget.counts(), (actual.len(), pieces), "{language}");
            let mut budget = CodeBudget::for_max_nodes(pieces - 1);
            assert!(
                matches!(
                    crate::code::tokenize_bounded(language, source, &mut budget, None),
                    Err(CodeWorkError::RawTokens(_) | CodeWorkError::ExpandedPieces(_))
                ),
                "{language}"
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn original_collector_counts_errors_and_context_causes() {
        let rows: serde_json::Value = serde_json::from_str(d2_rs_testdata::oracle_fixture!(
            "tests/fixtures/code-token-budget-v090.json"
        ))
        .unwrap();
        assert_eq!(rows.as_array().unwrap().len(), 264);
        for row in rows.as_array().unwrap() {
            let r = &row["request"];
            let token = CancellationToken::new();
            let cancel = r["cancel"].as_str().unwrap();
            if cancel == "before" {
                token.cancel();
            } else if cancel == "deadline_before" {
                token.expire();
            }
            let mut budget = CodeBudget::for_max_nodes(r["max_nodes"].as_u64().unwrap() as usize);
            let mut control = Control::bounded(&mut budget, Some(&token));
            let mut returned = 0;
            if control.check() {
                let values = r["values"].as_array().unwrap();
                let count = values.len() * r["sequence_repeat"].as_u64().unwrap() as usize;
                for i in 0..count {
                    if i == 1 && cancel == "after_first" {
                        token.cancel();
                    }
                    let v = &values[i % values.len()];
                    let value = v["unit"]
                        .as_str()
                        .unwrap()
                        .repeat(v["repeat"].as_u64().unwrap() as usize);
                    if !control.accept(&value) {
                        break;
                    }
                    returned += 1;
                }
            }
            let error = control
                .error
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default();
            if !error.is_empty() {
                returned = 0;
            }
            drop(control);
            assert_eq!(error, row["error"].as_str().unwrap(), "{}", r["name"]);
            assert_eq!(
                budget.counts(),
                (
                    row["raw_tokens"].as_u64().unwrap() as usize,
                    row["expanded_pieces"].as_u64().unwrap() as usize
                ),
                "{}",
                r["name"]
            );
            assert_eq!(
                returned,
                row["returned_tokens"].as_u64().unwrap() as usize,
                "{}",
                r["name"]
            );
        }
    }
    #[cfg(feature = "oracle-fixtures")]
    #[test]
    fn bounded_lexers_keep_complete_original_streams_and_shared_delegation_counts() {
        let corpora = [
            d2_rs_testdata::oracle_fixture!("tests/fixtures/chroma-go-template-tokens.json"),
            d2_rs_testdata::oracle_fixture!("tests/fixtures/python-tokens-v227.json"),
            d2_rs_testdata::oracle_fixture!("tests/fixtures/markdown-yaml-tokens-v227.json"),
            d2_rs_testdata::oracle_fixture!("tests/fixtures/javascript-json-tokens-v227.json"),
        ];
        for corpus in corpora {
            let rows: serde_json::Value = serde_json::from_str(corpus).unwrap();
            for row in rows.as_array().unwrap() {
                let mut budget = CodeBudget::default();
                let tokens = super::super::tokenize_bounded(
                    row["language"].as_str().unwrap(),
                    row["source"].as_str().unwrap(),
                    &mut budget,
                    None,
                )
                .unwrap()
                .unwrap();
                let actual: Vec<_> = tokens
                    .iter()
                    .map(|t| serde_json::json!({"kind":t.kind,"value":t.value}))
                    .collect();
                assert_eq!(
                    serde_json::json!(actual),
                    row["tokens"],
                    "{}",
                    row["source"]
                );
                assert_eq!(
                    budget.counts(),
                    (
                        tokens.len(),
                        tokens
                            .iter()
                            .map(|t| 1 + t.value.bytes().filter(|b| *b == b'\n').count())
                            .sum()
                    )
                );
            }
        }
        let mut budget = CodeBudget::for_max_nodes(3);
        assert!(
            super::super::tokenize_bounded("go", "a b", &mut budget, None)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            super::super::tokenize_bounded("go", "c", &mut budget, None).unwrap_err(),
            CodeWorkError::RawTokens(3)
        );
        assert_eq!(budget.counts(), (3, 3));
        let token = CancellationToken::new();
        token.cancel();
        let mut budget = CodeBudget::default();
        assert_eq!(
            super::super::tokenize_bounded(
                "markdown",
                "```js\nlet x=1;\n```",
                &mut budget,
                Some(&token)
            )
            .unwrap_err(),
            CodeWorkError::Cancelled
        );
        assert_eq!(budget.counts(), (0, 0));
    }
}
