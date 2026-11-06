// SPDX-License-Identifier: MPL-2.0
//! Native translation of pinned d2format. Output retains arbitrary AST bytes.
use d2_rs_core::ast::keywords::is_reserved_keyword;
use d2_rs_core::ast::*;

/// Format an AST node. Upstream mutates single-quoted raw spelling and synthetic
/// block-string ranges/values; shared nodes retain those same mutations here.
pub fn format(node: &Child) -> Vec<u8> {
    format_with_options(node, FormatOptions::default())
}

/// Per-call specials avoid global mutation while retaining upstream customization.
#[derive(Clone, Copy)]
pub struct FormatOptions<'a> {
    pub unquoted_key_specials: &'a str,
    pub unquoted_value_specials: &'a str,
}
impl Default for FormatOptions<'_> {
    fn default() -> Self {
        Self {
            unquoted_key_specials: UNQUOTED_KEY_SPECIALS,
            unquoted_value_specials: UNQUOTED_VALUE_SPECIALS,
        }
    }
}
pub fn format_with_options(node: &Child, options: FormatOptions<'_>) -> Vec<u8> {
    let mut printer = Printer {
        options,
        ..Default::default()
    };
    printer.node(node);
    printer.output
}
#[derive(Default)]
struct Printer<'a> {
    options: FormatOptions<'a>,
    output: Vec<u8>,
    indent: usize,
    in_key: bool,
}
impl Printer<'_> {
    fn put(&mut self, bytes: impl AsRef<[u8]>) {
        self.output.extend_from_slice(bytes.as_ref());
    }
    fn newline(&mut self) {
        self.put(b"\n");
        self.output.resize(self.output.len() + self.indent, b' ');
    }
    fn node(&mut self, node: &Child) {
        match node {
            Child::Null(_) => self.put("null"),
            Child::Suspension(n) => self.put(if n.read().value {
                "suspend"
            } else {
                "unsuspend"
            }),
            Child::Boolean(n) => self.put(if n.read().value { "true" } else { "false" }),
            Child::Number(n) => self.put(&n.read().raw),
            Child::Comment(n) => {
                let n = n.read();
                for (i, line) in n.value.split('\n').enumerate() {
                    if i > 0 {
                        self.newline();
                    }
                    self.put("#");
                    if !line.is_empty() {
                        self.put(" ");
                    }
                    self.put(line);
                }
            }
            Child::BlockComment(n) => {
                let n = n.read();
                self.put("\"\"\"");
                if n.range.one_line() {
                    self.put(" ");
                }
                self.lines(n.value.as_bytes(), n.range.one_line());
                if n.range.one_line() {
                    self.put(" ");
                } else {
                    self.newline();
                }
                self.put("\"\"\"");
            }
            Child::Unquoted(n) => self.interpolations(&n.read().value, false),
            Child::DoubleQuoted(n) => {
                self.put("\"");
                self.interpolations(&n.read().value, true);
                self.put("\"");
            }
            Child::SingleQuoted(n) => {
                let mut n = n.write();
                let value = escape(
                    &n.value.to_text(),
                    Escape::Single,
                    self.in_key,
                    self.options,
                );
                if n.raw.is_empty() {
                    n.raw = value.clone();
                }
                self.put("'");
                self.put(value);
                self.put("'");
            }
            Child::Block(n) => self.block(&mut n.write()),
            Child::Substitution(n) => {
                let n = n.read();
                if n.spread {
                    self.put("...");
                }
                self.put("${");
                self.path(&n.path);
                self.put("}");
            }
            Child::Import(n) => {
                let n = n.read();
                if n.spread {
                    self.put("...");
                }
                self.put("@");
                let pre = clean_path(&n.pre);
                if pre != "." {
                    self.put(pre);
                    self.put("/");
                }
                if let Some(path) = &n.path {
                    for (i, slot) in path.iter().enumerate() {
                        if i > 0 {
                            self.put(".");
                        }
                        let slot = slot.read();
                        let value = slot.as_ref().expect("nil import component").read();
                        if i == 0 {
                            self.node(
                                &raw_string_box_with_specials(
                                    clean_path(&value.scalar_string()),
                                    true,
                                    self.options.unquoted_key_specials,
                                    self.options.unquoted_value_specials,
                                )
                                .unbox()
                                .unwrap(),
                            );
                        } else {
                            self.optional(value.unbox());
                        }
                    }
                }
            }
            Child::KeyPath(n) => self.key(&n.read()),
            Child::Edge(n) => {
                let n = n.read();
                if let Some(src) = &n.src {
                    self.key(&src.read());
                    self.put(" ");
                }
                self.arrow(&n);
            }
            Child::EdgeIndex(n) => {
                let n = n.read();
                self.put("[");
                if n.glob {
                    self.put("*");
                } else {
                    self.put(n.int.as_ref().expect("nil edge index").read().to_string());
                }
                self.put("]");
            }
            Child::Key(n) => self.map_key(&n.read()),
            Child::Array(n) => self.array(&n.read()),
            Child::Map(n) => self.map(&n.read()),
        }
    }
    fn optional(&mut self, node: Option<Child>) {
        if let Some(node) = node {
            self.node(&node);
        }
    }
    fn lines(&mut self, value: &[u8], one_line: bool) {
        for line in value.split(|b| *b == b'\n') {
            if !one_line {
                if line.is_empty() {
                    self.put("\n");
                } else {
                    self.newline();
                }
            }
            self.put(line);
        }
    }
    fn interpolations(&mut self, boxes: &Option<SharedSlice<InterpolationBox>>, double: bool) {
        let mut previous_sub = false;
        for b in boxes.iter().flat_map(SharedSlice::iter) {
            let b = b.read();
            if let Some(sub) = &b.substitution {
                self.node(&Child::Substitution(sub.clone()));
                previous_sub = true;
                continue;
            }
            let value = b.string.as_ref().expect("nil interpolation string").read();
            let mut raw: ByteString = match &b.raw_string {
                None => escape(
                    &value.to_text(),
                    if double {
                        Escape::Double
                    } else {
                        Escape::Unquoted
                    },
                    self.in_key,
                    self.options,
                )
                .into(),
                Some(_) if previous_sub => value.clone(),
                Some(raw) => raw.read().clone().into(),
            };
            if !double && is_reserved_keyword(&crate::parser::go_lower(&raw.to_text())) {
                raw = crate::parser::go_lower(&raw.to_text()).into();
            }
            self.put(raw.as_bytes());
            previous_sub = false;
        }
    }
    fn block(&mut self, n: &mut BlockString) {
        // Go indexes the final byte, then string(byte) UTF-8-encodes its value.
        // For non-ASCII quotes this deliberately differs from repeating a rune.
        let mut quote = n.quote.as_bytes().to_vec();
        while contains(
            n.value.as_bytes(),
            &[b"|".as_slice(), quote.as_slice()].concat(),
        ) {
            let byte = quote.last().copied().unwrap_or(b'|');
            quote.extend_from_slice(char::from(byte).encode_utf8(&mut [0; 4]).as_bytes());
        }
        while contains(
            n.value.as_bytes(),
            &[quote.as_slice(), b"|".as_slice()].concat(),
        ) {
            let byte = *quote.last().expect("empty block delimiter");
            quote.extend_from_slice(char::from(byte).encode_utf8(&mut [0; 4]).as_bytes());
        }
        if n.range == Range::default() {
            if n.value.as_bytes().contains(&b'\n') {
                n.range.start.line = 1;
                n.range.end.line = 2;
            }
            n.value = trim_space(n.value.as_bytes()).to_vec().into();
        }
        self.put("|");
        self.put(&quote);
        self.put(&n.tag);
        if !n.range.one_line() {
            self.indent += 2;
        } else {
            self.put(" ");
        }
        self.lines(n.value.as_bytes(), n.range.one_line());
        if !n.range.one_line() {
            self.indent -= 2;
            self.newline();
        } else if !n.value.as_bytes().is_empty() {
            self.put(" ");
        }
        self.put(quote);
        self.put("|");
    }
    fn path(&mut self, path: &Option<SharedSlice<Option<Shared<StringBox>>>>) {
        for (i, slot) in path.iter().flat_map(SharedSlice::iter).enumerate() {
            if i > 0 {
                self.put(".");
            }
            self.optional(
                slot.read()
                    .as_ref()
                    .expect("nil path component")
                    .read()
                    .unbox(),
            );
        }
    }
    fn key(&mut self, key: &KeyPath) {
        self.in_key = true;
        self.path(&key.path);
        self.in_key = false;
    }
    fn arrow(&mut self, e: &Edge) {
        self.put(if e.src_arrow.is_empty() {
            "-"
        } else {
            &e.src_arrow
        });
        if e.dst_arrow.is_empty() {
            self.put("-");
        } else {
            if !e.src_arrow.is_empty() {
                self.put("-");
            }
            self.put(&e.dst_arrow);
        }
        if let Some(dst) = &e.dst {
            self.put(" ");
            self.key(&dst.read());
        }
    }
    fn map_key(&mut self, key: &Key) {
        if key.ampersand {
            self.put("&");
        } else if key.not_ampersand {
            self.put("!&");
        }
        if let Some(k) = &key.key {
            self.key(&k.read());
        }
        let edges: Vec<_> = key
            .edges
            .iter()
            .flat_map(SharedSlice::iter)
            .map(|e| e.read().clone().expect("nil edge"))
            .collect();
        if !edges.is_empty() {
            if key.key.is_some() {
                self.put(".");
            }
            let grouped = key.key.is_some() || key.edge_index.is_some() || key.edge_key.is_some();
            if grouped {
                self.put("(");
            }
            if let Some(src) = &edges[0].read().src {
                self.key(&src.read());
                self.put(" ");
            }
            for (i, e) in edges.iter().enumerate() {
                if i > 0 {
                    self.put(" ");
                }
                self.arrow(&e.read());
            }
            if grouped {
                self.put(")");
            }
            if let Some(i) = &key.edge_index {
                self.node(&Child::EdgeIndex(i.clone()));
            }
            if let Some(k) = &key.edge_key {
                self.put(".");
                self.key(&k.read());
            }
        }
        let primary = key.primary.unbox();
        if primary.is_some() {
            self.put(": ");
            self.optional(primary.clone());
        }
        if key
            .value
            .map
            .as_ref()
            .is_some_and(|m| m.read().nodes.as_ref().is_none_or(SharedSlice::is_empty))
        {
            return;
        }
        if let Some(value) = key.value.unbox() {
            self.put(if primary.is_none() { ": " } else { " " });
            self.node(&value);
        }
    }
    fn array(&mut self, a: &Array) {
        self.put("[");
        let one = a.range.one_line();
        if !one {
            self.indent += 2;
        }
        let mut previous = a.range.clone();
        let mut has_previous = false;
        for (i, b) in a.nodes.iter().flat_map(SharedSlice::iter).enumerate() {
            let b = b.read();
            let node = b.unbox().expect("nil array node");
            let range = node.range();
            if i > 0
                && (b.comment.is_some() || b.block_comment.is_some())
                && range.start.line == previous.end.line
                && range.one_line()
            {
                self.put(" ");
                self.node(&node);
                continue;
            }
            if !one {
                if has_previous && range.start.line - previous.end.line > 1 {
                    self.put("\n");
                }
                self.newline();
            } else if i > 0 {
                self.put("; ");
            }
            self.node(&node);
            previous = range;
            has_previous = true;
        }
        if !one {
            self.indent -= 2;
            self.newline();
        }
        self.put("]");
    }
    fn map(&mut self, m: &Map) {
        let file = m.is_file_map();
        let one = m.range.one_line();
        if !file {
            self.put("{");
            if !one {
                self.indent += 2;
            }
        }
        let mut boards = Vec::new();
        let mut previous = m.range.clone();
        let mut has_previous = false;
        let count = m.nodes.as_ref().map_or(0, SharedSlice::len);
        for (i, b) in m.nodes.iter().flat_map(SharedSlice::iter).enumerate() {
            let b = b.read();
            let node = b.unbox().expect("nil map node");
            let range = node.range();
            if b.is_board_node() {
                if b.map_key
                    .as_ref()
                    .unwrap()
                    .read()
                    .value
                    .map
                    .as_ref()
                    .is_some_and(|m| m.read().nodes.as_ref().is_some_and(|n| !n.is_empty()))
                {
                    boards.push(node);
                }
                previous = range;
                has_previous = true;
                continue;
            }
            if i > 0
                && (b.comment.is_some() || b.block_comment.is_some())
                && range.start.line == previous.end.line
                && range.one_line()
            {
                self.put(" ");
                self.node(&node);
                continue;
            }
            if !one {
                if has_previous && range.start.line - previous.end.line > 1 {
                    self.put("\n");
                }
                if !file || i > 0 {
                    self.newline();
                }
            } else if i > 0 {
                self.put("; ");
            }
            self.node(&node);
            previous = range;
            has_previous = true;
        }
        for (i, node) in boards.iter().enumerate() {
            if node.range().start.line != 0 {
                self.put("\n");
            }
            if i != 0 || count > boards.len() {
                self.put("\n");
            }
            self.output.resize(self.output.len() + self.indent, b' ');
            self.node(node);
        }
        if !file {
            if !one {
                self.indent -= 2;
                self.newline();
            }
            self.put("}");
        } else if count > 0 {
            self.put("\n");
        }
    }
}
fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack.windows(needle.len()).any(|w| w == needle)
}
fn trim_space(mut value: &[u8]) -> &[u8] {
    while !value.is_empty() {
        let (r, n) = crate::parser::decode_rune(value);
        if !crate::parser::is_space(r) {
            break;
        }
        value = &value[n..];
    }
    // Invalid suffix bytes terminate trimming, like Go's TrimSpace.
    while !value.is_empty() {
        let mut start = value.len() - 1;
        while start > 0 && value[start] & 0xc0 == 0x80 {
            start -= 1;
        }
        let (r, n) = crate::parser::decode_rune(&value[start..]);
        if start + n != value.len() || !crate::parser::is_space(r) {
            break;
        }
        value = &value[..start];
    }
    value
}
#[derive(Clone, Copy)]
enum Escape {
    Single,
    Double,
    Unquoted,
}
fn escape(value: &str, kind: Escape, in_key: bool, options: FormatOptions<'_>) -> String {
    if matches!(kind, Escape::Unquoted) {
        if value.is_empty() {
            return "\"\"".into();
        }
        if value.eq_ignore_ascii_case("null") {
            return "'null'".into();
        }
    }
    let mut out = String::new();
    for (i, r) in value.char_indices() {
        if r == '\n' {
            out.push_str("\\n");
            continue;
        }
        let slash = match kind {
            Escape::Single => {
                if r == '\'' {
                    out.push('\'');
                }
                false
            }
            Escape::Double => r == '"' || r == '\\' || (!in_key && r == '$'),
            Escape::Unquoted => match r {
                '\'' | '"' | '|' => i == 0,
                '-' if in_key => value.as_bytes().get(i + 1) == Some(&b'-'),
                '&' if in_key => i == 0,
                _ => {
                    if in_key {
                        options.unquoted_key_specials.contains(r)
                    } else {
                        options.unquoted_value_specials.contains(r)
                    }
                }
            },
        };
        if slash {
            out.push('\\');
        }
        out.push(r);
    }
    out
}
fn clean_path(path: &str) -> String {
    let absolute = path.starts_with('/');
    let mut parts = Vec::new();
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push(p);
                }
            }
            _ => parts.push(p),
        }
    }
    let result = format!("{}{}", if absolute { "/" } else { "" }, parts.join("/"));
    if result.is_empty() {
        ".".into()
    } else {
        result
    }
}

/// Canonical parseable spelling of each key component. Empty paths return None,
/// preserving the source API's nil result independently of empty byte output.
pub fn key_path(path: &KeyPath) -> Option<Vec<Vec<u8>>> {
    key_path_with_options(path, FormatOptions::default())
}
pub fn key_path_with_options(path: &KeyPath, options: FormatOptions<'_>) -> Option<Vec<Vec<u8>>> {
    let parts = path.path.as_ref().filter(|p| !p.is_empty())?;
    Some(
        parts
            .iter()
            .map(|slot| {
                canonical_component(
                    &slot.read().as_ref().expect("nil key component").read(),
                    options,
                )
            })
            .collect(),
    )
}
/// Canonicalize scalar values, ignoring input quote forms as upstream does.
pub fn format_key_path(path: &[StringBox]) -> Vec<u8> {
    format_key_path_with_options(path, FormatOptions::default())
}
pub fn format_key_path_with_options(path: &[StringBox], options: FormatOptions<'_>) -> Vec<u8> {
    let mut output = Vec::new();
    for (i, part) in path.iter().enumerate() {
        if i > 0 {
            output.push(b'.');
        }
        output.extend(canonical_component(part, options));
    }
    output
}
fn canonical_component(part: &StringBox, options: FormatOptions<'_>) -> Vec<u8> {
    // d2format/keypath.go's plainKey path avoids temporary shared AST nodes.
    // Input quote forms are ignored, reserved words are lowercased, and null
    // remains quoted. Custom specials still decide whether quoting is required.
    if let Some(bytes) = part.with_scalar_bytes(|bytes| {
        (!bytes.is_empty()
            && bytes.iter().all(|byte| {
                (byte.is_ascii_alphanumeric() || *byte == b'_')
                    && !options.unquoted_key_specials.as_bytes().contains(byte)
            }))
        .then(|| {
            let mut spelling = bytes.to_ascii_lowercase();
            if spelling == b"null" {
                return b"'null'".to_vec();
            }
            if !is_reserved_keyword(std::str::from_utf8(&spelling).expect("ASCII key")) {
                spelling.copy_from_slice(bytes);
            }
            spelling
        })
    }) {
        return bytes;
    }
    // Reconstruct spelling without changing the caller's stored bytes.
    let bytes = ByteString::from(part.scalar_bytes());
    let value = raw_string_box_with_specials(
        bytes,
        true,
        options.unquoted_key_specials,
        options.unquoted_value_specials,
    );
    let mut printer = Printer {
        options,
        in_key: true,
        ..Default::default()
    };
    printer.optional(value.unbox());
    printer.output
}
