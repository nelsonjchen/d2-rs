// SPDX-License-Identifier: MIT
// Native execution of pinned Chroma v2.27.0 lexers/embedded XML definitions.
// Sources: regexp.go LexerState.Iterator/Tokenise, mutators.go and emitters.go.
// Copyright 2017 Alec Thomas. Rust translation Copyright 2026 Nelson Chen.
use super::{Control, Token, push, xml_lexers};
use fancy_regex::{Regex, RegexBuilder};
use std::{collections::BTreeMap, sync::OnceLock};
#[path = "raku.rs"]
mod raku;
fn kind(name: &str) -> &'static str {
    super::chroma_style::token_name(name).unwrap_or_else(|| panic!("untranslated XML token {name}"))
}
/// Rewrites regexp2 (.NET-syntax) patterns into fancy-regex syntax.
/// Escape- and class-aware: an escaped backslash is copied verbatim, `\b`
/// inside a class stays a backspace, and characters that the Rust class
/// parser treats as nesting/set operators (`[`, `&`, `~`, a leading `]`)
/// are escaped inside classes where .NET reads them literally.
fn translate(pattern: &str) -> String {
    let word = r"[\p{L}\p{Mn}\p{Nd}\p{Pc}\u{200c}\u{200d}]";
    let not_word = r"[^\p{L}\p{Mn}\p{Nd}\p{Pc}\u{200c}\u{200d}]";
    let boundary = format!("(?:(?<={word})(?!{word})|(?<!{word})(?={word}))");
    let nonboundary = format!("(?:(?<={word})(?={word})|(?<!{word})(?!{word}))");
    let mut out = String::with_capacity(pattern.len());
    let chars: Vec<char> = pattern.chars().collect();
    let mut in_class = false;
    let mut class_start = false;
    let mut i = 0;
    while i < chars.len() {
        // regexp2 syntax/charclass.go charInCategories returns on the first
        // Word category, including its negative case. Later Nd is ignored.
        // Match only actual classes, never an escaped literal '['.
        if !in_class {
            let special = if chars[i..].starts_with(&['[', '^', '\\', 'W', '\\', 'd', ']']) {
                Some((7, word))
            } else if chars[i..].starts_with(&['[', '\\', 'W', '\\', 'd', ']']) {
                Some((6, not_word))
            } else {
                None
            };
            if let Some((length, replacement)) = special {
                out.push_str(replacement);
                i += length;
                continue;
            }
        }
        let c = chars[i];
        i += 1;
        let at_start = class_start;
        class_start = false;
        if c == '\\' {
            let Some(&next) = chars.get(i) else {
                out.push('\\');
                break;
            };
            i += 1;
            match next {
                // regexp2's Unicode word class: L, Mn, Nd, Pc and the joiners.
                'w' => out.push_str(word),
                'W' => out.push_str(not_word),
                'd' => out.push_str(r"\p{Nd}"),
                'b' if in_class => out.push_str(r"\x08"),
                'b' => out.push_str(&boundary),
                'B' => out.push_str(&nonboundary),
                other if !other.is_ascii_alphanumeric() => {
                    // regexp2's escaped punctuation is literal. In particular,
                    // fancy-regex gives \< and \> word-boundary semantics.
                    use std::fmt::Write;
                    write!(out, r"\x{{{:x}}}", other as u32).unwrap();
                }
                other => {
                    out.push('\\');
                    out.push(other);
                }
            }
            continue;
        }
        if in_class {
            match c {
                ']' if at_start => out.push_str(r"\]"),
                ']' => {
                    in_class = false;
                    out.push(']');
                }
                '^' if at_start && out.ends_with('[') => {
                    out.push('^');
                    class_start = true;
                }
                '[' | '&' | '~' => {
                    out.push('\\');
                    out.push(c);
                }
                _ => out.push(c),
            }
        } else {
            if c == '[' {
                in_class = true;
                class_start = true;
            }
            out.push(c);
        }
    }
    static OPTIONAL_ASSERTION: OnceLock<regex::Regex> = OnceLock::new();
    OPTIONAL_ASSERTION
        .get_or_init(|| regex::Regex::new(r"\(\?=([^()]*)\)\?").unwrap())
        .replace_all(&out, "(?:(?=$1)|)")
        .into_owned()
}

enum Emitter {
    Token(&'static str),
    Groups(Vec<Emitter>),
    Using(String),
    SelfState(String),
    RstCodeBlock,
    Raku(String),
    NamedGroups(BTreeMap<String, Emitter>),
    UsingByGroup {
        language: usize,
        code: usize,
        emitters: Vec<Emitter>,
    },
}
enum Mutator {
    Pop(usize),
    Push(Vec<String>),
    HaxePreproc,
    RakuBrackets(u8),
    RakuReplace(Box<raku::Replacement>),
}
struct Rule {
    // Compile lazily: a normal run attempts only a prefix of each state's rules.
    pattern: String,
    regex: OnceLock<Matcher>,
    emitter: Option<Emitter>,
    mutators: Vec<Mutator>,
}
// regexp2 evaluates lookbehind right-to-left, including captures. For the
// variable-length assertions with nested boundaries that fancy-regex cannot
// compile, execute the reversed expression over a reversed input instead.
enum Matcher {
    Direct(Regex),
    Behind {
        assertions: Vec<(Regex, bool)>,
        rest: Regex,
    },
}
fn expression(expr: &fancy_regex::Expr, reverse: bool) -> String {
    use fancy_regex::{Assertion as A, Expr as E, LookAround as L};
    match expr {
        E::Concat(items) => {
            let mut items = items.iter().collect::<Vec<_>>();
            if reverse {
                items.reverse();
            }
            items.into_iter().map(|e| expression(e, reverse)).collect()
        }
        E::Alt(items) => format!(
            "(?:{})",
            items
                .iter()
                .map(|e| expression(e, reverse))
                .collect::<Vec<_>>()
                .join("|")
        ),
        E::Group(inner) => format!("({})", expression(inner, reverse)),
        E::LookAround(inner, direction) => {
            let direction = if reverse {
                match direction {
                    L::LookAhead => L::LookBehind,
                    L::LookAheadNeg => L::LookBehindNeg,
                    L::LookBehind => L::LookAhead,
                    L::LookBehindNeg => L::LookAheadNeg,
                }
            } else {
                *direction
            };
            let prefix = match direction {
                L::LookAhead => "?=",
                L::LookAheadNeg => "?!",
                L::LookBehind => "?<=",
                L::LookBehindNeg => "?<!",
            };
            format!("({prefix}{})", expression(inner, reverse))
        }
        E::Repeat {
            child,
            lo,
            hi,
            greedy,
        } => format!(
            "(?:{}){{{},{}}}{}",
            expression(child, reverse),
            lo,
            if *hi == usize::MAX {
                String::new()
            } else {
                hi.to_string()
            },
            if *greedy { "" } else { "?" }
        ),
        E::Literal { val, casei } if reverse => {
            let literal = regex::escape(&val.chars().rev().collect::<String>());
            if *casei {
                format!("(?i:{literal})")
            } else {
                literal
            }
        }
        E::Assertion(a) => match (a, reverse) {
            (A::StartText, false) | (A::EndText, true) => r"\A".to_owned(),
            (A::EndText, false) | (A::StartText, true) => r"\z".to_owned(),
            (A::StartLine { .. }, false) | (A::EndLine { .. }, true) => "(?m:^)".to_owned(),
            (A::EndLine { .. }, false) | (A::StartLine { .. }, true) => "(?m:$)".to_owned(),
            (A::WordBoundary, _) => r"\b".to_owned(),
            (A::NotWordBoundary, _) => r"\B".to_owned(),
            _ => panic!("unsupported Chroma assertion {a:?}"),
        },
        E::ContinueFromPreviousMatchEnd => r"\G".to_owned(),
        _ => {
            let mut out = String::new();
            expr.to_str(&mut out, 0);
            out
        }
    }
}
impl Matcher {
    fn compile(pattern: &str) -> Result<Self, String> {
        let error = match RegexBuilder::new(pattern).build() {
            Ok(regex) => return Ok(Self::Direct(regex)),
            Err(error) => error.to_string(),
        };
        let tree = fancy_regex::Expr::parse_tree(pattern).map_err(|e| e.to_string())?;
        fn flatten(expr: fancy_regex::Expr, parts: &mut Vec<fancy_regex::Expr>) {
            if let fancy_regex::Expr::Concat(items) = expr {
                for item in items {
                    flatten(item, parts);
                }
            } else {
                parts.push(expr);
            }
        }
        let mut parts = Vec::new();
        flatten(tree.expr, &mut parts);
        if matches!(
            parts.first(),
            Some(fancy_regex::Expr::ContinueFromPreviousMatchEnd)
        ) {
            parts.remove(0);
        }
        let mut assertions = Vec::new();
        while matches!(
            parts.first(),
            Some(fancy_regex::Expr::LookAround(
                _,
                fancy_regex::LookAround::LookBehind | fancy_regex::LookAround::LookBehindNeg
            ))
        ) {
            let fancy_regex::Expr::LookAround(inner, direction) = parts.remove(0) else {
                unreachable!()
            };
            let regex = Regex::new(&format!(r"\G(?:{})", expression(&inner, true)))
                .map_err(|e| e.to_string())?;
            assertions.push((regex, direction == fancy_regex::LookAround::LookBehind));
        }
        if assertions.is_empty() {
            return Err(error);
        }
        let rest = Regex::new(&format!(
            r"\G(?:{})",
            expression(&fancy_regex::Expr::Concat(parts), false)
        ))
        .map_err(|e| e.to_string())?;
        Ok(Self::Behind { assertions, rest })
    }
    fn captures<'a>(
        &self,
        text: &'a str,
        offset: usize,
        reversed: &'a std::cell::OnceCell<String>,
    ) -> Option<(usize, Vec<std::borrow::Cow<'a, str>>)> {
        let mut prefix = Vec::new();
        let regex = match self {
            Self::Direct(regex) => regex,
            Self::Behind { assertions, rest } => {
                let backwards = reversed.get_or_init(|| text.chars().rev().collect());
                for (assertion, positive) in assertions {
                    let matched = assertion
                        .captures_from_pos(backwards.as_str(), text.len() - offset)
                        .ok()
                        .flatten();
                    if matched.is_some() != *positive {
                        return None;
                    }
                    if let Some(captures) = matched {
                        for i in 1..captures.len() {
                            prefix.push(std::borrow::Cow::Owned(
                                captures.get(i).map_or_else(String::new, |g| {
                                    g.as_str().chars().rev().collect()
                                }),
                            ));
                        }
                    }
                }
                rest
            }
        };
        let captures = regex.captures_from_pos(text, offset).ok().flatten()?;
        let matched = captures.get(0)?;
        if matched.start() != offset {
            return None;
        }
        let mut groups = vec![std::borrow::Cow::Borrowed(matched.as_str())];
        groups.extend(prefix);
        groups.extend(
            (1..captures.len())
                .map(|i| std::borrow::Cow::Borrowed(captures.get(i).map_or("", |g| g.as_str()))),
        );
        Some((matched.end(), groups))
    }
}
impl Rule {
    fn regex(&self) -> &Matcher {
        self.regex.get_or_init(|| {
            Matcher::compile(&self.pattern)
                .unwrap_or_else(|e| panic!("Chroma {}: {e}", self.pattern))
        })
    }
}
struct Compiled {
    states: BTreeMap<String, Vec<Rule>>,
    ensure_nl: bool,
    haxe_preproc: bool,
    raku_runtime: bool,
}
fn compile(source: &str) -> Compiled {
    // Go encoding/xml accepts repeated state attributes for Push/Combined.
    // Convert them to children without changing their order for roxmltree.
    static STATES: OnceLock<regex::Regex> = OnceLock::new();
    static ATTR: OnceLock<regex::Regex> = OnceLock::new();
    let states = STATES.get_or_init(|| {
        regex::Regex::new(r#"<(push|combined)((?:\s+state="[^"]*")*)\s*/>"#).unwrap()
    });
    let attributes = ATTR.get_or_init(|| regex::Regex::new(r#"state="([^"]*)""#).unwrap());
    let mut preserved = String::with_capacity(source.len());
    let mut quoted = false;
    for ch in source.chars() {
        if ch == '"' {
            quoted = !quoted;
        }
        match ch {
            '\n' if quoted => preserved.push_str("&#10;"),
            '\r' if quoted => preserved.push_str("&#13;"),
            '\t' if quoted => preserved.push_str("&#9;"),
            _ => preserved.push(ch),
        }
    }
    let xml = states.replace_all(&preserved, |caps: &regex::Captures<'_>| {
        let mut result = format!("<{}>", &caps[1]);
        for attr in attributes.captures_iter(&caps[2]) {
            result.push_str("<state name=\"");
            result.push_str(&attr[1]);
            result.push_str("\"/>");
        }
        result.push_str(&format!("</{}>", &caps[1]));
        result
    });
    let doc = roxmltree::Document::parse(&xml).expect("pinned Chroma XML");
    let enabled = |name| {
        doc.descendants()
            .any(|n| n.has_tag_name(name) && n.text() == Some("true"))
    };
    let flags = format!(
        "{}{}{}",
        if enabled("not_multiline") { "" } else { "m" },
        if enabled("case_insensitive") { "i" } else { "" },
        if enabled("dot_all") { "s" } else { "" }
    );
    fn expand<'a>(
        doc: &'a roxmltree::Document<'a>,
        state: &str,
        rules: &mut Vec<roxmltree::Node<'a, 'a>>,
    ) {
        let node = doc
            .descendants()
            .find(|n| {
                n.has_tag_name("state")
                    && n.attribute("name") == Some(state)
                    && n.parent().is_some_and(|p| p.has_tag_name("rules"))
            })
            .unwrap();
        for rule in node.children().filter(|n| n.has_tag_name("rule")) {
            if let Some(include) = rule.children().find(|n| n.has_tag_name("include")) {
                expand(doc, include.attribute("state").unwrap(), rules);
            } else {
                rules.push(rule);
            }
        }
    }
    fn emitter(n: roxmltree::Node<'_, '_>) -> Option<Emitter> {
        Some(match n.tag_name().name() {
            "token" => Emitter::Token(kind(n.attribute("type").unwrap())),
            "bygroups" => Emitter::Groups(n.children().filter_map(emitter).collect()),
            "using" => Emitter::Using(n.attribute("lexer").unwrap().to_owned()),
            "usingself" => Emitter::SelfState(n.attribute("state").unwrap().to_owned()),
            "rst-code-block" => Emitter::RstCodeBlock,
            "raku-emitter" => Emitter::Raku(n.attribute("name").unwrap().to_owned()),
            "raku-groups" => Emitter::NamedGroups(
                n.children()
                    .filter(|n| n.has_tag_name("group"))
                    .map(|n| {
                        (
                            n.attribute("name").unwrap().to_owned(),
                            n.children().find_map(emitter).unwrap(),
                        )
                    })
                    .collect(),
            ),
            "usingbygroup" => Emitter::UsingByGroup {
                language: n
                    .children()
                    .find(|n| n.has_tag_name("sublexer_name_group"))
                    .unwrap()
                    .text()
                    .unwrap()
                    .parse()
                    .unwrap(),
                code: n
                    .children()
                    .find(|n| n.has_tag_name("code_group"))
                    .unwrap()
                    .text()
                    .unwrap()
                    .parse()
                    .unwrap(),
                emitters: n
                    .children()
                    .find(|n| n.has_tag_name("emitters"))
                    .unwrap()
                    .children()
                    .filter_map(emitter)
                    .collect(),
            },
            _ => return None,
        })
    }
    fn mutations<'a>(
        n: roxmltree::Node<'a, 'a>,
        doc: &'a roxmltree::Document<'a>,
        combined: &mut BTreeMap<String, Vec<roxmltree::Node<'a, 'a>>>,
        out: &mut Vec<Mutator>,
    ) {
        let names = || {
            n.children()
                .filter(|n| n.has_tag_name("state"))
                .map(|n| n.attribute("name").unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        match n.tag_name().name() {
            "pop" => out.push(Mutator::Pop(
                n.attribute("depth").unwrap_or("0").parse().unwrap(),
            )),
            "push" => out.push(Mutator::Push(names())),
            "haxe-preproc" => out.push(Mutator::HaxePreproc),
            "raku-brackets" => out.push(Mutator::RakuBrackets(
                n.attribute("class").unwrap().parse().unwrap(),
            )),
            "raku-replace" => out.push(Mutator::RakuReplace(Box::new(
                raku::Replacement::from_xml(n),
            ))),
            "combined" => {
                let names = names();
                let name = format!("__combined_{}", names.join("__"));
                combined.entry(name.clone()).or_insert_with(|| {
                    let mut rows = Vec::new();
                    for state in &names {
                        expand(doc, state, &mut rows);
                    }
                    rows
                });
                out.push(Mutator::Push(vec![name]));
            }
            "rule" | "mutators" => {
                for child in n.children().filter(|n| n.is_element()) {
                    mutations(child, doc, combined, out);
                }
            }
            _ => {}
        }
    }
    let mut pending = BTreeMap::new();
    for state in doc
        .descendants()
        .filter(|n| n.has_tag_name("state") && n.parent().is_some_and(|p| p.has_tag_name("rules")))
    {
        let name = state.attribute("name").unwrap();
        let mut rows = Vec::new();
        expand(&doc, name, &mut rows);
        pending.insert(name.to_owned(), rows);
    }
    let mut states = BTreeMap::new();
    while let Some((name, rows)) = pending.pop_first() {
        if states.contains_key(&name) {
            continue;
        }
        let rules = rows
            .into_iter()
            .map(|row| {
                let pattern = translate(row.attribute("pattern").unwrap_or(""));
                let pattern = format!(r"(?{flags})\G(?:{pattern})");
                let emitter = row.children().find_map(emitter);
                let mut mutators = Vec::new();
                mutations(row, &doc, &mut pending, &mut mutators);
                Rule {
                    pattern,
                    regex: OnceLock::new(),
                    emitter,
                    mutators,
                }
            })
            .collect();
        states.insert(name, rules);
    }
    Compiled {
        states,
        ensure_nl: enabled("ensure_nl"),
        haxe_preproc: doc.descendants().any(|n| n.has_tag_name("haxe-preproc")),
        raku_runtime: doc.descendants().any(|n| n.has_tag_name("raku-brackets")),
    }
}
fn lexer(name: &str) -> Option<&'static Compiled> {
    if name == "PHTML" {
        // lexers/php.go: Rules.Rename changes only the state key, leaving
        // references to root intact, then Merge installs the HTML-scanning root.
        static PHTML: OnceLock<Compiled> = OnceLock::new();
        return Some(PHTML.get_or_init(|| {
            let source = include_str!("../../assets/chroma/lexers/php.xml")
                .replace(r#"<state name="root">"#, r#"<state name="php">"#)
                .replace("</rules>", r#"<state name="root">
                    <rule pattern="&lt;\?(php)?"><token type="CommentPreproc"/><push state="php"/></rule>
                    <rule pattern="[^&lt;]+"><token type="Other"/></rule>
                    <rule pattern="&lt;"><token type="Other"/></rule>
                </state></rules>"#);
            compile(&source)
        }));
    }
    static LEXERS: OnceLock<BTreeMap<&'static str, (&'static str, OnceLock<Compiled>)>> =
        OnceLock::new();
    let lexers = LEXERS.get_or_init(|| {
        xml_lexers::SOURCES
            .iter()
            .map(|&(name, source)| (name, (source, OnceLock::new())))
            .collect()
    });
    let (source, compiled) = lexers.get(name)?;
    Some(compiled.get_or_init(|| compile(source)))
}
pub(super) fn tokenize(source: &str, name: &str, control: &mut Control<'_>) -> Option<Vec<Token>> {
    named(source, name, false, control)
}
fn named(source: &str, name: &str, nested: bool, control: &mut Control<'_>) -> Option<Vec<Token>> {
    let (language, root) = match name {
        "ERB" => ("ERB", "HTML"),
        "PHTML" => ("PHTML", "HTML"),
        "Go HTML Template" => ("Go Template", "HTML"),
        "YAML+Jinja" => ("Django/Jinja", "YAML"),
        "Svelte" => ("Svelte", "HTML"),
        "HTTP" => return http(source, nested, control),
        _ => {
            let mut tokens = run(source, lexer(name)?, "root", nested, control)?;
            if matches!(name, "Common Lisp" | "EmacsLisp") {
                // lexers/cl.go and emacs.go TypeRemappingLexer: exact string
                // lookup, last table entry wins (even with case-insensitive rules).
                type Mappings = BTreeMap<String, BTreeMap<String, String>>;
                static MAPPINGS: OnceLock<Mappings> = OnceLock::new();
                let mappings = MAPPINGS.get_or_init(|| {
                    serde_json::from_str(include_str!("../../assets/chroma/lisp-mappings.json"))
                        .unwrap()
                });
                for token in &mut tokens {
                    if token.kind == "NameVariable" {
                        if let Some(mapped) = mappings[name].get(&token.value) {
                            token.kind = kind(mapped);
                        }
                    }
                }
            }
            return Some(tokens);
        }
    };
    delegating(source, language, root, nested, control)
}
// lexers/http.go httpBodyContentTyper.Tokenise: header values select the
// body lexer by MIME type, with structured-suffix fallback. Intermediate
// headers/body tokens are not collected by D2's final token budget.
fn http(source: &str, nested: bool, control: &mut Control<'_>) -> Option<Vec<Token>> {
    let mut intermediate = control.unmetered();
    let raw = run(source, lexer("HTTP")?, "root", nested, &mut intermediate)?;
    let mut content_type = String::new();
    let mut is_content_type = false;
    let mut tokens = Vec::new();
    for mut token in raw {
        if !control.check() {
            break;
        }
        match token.kind {
            "Name" if token.value.eq_ignore_ascii_case("content-type") => is_content_type = true,
            "Literal" if is_content_type => {
                is_content_type = false;
                content_type = token.value.trim().to_owned();
                if let Some(pos) = content_type.find(';').filter(|&pos| pos > 0) {
                    content_type = content_type[..pos].trim().to_owned();
                }
            }
            "Generic" if !content_type.is_empty() => {
                let mut language = super::registry::match_mime_type(&content_type);
                if language.is_none() {
                    if let Some(plus) = content_type.rfind('+') {
                        let slash = content_type.find('/').map_or(0, |pos| pos + 1);
                        content_type =
                            format!("{}{}", &content_type[..slash], &content_type[plus + 1..]);
                        language = super::registry::match_mime_type(&content_type);
                    }
                }
                if let Some(language) = language {
                    tokens.extend(delegate(&token.value, language, false, control)?);
                    continue;
                }
                token.kind = "Text";
            }
            _ => {}
        }
        push(&mut tokens, token.kind, &token.value, control);
    }
    Some(tokens)
}
fn coalesce(tokens: Vec<Token>) -> Vec<Token> {
    let mut result: Vec<Token> = Vec::new();
    for token in tokens.into_iter().filter(|t| !t.value.is_empty()) {
        if let Some(last) = result
            .last_mut()
            .filter(|t| t.kind == token.kind && t.value.len() < 8192)
        {
            last.value.push_str(&token.value);
        } else {
            result.push(token);
        }
    }
    result
}
// delegate.go DelegatingLexer.Tokenise: coalesce language tokens, lex the
// concatenated Other spans once, then interleave at original byte offsets.
fn delegating(
    source: &str,
    language: &str,
    root: &str,
    nested: bool,
    control: &mut Control<'_>,
) -> Option<Vec<Token>> {
    let mut intermediate = control.unmetered();
    let language_tokens = coalesce(run(
        source,
        lexer(language)?,
        "root",
        nested,
        &mut intermediate,
    )?);
    let mut others = String::new();
    let mut insertions: Vec<(usize, usize, Vec<Token>)> = Vec::new();
    let mut offset = 0;
    let mut was_other = true;
    for token in language_tokens {
        let length = token.value.len();
        if token.kind == "Other" {
            others.push_str(&token.value);
            was_other = true;
        } else {
            if was_other {
                insertions.push((offset, offset, Vec::new()));
            }
            let (_, end, tokens) = insertions.last_mut()?;
            *end = offset + length;
            tokens.push(token);
            was_other = false;
        }
        offset += length;
    }
    let raw = if insertions.is_empty() {
        delegate(source, root, nested, &mut intermediate)?
    } else {
        let root_tokens = coalesce(delegate(&others, root, nested, &mut intermediate)?);
        let mut root_tokens = root_tokens.into_iter();
        let mut token = root_tokens.next();
        let mut insertions = insertions.into_iter().peekable();
        let mut offset = 0;
        let mut out = Vec::new();
        while token.is_some() || insertions.peek().is_some() {
            if insertions
                .peek()
                .is_some_and(|i| token.as_ref().is_none_or(|t| i.0 < offset + t.value.len()))
            {
                let (start, end, tokens) = insertions.next()?;
                if let Some(mut current) = token.take() {
                    let length = start - offset;
                    if length > 0 {
                        let tail = current.value.split_off(length);
                        offset += current.value.len();
                        out.push(current.clone());
                        current.value = tail;
                    }
                    token = if current.value.is_empty() {
                        root_tokens.next()
                    } else {
                        Some(current)
                    };
                }
                out.extend(tokens);
                offset += end - start;
            } else {
                let current = token.take()?;
                offset += current.value.len();
                out.push(current);
                token = root_tokens.next();
            }
        }
        out
    };
    let mut tokens = Vec::new();
    for token in raw {
        push(&mut tokens, token.kind, &token.value, control);
    }
    Some(tokens)
}
fn delegate(
    source: &str,
    language: &str,
    nested: bool,
    control: &mut Control<'_>,
) -> Option<Vec<Token>> {
    let name = super::registry::get(language)?;
    // Retain the already-qualified Go/Python/Markdown/YAML implementations.
    match name {
        "Go" | "Go Template" | "Go Text Template" | "Python" | "markdown" | "YAML"
        | "plaintext" => super::tokenize_inner(language, source, control),
        _ => named(source, name, nested, control),
    }
}
impl Emitter {
    fn emit(
        &self,
        groups: &[&str],
        lexer: &Compiled,
        tokens: &mut Vec<Token>,
        control: &mut Control<'_>,
    ) -> Option<()> {
        if !control.check() {
            return None;
        }
        match self {
            Self::Raku(_) | Self::NamedGroups(_) => {
                return raku::emit(self, groups, &[], &BTreeMap::new(), lexer, tokens, control);
            }
            Self::Token(kind) => {
                // LexerState.Iterator discards Ignore, before the caller's budget.
                if *kind != "Ignore" {
                    push(tokens, kind, groups[0], control);
                }
            }
            Self::Groups(emitters) => {
                if emitters.len() + 1 != groups.len() {
                    push(tokens, "Error", groups[0], control);
                } else {
                    for (emitter, &group) in emitters.iter().zip(&groups[1..]) {
                        emitter.emit(&[group], lexer, tokens, control)?;
                    }
                }
            }
            Self::Using(language) => tokens.extend(delegate(groups[0], language, true, control)?),
            Self::SelfState(state) => tokens.extend(run(groups[0], lexer, state, true, control)?),
            Self::RstCodeBlock => {
                // lexers/rst.go rstCodeBlock: concatenate indentation, first
                // line and following lines before invoking the nested lexer.
                for (kind, value) in [
                    "Punctuation",
                    "Text",
                    "OperatorWord",
                    "Punctuation",
                    "Text",
                    "Keyword",
                    "Text",
                ]
                .into_iter()
                .zip(&groups[1..8])
                {
                    push(tokens, kind, value, control);
                }
                let code = groups[8..].concat();
                if super::registry::get(groups[6]).is_some() {
                    tokens.extend(delegate(&code, groups[6], false, control)?);
                } else {
                    push(tokens, "LiteralString", &code, control);
                }
            }
            Self::UsingByGroup {
                language,
                code,
                emitters,
            } => {
                if emitters.len() + 1 != groups.len() {
                    return None;
                }
                for (i, (emitter, &group)) in emitters.iter().zip(&groups[1..]).enumerate() {
                    if i + 1 == *code && super::registry::get(groups[*language]).is_some() {
                        tokens.extend(delegate(group, groups[*language], false, control)?);
                    } else {
                        emitter.emit(&[group], lexer, tokens, control)?;
                    }
                }
            }
        }
        Some(())
    }
}
fn run(
    source: &str,
    lexer: &Compiled,
    initial_state: &str,
    nested: bool,
    control: &mut Control<'_>,
) -> Option<Vec<Token>> {
    if lexer.raku_runtime {
        return raku::run(source, lexer, initial_state, nested, control);
    }
    if lexer.haxe_preproc {
        run_with_stack(
            source,
            lexer,
            initial_state,
            nested,
            control,
            super::haxe::Stack::new(initial_state),
        )
    } else {
        run_with_stack(
            source,
            lexer,
            initial_state,
            nested,
            control,
            vec![initial_state],
        )
    }
}
// Monomorphize the shared-slice stack only for Haxe. Other lexers retain the
// plain Vec operations and allocations they used before Haxe was supported.
pub(super) trait StateStack<'a> {
    fn len(&self) -> usize;
    fn last(&self) -> Option<&'a str>;
    fn push(&mut self, state: &'a str);
    fn truncate(&mut self, length: usize);
    fn reset(&mut self, state: &'a str);
    fn preproc(&mut self, _directive: &str) {
        unreachable!("Haxe mutator on a plain stack")
    }
}
impl<'a> StateStack<'a> for Vec<&'a str> {
    fn len(&self) -> usize {
        Vec::len(self)
    }
    fn last(&self) -> Option<&'a str> {
        self.as_slice().last().copied()
    }
    fn push(&mut self, state: &'a str) {
        Vec::push(self, state);
    }
    fn truncate(&mut self, length: usize) {
        Vec::truncate(self, length);
    }
    fn reset(&mut self, state: &'a str) {
        *self = vec![state];
    }
}
fn run_with_stack<'a>(
    source: &str,
    lexer: &'a Compiled,
    initial_state: &'a str,
    nested: bool,
    control: &mut Control<'_>,
    mut stack: impl StateStack<'a>,
) -> Option<Vec<Token>> {
    let end = source.len();
    let text = if !nested && lexer.ensure_nl && !source.ends_with('\n') {
        std::borrow::Cow::Owned(format!("{source}\n"))
    } else {
        std::borrow::Cow::Borrowed(source)
    };
    let reversed = std::cell::OnceCell::new();
    let mut offset = 0;
    let mut tokens = Vec::new();
    while offset < end && stack.len() != 0 && control.check() {
        let state = stack.last().unwrap();
        let mut selected = None;
        for rule in lexer.states.get(state)? {
            if !control.check() {
                return None;
            }
            // regexp.go matchRules treats a regexp2 match timeout as no match.
            if let Some((end, groups)) = rule.regex().captures(text.as_ref(), offset, &reversed) {
                selected = Some((rule, end, groups));
                break;
            }
        }
        if let Some((rule, end, captures)) = selected {
            offset = end;
            for mutator in &rule.mutators {
                match mutator {
                    Mutator::Pop(depth) => stack.truncate(stack.len().checked_sub(*depth)?),
                    Mutator::Push(states) if states.is_empty() => stack.push(state),
                    Mutator::Push(states) => {
                        for state in states {
                            if state == "#pop" {
                                stack.truncate(stack.len().checked_sub(1)?);
                            } else {
                                stack.push(state.as_str());
                            }
                        }
                    }
                    Mutator::HaxePreproc => stack.preproc(captures.get(2)?.as_ref()),
                    Mutator::RakuBrackets(_) | Mutator::RakuReplace(_) => {
                        unreachable!("Raku has a call-local rule table")
                    }
                }
            }
            if let Some(emitter) = &rule.emitter {
                let groups: Vec<_> = captures.iter().map(|g| g.as_ref()).collect();
                emitter.emit(&groups, lexer, &mut tokens, control)?;
            }
        } else {
            let ch = text[offset..].chars().next().unwrap();
            if ch == '\n' && state != initial_state {
                stack.reset(initial_state);
                continue;
            }
            let next = offset + ch.len_utf8();
            push(&mut tokens, "Error", &text[offset..next], control);
            offset = next;
        }
    }
    if stack.len() == 0 && offset != text.len() {
        push(&mut tokens, "Error", &text[offset..], control);
    }
    Some(tokens)
}
#[cfg(test)]
mod tests {
    #[test]
    fn every_pinned_rule_compiles() {
        let mut errors = Vec::new();
        for &(name, source) in super::xml_lexers::SOURCES {
            for (state, rules) in super::compile(source).states {
                for rule in rules {
                    if let Err(error) = super::Matcher::compile(&rule.pattern) {
                        errors.push(format!("{name}.{state}: {}: {error}", rule.pattern));
                    }
                }
            }
        }
        assert!(errors.is_empty(), "{}", errors.join("\n"));
    }
}
