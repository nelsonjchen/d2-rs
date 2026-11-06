// SPDX-License-Identifier: MPL-2.0 AND BSD-3-Clause
// The html.Node tree that D2 v0.9.0 lib/textmeasure LayoutMarkdown obtains
// from goquery.NewDocumentFromReader (golang.org/x/net v0.57.0 html.Parse)
// over Goldmark's XHTML output. This is the subset of the HTML5 tokenizer
// (token.go) and "in body"/"in table" tree construction (parse.go) that
// Goldmark output and its raw-HTML passthrough exercise: character
// references, void elements, implied </p> and </li>, heading nesting, the
// ignored newline after <pre>, adjacent-text merging and scoped end tags.
// Unsupported elements are later rejected by validateMarkdownSVGNodes,
// exactly as in Go.

pub type NodeId = usize;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeType {
    Document,
    Element,
    Text,
    Comment,
}

#[derive(Clone, Debug)]
pub struct Node {
    pub kind: NodeType,
    pub data: String,
    pub attr: Vec<(String, String)>,
    pub parent: Option<NodeId>,
    pub first_child: Option<NodeId>,
    pub last_child: Option<NodeId>,
    pub prev_sibling: Option<NodeId>,
    pub next_sibling: Option<NodeId>,
}

#[derive(Clone, Debug, Default)]
pub struct Dom {
    pub nodes: Vec<Node>,
}

impl Dom {
    fn add(&mut self, kind: NodeType, data: String, attr: Vec<(String, String)>) -> NodeId {
        self.nodes.push(Node {
            kind,
            data,
            attr,
            parent: None,
            first_child: None,
            last_child: None,
            prev_sibling: None,
            next_sibling: None,
        });
        self.nodes.len() - 1
    }

    fn append_child(&mut self, parent: NodeId, child: NodeId) {
        let last = self.nodes[parent].last_child;
        self.nodes[child].parent = Some(parent);
        self.nodes[child].prev_sibling = last;
        match last {
            Some(last) => self.nodes[last].next_sibling = Some(child),
            None => self.nodes[parent].first_child = Some(child),
        }
        self.nodes[parent].last_child = Some(child);
    }

    pub fn node(&self, id: NodeId) -> &Node {
        &self.nodes[id]
    }

    pub fn children(&self, id: NodeId) -> Children<'_> {
        Children {
            dom: self,
            next: self.nodes[id].first_child,
        }
    }

    pub fn is_element(&self, id: NodeId, name: &str) -> bool {
        let n = &self.nodes[id];
        n.kind == NodeType::Element && n.data == name
    }

    /// nodeAttr.
    pub fn attr(&self, id: NodeId, name: &str) -> &str {
        self.nodes[id]
            .attr
            .iter()
            .find(|(k, _)| k == name)
            .map_or("", |(_, v)| v.as_str())
    }
}

pub struct Children<'a> {
    dom: &'a Dom,
    next: Option<NodeId>,
}

impl Iterator for Children<'_> {
    type Item = NodeId;
    fn next(&mut self) -> Option<NodeId> {
        let id = self.next?;
        self.next = self.dom.nodes[id].next_sibling;
        Some(id)
    }
}

fn is_void(name: &str) -> bool {
    matches!(
        name,
        "area"
            | "base"
            | "br"
            | "col"
            | "embed"
            | "hr"
            | "img"
            | "input"
            | "keygen"
            | "link"
            | "meta"
            | "param"
            | "source"
            | "track"
            | "wbr"
    )
}

/// parse.go isSpecialElement (HTML namespace).
fn is_special(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "applet"
            | "area"
            | "article"
            | "aside"
            | "base"
            | "basefont"
            | "bgsound"
            | "blockquote"
            | "body"
            | "br"
            | "button"
            | "caption"
            | "center"
            | "col"
            | "colgroup"
            | "dd"
            | "details"
            | "dir"
            | "div"
            | "dl"
            | "dt"
            | "embed"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "form"
            | "frame"
            | "frameset"
            | "h1"
            | "h2"
            | "h3"
            | "h4"
            | "h5"
            | "h6"
            | "head"
            | "header"
            | "hgroup"
            | "hr"
            | "html"
            | "iframe"
            | "img"
            | "input"
            | "keygen"
            | "li"
            | "link"
            | "listing"
            | "main"
            | "marquee"
            | "menu"
            | "meta"
            | "nav"
            | "noembed"
            | "noframes"
            | "noscript"
            | "object"
            | "ol"
            | "p"
            | "param"
            | "plaintext"
            | "pre"
            | "script"
            | "search"
            | "section"
            | "select"
            | "source"
            | "style"
            | "summary"
            | "table"
            | "tbody"
            | "td"
            | "template"
            | "textarea"
            | "tfoot"
            | "th"
            | "thead"
            | "title"
            | "tr"
            | "track"
            | "ul"
            | "wbr"
            | "xmp"
    )
}

fn is_heading(name: &str) -> bool {
    matches!(name, "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
}

/// Start tags that close an open <p> in button scope (parse.go inBodyIM).
fn closes_p(name: &str) -> bool {
    matches!(
        name,
        "address"
            | "article"
            | "aside"
            | "blockquote"
            | "center"
            | "details"
            | "dialog"
            | "dir"
            | "div"
            | "dl"
            | "fieldset"
            | "figcaption"
            | "figure"
            | "footer"
            | "header"
            | "hgroup"
            | "main"
            | "menu"
            | "nav"
            | "ol"
            | "p"
            | "search"
            | "section"
            | "summary"
            | "ul"
            | "pre"
            | "listing"
            | "hr"
            | "dd"
            | "dt"
            | "li"
    ) || is_heading(name)
}

enum Token {
    Text(String),
    Start(String, Vec<(String, String)>),
    End(String),
    Comment(String),
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\n' | b'\r' | b'\t' | 0x0c)
}

fn unescape(text: &str) -> String {
    if !text.contains('&') {
        return text.to_owned();
    }
    String::from_utf8_lossy(&crate::markdown::html::unescape_html(text.as_bytes())).into_owned()
}

/// token.go convertNewlines.
fn convert_newlines(s: &str) -> String {
    if !s.contains('\r') {
        return s.to_owned();
    }
    s.replace("\r\n", "\n").replace('\r', "\n")
}

fn tokenize(src: &str) -> Vec<Token> {
    let bytes = src.as_bytes();
    let mut tokens = Vec::new();
    let mut text_start = 0;
    let mut i = 0;
    let flush = |tokens: &mut Vec<Token>, from: usize, to: usize| {
        if to > from {
            tokens.push(Token::Text(unescape(&convert_newlines(&src[from..to]))));
        }
    };
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        let next = bytes.get(i + 1).copied();
        if src[i..].starts_with("<!--") {
            flush(&mut tokens, text_start, i);
            let body_start = i + 4;
            let end = src[body_start..]
                .find("-->")
                .map_or(bytes.len(), |n| body_start + n);
            tokens.push(Token::Comment(src[body_start..end].to_owned()));
            i = (end + 3).min(bytes.len());
            text_start = i;
            continue;
        }
        let closing = next == Some(b'/');
        let name_start = if closing { i + 2 } else { i + 1 };
        if !bytes.get(name_start).is_some_and(u8::is_ascii_alphabetic) {
            if next == Some(b'!') || next == Some(b'?') {
                // Bogus comment / doctype up to '>'.
                flush(&mut tokens, text_start, i);
                let end = src[i..].find('>').map_or(bytes.len(), |n| i + n);
                i = (end + 1).min(bytes.len());
                text_start = i;
                continue;
            }
            i += 1;
            continue;
        }
        flush(&mut tokens, text_start, i);
        let mut j = name_start;
        while j < bytes.len() && !is_space(bytes[j]) && bytes[j] != b'/' && bytes[j] != b'>' {
            j += 1;
        }
        let name = src[name_start..j].to_ascii_lowercase();
        let mut attrs = Vec::new();
        loop {
            while j < bytes.len() && (is_space(bytes[j]) || bytes[j] == b'/') {
                j += 1;
            }
            if j >= bytes.len() {
                break;
            }
            if bytes[j] == b'>' {
                j += 1;
                break;
            }
            let key_start = j;
            j += 1;
            while j < bytes.len() && !is_space(bytes[j]) && !matches!(bytes[j], b'=' | b'/' | b'>')
            {
                j += 1;
            }
            let key = src[key_start..j].to_ascii_lowercase();
            while j < bytes.len() && is_space(bytes[j]) {
                j += 1;
            }
            let mut value = String::new();
            if bytes.get(j) == Some(&b'=') {
                j += 1;
                while j < bytes.len() && is_space(bytes[j]) {
                    j += 1;
                }
                match bytes.get(j) {
                    Some(&q) if q == b'"' || q == b'\'' => {
                        let start = j + 1;
                        let end = src[start..]
                            .find(q as char)
                            .map_or(bytes.len(), |n| start + n);
                        value = src[start..end].to_owned();
                        j = (end + 1).min(bytes.len());
                    }
                    _ => {
                        let start = j;
                        while j < bytes.len() && !is_space(bytes[j]) && bytes[j] != b'>' {
                            j += 1;
                        }
                        value = src[start..j].to_owned();
                    }
                }
            }
            if !closing && !attrs.iter().any(|(k, _): &(String, String)| *k == key) {
                attrs.push((key, unescape(&convert_newlines(&value))));
            }
        }
        tokens.push(if closing {
            Token::End(name)
        } else {
            Token::Start(name, attrs)
        });
        i = j;
        text_start = i;
    }
    flush(&mut tokens, text_start, bytes.len());
    tokens
}

struct Builder {
    dom: Dom,
    stack: Vec<NodeId>,
    skip_newline: bool,
}

impl Builder {
    fn current(&self) -> NodeId {
        *self.stack.last().expect("open elements")
    }
    fn name(&self, id: NodeId) -> &str {
        &self.dom.nodes[id].data
    }
    fn in_scope(&self, matches: impl Fn(&str) -> bool, extra: &[&str]) -> bool {
        for &id in self.stack.iter().rev() {
            let name = self.name(id);
            if matches(name) {
                return true;
            }
            if matches!(
                name,
                "applet"
                    | "caption"
                    | "html"
                    | "table"
                    | "td"
                    | "th"
                    | "marquee"
                    | "object"
                    | "template"
            ) || extra.contains(&name)
            {
                return false;
            }
        }
        false
    }
    fn generate_implied_end_tags(&mut self, except: &str) {
        while let Some(&id) = self.stack.last() {
            let name = self.name(id);
            if name != except
                && matches!(
                    name,
                    "dd" | "dt" | "li" | "optgroup" | "option" | "p" | "rb" | "rp" | "rt" | "rtc"
                )
            {
                self.stack.pop();
            } else {
                break;
            }
        }
    }
    fn pop_until(&mut self, matches: impl Fn(&str) -> bool) {
        while self.stack.len() > 2 {
            let id = self.stack.pop().expect("stack");
            if matches(self.name(id)) {
                return;
            }
        }
    }
    fn close_p_in_button_scope(&mut self) {
        if self.in_scope(|n| n == "p", &["button"]) {
            self.generate_implied_end_tags("p");
            self.pop_until(|n| n == "p");
        }
    }
    fn insert(&mut self, name: String, attrs: Vec<(String, String)>, push: bool) {
        let id = self.dom.add(NodeType::Element, name, attrs);
        let parent = self.current();
        self.dom.append_child(parent, id);
        if push {
            self.stack.push(id);
        }
    }
    fn text(&mut self, mut text: String) {
        if std::mem::take(&mut self.skip_newline) && text.starts_with('\n') {
            text.remove(0);
        }
        text.retain(|c| c != '\0');
        if text.is_empty() {
            return;
        }
        let parent = self.current();
        if let Some(last) = self.dom.nodes[parent].last_child {
            if self.dom.nodes[last].kind == NodeType::Text {
                self.dom.nodes[last].data.push_str(&text);
                return;
            }
        }
        let id = self.dom.add(NodeType::Text, text, Vec::new());
        self.dom.append_child(parent, id);
    }
    fn start(&mut self, name: String, attrs: Vec<(String, String)>) {
        self.skip_newline = false;
        match name.as_str() {
            "html" | "head" | "body" => return,
            "li" => {
                for index in (0..self.stack.len()).rev() {
                    let id = self.stack[index];
                    let node = self.name(id).to_owned();
                    if node == "li" {
                        self.generate_implied_end_tags("li");
                        self.pop_until(|n| n == "li");
                        break;
                    }
                    if is_special(&node) && !matches!(node.as_str(), "address" | "div" | "p") {
                        break;
                    }
                }
            }
            "dd" | "dt" => {
                for index in (0..self.stack.len()).rev() {
                    let id = self.stack[index];
                    let node = self.name(id).to_owned();
                    if node == "dd" || node == "dt" {
                        self.generate_implied_end_tags(&node);
                        self.pop_until(|n| n == node);
                        break;
                    }
                    if is_special(&node) && !matches!(node.as_str(), "address" | "div" | "p") {
                        break;
                    }
                }
            }
            _ => {}
        }
        if closes_p(&name) {
            self.close_p_in_button_scope();
        }
        if is_heading(&name) && is_heading(self.name(self.current())) {
            self.stack.pop();
        }
        let void = is_void(&name);
        let pre = matches!(name.as_str(), "pre" | "listing");
        self.insert(name, attrs, !void);
        self.skip_newline = pre;
    }
    fn end(&mut self, name: String) {
        self.skip_newline = false;
        match name.as_str() {
            "html" | "body" | "head" => {}
            "br" => self.insert("br".into(), Vec::new(), false),
            "p" => {
                if !self.in_scope(|n| n == "p", &["button"]) {
                    self.insert("p".into(), Vec::new(), true);
                }
                self.generate_implied_end_tags("p");
                self.pop_until(|n| n == "p");
            }
            "li" => {
                if self.in_scope(|n| n == "li", &["ol", "ul"]) {
                    self.generate_implied_end_tags("li");
                    self.pop_until(|n| n == "li");
                }
            }
            "address" | "article" | "aside" | "blockquote" | "button" | "center" | "details"
            | "dialog" | "dir" | "div" | "dl" | "fieldset" | "figcaption" | "figure" | "footer"
            | "header" | "hgroup" | "listing" | "main" | "menu" | "nav" | "ol" | "pre"
            | "search" | "section" | "summary" | "ul" | "table" | "thead" | "tbody" | "tfoot"
            | "tr" | "td" | "th" | "caption" => {
                // inBodyIM's scoped block end tags and the in-table/row/cell
                // insertion modes' end tags: ignored unless open in scope.
                let target = name.clone();
                if self.stack.iter().any(|&id| self.name(id) == target)
                    && self.in_scope(|n| n == target, &[])
                {
                    self.generate_implied_end_tags("");
                    self.pop_until(|n| n == target);
                }
            }
            h if is_heading(h) => {
                if self.in_scope(is_heading, &[]) {
                    self.generate_implied_end_tags("");
                    self.pop_until(is_heading);
                }
            }
            _ => {
                for index in (2..self.stack.len()).rev() {
                    let id = self.stack[index];
                    if self.name(id) == name {
                        self.generate_implied_end_tags(&name);
                        self.stack.truncate(index);
                        return;
                    }
                    if is_special(self.name(id)) {
                        return;
                    }
                }
            }
        }
    }
}

/// html.Parse over a body fragment; returns the tree and its <body>.
pub fn parse_document(html: &str) -> (Dom, NodeId) {
    let mut dom = Dom::default();
    let document = dom.add(NodeType::Document, String::new(), Vec::new());
    let html_el = dom.add(NodeType::Element, "html".into(), Vec::new());
    dom.append_child(document, html_el);
    let head = dom.add(NodeType::Element, "head".into(), Vec::new());
    dom.append_child(html_el, head);
    let body = dom.add(NodeType::Element, "body".into(), Vec::new());
    dom.append_child(html_el, body);
    let mut builder = Builder {
        dom,
        stack: vec![html_el, body],
        skip_newline: false,
    };
    let mut started = false;
    for token in tokenize(html) {
        match token {
            Token::Text(text) => {
                // Whitespace before the first body content is dropped by the
                // "before html"/"before head"/"after head" insertion modes.
                let text = if started {
                    text
                } else {
                    text.trim_start_matches([' ', '\n', '\t', '\r', '\x0c'])
                        .to_owned()
                };
                if !text.is_empty() {
                    started = true;
                    builder.text(text);
                }
            }
            Token::Comment(comment) => {
                let id = builder.dom.add(NodeType::Comment, comment, Vec::new());
                let parent = if started { builder.current() } else { document };
                builder.dom.append_child(parent, id);
            }
            Token::Start(name, attrs) => {
                started = true;
                builder.start(name, attrs);
            }
            Token::End(name) => {
                if started {
                    builder.end(name);
                }
            }
        }
    }
    (builder.dom, body)
}

/// Preserve the HTML tree's text-node boundaries in an XML measurement tree.
/// Non-void self-closing tags are ordinary start tags in x/net/html, so an
/// XML parser cannot consume their original spelling directly.
pub fn measurement_fragment(html: &str) -> String {
    fn escaped(output: &mut String, value: &str, attribute: bool) {
        for c in value.chars() {
            match c {
                '&' => output.push_str("&amp;"),
                '<' => output.push_str("&lt;"),
                '>' => output.push_str("&gt;"),
                '"' if attribute => output.push_str("&quot;"),
                _ => output.push(c),
            }
        }
    }
    fn write(dom: &Dom, id: NodeId, output: &mut String) {
        let node = dom.node(id);
        match node.kind {
            NodeType::Text => escaped(output, &node.data, false),
            NodeType::Comment => {
                output.push_str("<!--");
                output.push_str(&node.data);
                output.push_str("-->");
            }
            NodeType::Element => {
                output.push('<');
                output.push_str(&node.data);
                for (name, value) in &node.attr {
                    output.push(' ');
                    output.push_str(name);
                    output.push_str("=\"");
                    escaped(output, value, true);
                    output.push('"');
                }
                output.push('>');
                for child in dom.children(id) {
                    write(dom, child, output);
                }
                output.push_str("</");
                output.push_str(&node.data);
                output.push('>');
            }
            NodeType::Document => {}
        }
    }
    let (dom, body) = parse_document(html);
    let mut output = String::with_capacity(html.len());
    for child in dom.children(body) {
        write(&dom, child, &mut output);
    }
    output
}

impl Dom {
    pub fn has_markdown_heading_ancestor(&self, n: NodeId) -> bool {
        let mut ancestor = self.node(n).parent;
        while let Some(a) = ancestor {
            let node = self.node(a);
            if node.kind == NodeType::Element
                && matches!(node.data.as_str(), "h1" | "h2" | "h3" | "h4" | "h5" | "h6")
            {
                return true;
            }
            ancestor = node.parent;
        }
        false
    }

    /// hasAncestorElement.
    pub fn has_ancestor_element(&self, n: NodeId, el: &str) -> bool {
        let mut ancestor = self.node(n).parent;
        while let Some(a) = ancestor {
            if self.is_element(a, el) {
                return true;
            }
            ancestor = self.node(a).parent;
        }
        false
    }

    fn blank(&self, n: NodeId) -> bool {
        self.node(n).data.trim().is_empty()
    }

    /// hasPrev.
    pub fn has_prev(&self, n: NodeId) -> bool {
        match self.node(n).prev_sibling {
            None => false,
            Some(p) if self.blank(p) => self.has_prev(p),
            Some(_) => true,
        }
    }

    /// hasNext.
    pub fn has_next(&self, n: NodeId) -> bool {
        match self.node(n).next_sibling {
            None => false,
            Some(p) if self.blank(p) => self.has_next(p),
            Some(_) => true,
        }
    }

    /// getPrev (which, as in Go, continues through getNext).
    pub fn get_prev(&self, n: Option<NodeId>) -> Option<NodeId> {
        let n = n?;
        if self.blank(n) {
            if let Some(next) = self.get_next(self.node(n).prev_sibling) {
                return Some(next);
            }
        }
        Some(n)
    }

    /// getNext.
    pub fn get_next(&self, n: Option<NodeId>) -> Option<NodeId> {
        let n = n?;
        if self.blank(n) {
            if let Some(next) = self.get_next(self.node(n).next_sibling) {
                return Some(next);
            }
        }
        Some(n)
    }

    pub fn parent_element_type(&self, n: NodeId) -> &str {
        match self.node(n).parent {
            Some(p) if self.node(p).kind == NodeType::Element => &self.node(p).data,
            _ => "",
        }
    }

    pub fn element_data(&self, n: NodeId) -> Option<&str> {
        let node = self.node(n);
        (node.kind == NodeType::Element).then_some(node.data.as_str())
    }

    /// markdownNodeText.
    pub fn markdown_node_text(&self, n: NodeId) -> String {
        fn walk(dom: &Dom, n: NodeId, out: &mut String) {
            let node = dom.node(n);
            if node.kind == NodeType::Text {
                out.push_str(&node.data);
                return;
            }
            if node.kind == NodeType::Element && node.data == "br" {
                out.push('\n');
                return;
            }
            for child in dom.children(n) {
                walk(dom, child, out);
            }
        }
        let mut out = String::new();
        walk(self, n, &mut out);
        out
    }
}
