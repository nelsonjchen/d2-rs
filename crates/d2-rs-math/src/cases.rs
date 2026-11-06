// Copyright (c) 2018-2022 MathJax Consortium
// SPDX-License-Identifier: Apache-2.0
// Pinned tex/cases_handlers.go numCasesTextEnd.
// Cases.Entry scans physical text before normal command dispatch resumes.
// A control sequence inside braces cannot close the environment or the cell.
pub(super) fn text_end(source: &[char], start: usize) -> Result<usize, String> {
    let mut braces = 0usize;
    let mut i = start;
    while i < source.len() {
        match source[i] {
            '{' => {
                braces += 1;
                i += 1;
            }
            '}' => {
                if braces == 0 {
                    return Ok(i);
                }
                braces -= 1;
                i += 1;
            }
            '&' if braces == 0 => {
                return Err("Extra alignment tab in text for numcase environment".into());
            }
            '\\' if braces == 0 => {
                let Some(&next) = source.get(i + 1) else {
                    return Err("NumCases text scanner has no control-sequence token".into());
                };
                let mut j = i + 1;
                while source.get(j).is_some_and(char::is_ascii_alphabetic) {
                    j += 1;
                }
                if j == i + 1 {
                    if matches!(next, '\n' | '\r' | '\u{2028}' | '\u{2029}') {
                        return Err("NumCases text scanner has no control-sequence token".into());
                    }
                    j += 1;
                }
                let command: String = source[i + 1..j].iter().collect();
                if matches!(command.as_str(), "\\" | "cr" | "end" | "label") {
                    return Ok(i);
                }
                // Source resumes on the last command character. In
                // particular an escaped brace is still a brace to this scan.
                i = if next.is_ascii_alphabetic() {
                    j - 1
                } else {
                    i + 1
                };
            }
            _ => i += 1,
        }
    }
    Ok(source.len())
}

#[cfg(test)]
mod tests {
    use crate::render;

    #[test]
    fn cases_text_owns_braced_commands_and_reports_missing_environment_first() {
        for name in ["numcases", "subnumcases"] {
            let missing =
                render(&format!("\\begin{{{name}}}{{x}}x&{{missing\\end{{{name}}}")).unwrap();
            assert!(missing.contains(&format!("data-mjx-error=\"Missing \\end{{{name}}}\"")));
            let extra = render(&format!(
                "\\begin{{{name}}}{{x}}x&text&extra\\end{{{name}}}"
            ))
            .unwrap();
            assert!(extra.contains("Extra alignment tab in text for numcase environment"));
        }
    }
}
