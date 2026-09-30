//! Accepting a proposal on the server: markdown → Y.XmlFragment, and back.
//!
//! Until this module, only the browser applied a proposal. `docprops.rs` still
//! explains why that was the rule — building ProseMirror content means knowing
//! the editor's exact schema, and a server writing nodes it half-understands is
//! how a shared document gets quietly corrupted. The owner now wants an agent to
//! be able to accept, so the rule became a GUARD instead of a prohibition:
//!
//! - **The browser is the reference, and this is its twin.** Every function here
//!   ports one in `web/src/lib/doc-ops.ts` or `web/src/lib/inline-markdown.ts`,
//!   step for step. `web/src/lib/proposal-parity.test.ts` runs the REAL accept
//!   (Tiptap editor, SectionEditor schema, Collaboration on a Y.Doc) and commits
//!   what lands in the CRDT to `tests/fixtures/proposal-markdown.json`;
//!   `tests::matches_the_browser_fixture` below replays the same ops here and
//!   must produce the same element names, attributes, text runs and formatting
//!   attributes. Change either side and one of the two goes red.
//! - **What cannot be matched is refused, not approximated.** The browser parses
//!   an HTML table or a `<details>` block with the DOM and the editor's
//!   `parseHTML` rules — a whole HTML parser this binary does not carry. A
//!   proposal containing one is refused (`validation.proposal_unsupported`)
//!   before anything is written; a person can still accept it in the browser.
//! - **Nothing is written until every op has parsed.** A refusal must leave the
//!   document exactly as it was, and `RoomGuard::mutate` only broadcasts what
//!   a closure that returned `Ok` wrote.
//!
//! The read side lives here too (`inline_markdown`), because it is the same
//! grammar run backwards: an agent reading a section must see `**bold**` where
//! the editor shows bold, and reading a block and proposing it back unchanged
//! must still be recognised as a no-op.

use crate::error::{ApiError, ApiResult};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use yrs::types::text::YChange;
use yrs::types::xml::XmlOut;
use yrs::types::{Attrs, Delta};
use yrs::{
    Any, Map, Out, ReadTxn, Text, TransactionMut, Xml, XmlElementPrelim, XmlFragment,
    XmlFragmentRef, XmlTextPrelim,
};

// ---- inline grammar: the twin of web/src/lib/inline-markdown.ts -------------

/// One mark, in the order the TS side sorts them (`MARK_ORDER`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    Link(String),
    Bold,
    Italic,
    Strike,
    Code,
}

impl Mark {
    fn rank(&self) -> u8 {
        match self {
            Mark::Link(_) => 0,
            Mark::Bold => 1,
            Mark::Italic => 2,
            Mark::Strike => 3,
            Mark::Code => 4,
        }
    }
}

/// A run of text with its marks, sorted by rank.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub text: String,
    pub marks: Vec<Mark>,
}

impl Run {
    fn plain(text: &str) -> Self {
        Run {
            text: text.to_string(),
            marks: Vec::new(),
        }
    }
}

const ESCAPABLE: &str = "!\"#$%&'()*+,-./:;<=>?@[\\]^_`{|}~";

fn escapable(c: char) -> bool {
    ESCAPABLE.contains(c)
}

/// The inline grammar's whitespace: exactly the TS `SPACE` set.
fn inline_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c' | '\x0b')
}

/// `/^[\p{Alphabetic}\p{N}]$/u` — Rust's `is_numeric` is exactly `\p{N}`.
fn inline_alnum(c: char) -> bool {
    c.is_alphabetic() || c.is_numeric()
}

enum Tok {
    Ch { c: char, literal: bool },
    Code(String),
}

impl Tok {
    fn ch(&self) -> Option<char> {
        match self {
            Tok::Ch { c, .. } => Some(*c),
            Tok::Code(_) => None,
        }
    }
    fn delim(&self, want: char) -> bool {
        matches!(self, Tok::Ch { c, literal: false } if *c == want)
    }
    fn space(&self) -> bool {
        self.ch().is_some_and(inline_space)
    }
    fn alnum(&self) -> bool {
        self.ch().is_some_and(inline_alnum)
    }
}

fn tokenize(chars: &[char]) -> Vec<Tok> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '`' {
            let mut e = i;
            while e < chars.len() && chars[e] == '`' {
                e += 1;
            }
            runs.push((i, e - i));
            i = e;
        } else {
            i += 1;
        }
    }
    let mut next_same = vec![None; runs.len()];
    let mut last_by_len: HashMap<usize, usize> = HashMap::new();
    for k in (0..runs.len()).rev() {
        next_same[k] = last_by_len.get(&runs[k].1).copied();
        last_by_len.insert(runs[k].1, k);
    }
    let run_at: HashMap<usize, usize> = runs.iter().enumerate().map(|(k, r)| (r.0, k)).collect();

    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '\\' && i + 1 < chars.len() && escapable(chars[i + 1]) {
            out.push(Tok::Ch {
                c: chars[i + 1],
                literal: true,
            });
            i += 2;
            continue;
        }
        if let Some(&k) = run_at.get(&i) {
            let (start, len) = runs[k];
            if let Some(close) = next_same[k] {
                let end = runs[close].0;
                out.push(Tok::Code(chars[start + len..end].iter().collect()));
                i = end + len;
            } else {
                for _ in 0..len {
                    out.push(Tok::Ch {
                        c: '`',
                        literal: true,
                    });
                }
                i += len;
            }
            continue;
        }
        out.push(Tok::Ch { c, literal: false });
        i += 1;
    }
    out
}

/// Whether a link destination may become a link: a scheme must be http(s) or
/// mailto. The TS `safeHref`.
pub fn safe_href(href: &str) -> bool {
    let mut chars = href.char_indices();
    match chars.next() {
        Some((_, c)) if c.is_ascii_alphabetic() => {}
        _ => return true,
    }
    for (i, c) in chars {
        if c == ':' {
            let scheme = href[..i].to_ascii_lowercase();
            return ["http", "https", "mailto"].contains(&scheme.as_str());
        }
        if !(c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-')) {
            return true;
        }
    }
    true
}

struct Delim {
    text: &'static str,
    c: char,
    mark: Mark,
}

fn doubles() -> [Delim; 3] {
    [
        Delim {
            text: "**",
            c: '*',
            mark: Mark::Bold,
        },
        Delim {
            text: "__",
            c: '_',
            mark: Mark::Bold,
        },
        Delim {
            text: "~~",
            c: '~',
            mark: Mark::Strike,
        },
    ]
}

fn singles() -> [Delim; 2] {
    [
        Delim {
            text: "*",
            c: '*',
            mark: Mark::Italic,
        },
        Delim {
            text: "_",
            c: '_',
            mark: Mark::Italic,
        },
    ]
}

fn with_mark(marks: &[Mark], mark: &Mark) -> Vec<Mark> {
    if marks
        .iter()
        .any(|m| std::mem::discriminant(m) == std::mem::discriminant(mark))
    {
        return marks.to_vec();
    }
    let mut out = marks.to_vec();
    out.push(mark.clone());
    out.sort_by_key(Mark::rank);
    out
}

struct InlineParser {
    t: Vec<Tok>,
    closers: HashMap<&'static str, Vec<usize>>,
    matching: HashMap<usize, usize>,
    stop: Vec<usize>,
}

impl InlineParser {
    fn new(text: &str) -> Self {
        let chars: Vec<char> = text.chars().collect();
        let t = tokenize(&chars);
        let m = t.len();
        let at = |i: usize| t.get(i);
        let mut closers = HashMap::new();
        for d in doubles() {
            let mut list = Vec::new();
            let mut j = 1;
            while j + 1 < m {
                let ok = t[j].delim(d.c)
                    && t[j + 1].delim(d.c)
                    && !at(j + 2).is_some_and(|x| x.delim(d.c))
                    && !t[j - 1].space()
                    && !(d.c == '_' && at(j + 2).is_some_and(Tok::alnum));
                if ok {
                    list.push(j);
                }
                j += 1;
            }
            closers.insert(d.text, list);
        }
        for d in singles() {
            let mut list = Vec::new();
            for j in 1..m {
                let ok = t[j].delim(d.c)
                    && !t[j - 1].delim(d.c)
                    && !t[j - 1].space()
                    && !(d.c == '_' && at(j + 1).is_some_and(Tok::alnum));
                if ok {
                    list.push(j);
                }
            }
            closers.insert(d.text, list);
        }
        let mut matching = HashMap::new();
        let mut stack = Vec::new();
        for (i, tok) in t.iter().enumerate() {
            if tok.delim('[') {
                stack.push(i);
            } else if tok.delim(']') {
                if let Some(open) = stack.pop() {
                    matching.insert(open, i);
                }
            }
        }
        let mut stop = vec![m; m + 1];
        for p in (0..m).rev() {
            let tok = &t[p];
            stop[p] = if matches!(tok, Tok::Code(_)) || tok.space() || tok.delim(')') {
                p
            } else {
                stop[p + 1]
            };
        }
        InlineParser {
            t,
            closers,
            matching,
            stop,
        }
    }

    fn at(&self, i: usize) -> Option<&Tok> {
        self.t.get(i)
    }

    fn before(&self, i: usize) -> Option<&Tok> {
        i.checked_sub(1).and_then(|p| self.t.get(p))
    }

    fn first_closer(&self, d: &Delim, from: usize, end: usize) -> Option<usize> {
        let list = &self.closers[d.text];
        let lo = list.partition_point(|&j| j < from);
        list.get(lo)
            .copied()
            .filter(|&j| j + d.text.chars().count() <= end)
    }

    fn parse(&self, start: usize, end: usize, links: bool) -> Vec<Run> {
        let mut runs: Vec<Run> = Vec::new();
        let mut buf = String::new();
        fn flush(runs: &mut Vec<Run>, buf: &mut String) {
            if !buf.is_empty() {
                runs.push(Run::plain(buf));
                buf.clear();
            }
        }
        let mut i = start;
        'outer: while i < end {
            let tok = &self.t[i];
            if let Tok::Code(code) = tok {
                flush(&mut runs, &mut buf);
                runs.push(Run {
                    text: code.clone(),
                    marks: vec![Mark::Code],
                });
                i += 1;
                continue;
            }
            if links && tok.delim('[') {
                if let Some(&k) = self.matching.get(&i) {
                    if k > i + 1 && k + 1 < end && self.at(k + 1).is_some_and(|x| x.delim('(')) {
                        let h = self.stop[k + 2];
                        if h < end && h > k + 2 && self.t[h].delim(')') {
                            let href: String =
                                self.t[k + 2..h].iter().filter_map(Tok::ch).collect();
                            if safe_href(&href) {
                                flush(&mut runs, &mut buf);
                                let mark = Mark::Link(href);
                                for r in self.parse(i + 1, k, false) {
                                    runs.push(Run {
                                        marks: with_mark(&r.marks, &mark),
                                        text: r.text,
                                    });
                                }
                                i = h + 1;
                                continue;
                            }
                        }
                    }
                }
            }
            if let Tok::Ch { c, literal: false } = tok {
                if matches!(c, '*' | '_' | '~') {
                    for d in doubles() {
                        if d.c != *c
                            || !self.at(i + 1).is_some_and(|x| x.delim(d.c))
                            || self.before(i).is_some_and(|x| x.delim(d.c))
                        {
                            continue;
                        }
                        if i + 2 >= end || self.t[i + 2].space() {
                            continue;
                        }
                        if d.c == '_' && self.before(i).is_some_and(Tok::alnum) {
                            continue;
                        }
                        let Some(j) = self.first_closer(&d, i + 3, end) else {
                            continue;
                        };
                        flush(&mut runs, &mut buf);
                        for r in self.parse(i + 2, j, links) {
                            runs.push(Run {
                                marks: with_mark(&r.marks, &d.mark),
                                text: r.text,
                            });
                        }
                        i = j + 2;
                        continue 'outer;
                    }
                    for d in singles() {
                        if d.c != *c
                            || i + 1 >= end
                            || self.t[i + 1].delim(d.c)
                            || self.t[i + 1].space()
                        {
                            continue;
                        }
                        if d.c == '_' && self.before(i).is_some_and(Tok::alnum) {
                            continue;
                        }
                        let Some(j) = self.first_closer(&d, i + 2, end) else {
                            continue;
                        };
                        flush(&mut runs, &mut buf);
                        for r in self.parse(i + 1, j, links) {
                            runs.push(Run {
                                marks: with_mark(&r.marks, &d.mark),
                                text: r.text,
                            });
                        }
                        i = j + 1;
                        continue 'outer;
                    }
                }
            }
            if let Some(c) = tok.ch() {
                buf.push(c);
            }
            i += 1;
        }
        flush(&mut runs, &mut buf);
        runs
    }
}

/// Parse one block's inline markdown into runs. The TS `parseInline`.
pub fn parse_inline(text: &str) -> Vec<Run> {
    let parser = InlineParser::new(text);
    merge(parser.parse(0, parser.t.len(), true))
}

fn merge(runs: Vec<Run>) -> Vec<Run> {
    let mut out: Vec<Run> = Vec::new();
    for r in runs {
        if r.text.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.marks == r.marks => last.text.push_str(&r.text),
            _ => out.push(r),
        }
    }
    out
}

// ---- inline markdown out: what an agent reads --------------------------------

/// The marks a Y.XmlText chunk carries, as y-prosemirror stores them: one
/// formatting attribute per mark, keyed by the mark's name, valued by its attrs.
fn marks_of(attrs: Option<&Attrs>) -> Vec<Mark> {
    let Some(attrs) = attrs else {
        return Vec::new();
    };
    let on = |key: &str| {
        attrs
            .get(key)
            .is_some_and(|v| !matches!(v, Any::Null | Any::Bool(false)))
    };
    let mut marks = Vec::new();
    if let Some(Any::Map(link)) = attrs.get("link") {
        if let Some(Any::String(href)) = link.get("href") {
            marks.push(Mark::Link(href.to_string()));
        }
    }
    for (key, mark) in [
        ("bold", Mark::Bold),
        ("italic", Mark::Italic),
        ("strike", Mark::Strike),
        ("code", Mark::Code),
    ] {
        if on(key) {
            marks.push(mark);
        }
    }
    marks
}

/// Every text run under an element, in document order, nested structure
/// flattened exactly the way `prose::element_text` flattens it — an inline atom
/// (a section reference, a hard break) contributes its plain text.
pub fn element_runs<T: ReadTxn>(txn: &T, el: &yrs::XmlElementRef) -> Vec<Run> {
    let mut out = Vec::new();
    collect_runs(txn, el, &mut out);
    merge(out)
}

fn collect_runs<T: ReadTxn>(txn: &T, el: &yrs::XmlElementRef, out: &mut Vec<Run>) {
    if el.tag().as_ref() == "sectionReference" {
        out.push(Run::plain(&crate::store::prose::element_text(txn, el)));
        return;
    }
    for child in el.children(txn) {
        match child {
            XmlOut::Text(text) => {
                for diff in text.diff(txn, YChange::identity) {
                    let insert = match &diff.insert {
                        Out::Any(Any::String(s)) => s.to_string(),
                        other => other.clone().to_string(txn),
                    };
                    out.push(Run {
                        text: insert,
                        marks: marks_of(diff.attributes.as_deref()),
                    });
                }
            }
            XmlOut::Element(inner) => collect_runs(txn, &inner, out),
            XmlOut::Fragment(_) => {}
        }
    }
}

/// Emphasis never starts or ends on whitespace in this grammar, so the spaces
/// at the edges of a bold, italic or strike SPAN move outside it. The words keep
/// their marks; only the spaces around them lose one. Spans, not runs: a bold
/// sentence with an italic word inside is one bold span, and the spaces around
/// the italic word stay bold.
fn normalize_for_render(runs: &[Run]) -> Vec<Run> {
    let mut chars: Vec<(char, Vec<Mark>)> = runs
        .iter()
        .flat_map(|r| r.text.chars().map(|c| (c, r.marks.clone())))
        .collect();
    for mark in [Mark::Bold, Mark::Italic, Mark::Strike] {
        let mut i = 0;
        while i < chars.len() {
            if !chars[i].1.contains(&mark) {
                i += 1;
                continue;
            }
            let mut end = i;
            while end < chars.len() && chars[end].1.contains(&mark) {
                end += 1;
            }
            let strip = |c: &mut (char, Vec<Mark>)| {
                if inline_space(c.0) && !c.1.contains(&Mark::Code) {
                    c.1.retain(|m| *m != mark);
                    true
                } else {
                    false
                }
            };
            let mut a = i;
            while a < end && strip(&mut chars[a]) {
                a += 1;
            }
            let mut b = end;
            while b > a && strip(&mut chars[b - 1]) {
                b -= 1;
            }
            i = end;
        }
    }
    merge(
        chars
            .into_iter()
            .map(|(c, marks)| Run {
                text: c.to_string(),
                marks,
            })
            .collect(),
    )
}

fn code_span(text: &str) -> String {
    let mut longest = 0;
    let mut current = 0;
    for c in text.chars() {
        if c == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    let fence = "`".repeat(longest + 1);
    format!("{fence}{text}{fence}")
}

fn escape_all(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if matches!(c, '\\' | '*' | '_' | '~' | '`' | '[' | ']') {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn render(runs: &[Run], italic: &str, escape: bool) -> String {
    let mut out = String::new();
    let mut stack: Vec<Mark> = Vec::new();
    let close = |m: &Mark, out: &mut String| match m {
        Mark::Link(href) => out.push_str(&format!("]({href})")),
        Mark::Bold => out.push_str("**"),
        Mark::Italic => out.push_str(italic),
        Mark::Strike => out.push_str("~~"),
        Mark::Code => {}
    };
    for r in runs {
        let want: Vec<&Mark> = r.marks.iter().filter(|m| **m != Mark::Code).collect();
        let mut keep = 0;
        while keep < stack.len() && want.contains(&&stack[keep]) {
            keep += 1;
        }
        while stack.len() > keep {
            let m = stack.pop().expect("non-empty stack");
            close(&m, &mut out);
        }
        for m in want {
            if !stack.contains(m) {
                out.push_str(match m {
                    Mark::Link(_) => "[",
                    Mark::Bold => "**",
                    Mark::Italic => italic,
                    Mark::Strike => "~~",
                    Mark::Code => "",
                });
                stack.push(m.clone());
            }
        }
        if r.marks.contains(&Mark::Code) {
            out.push_str(&code_span(&r.text));
        } else if escape {
            out.push_str(&escape_all(&r.text));
        } else {
            out.push_str(&r.text);
        }
    }
    while let Some(m) = stack.pop() {
        close(&m, &mut out);
    }
    out
}

/// Runs as inline markdown that parses back to exactly these runs.
///
/// Four spellings are tried, most readable first — `_italic_` or `*italic*`,
/// unescaped or with every marker escaped — and the first one `parse_inline`
/// reads back unchanged wins. So plain prose with a stray `*` or a
/// `snake_case_name` reads back as it is, and a literal `*stars*` a person typed
/// reads as `\*stars\*` rather than turning into italics when the agent proposes
/// the block back. Verification rather than cleverness: the parser is the
/// authority on what a spelling means.
pub fn inline_markdown(runs: &[Run]) -> String {
    let runs = normalize_for_render(runs);
    let expected = merge(runs.clone());
    let mut first = None;
    for (italic, escape) in [("_", false), ("*", false), ("_", true), ("*", true)] {
        let text = render(&runs, italic, escape);
        if parse_inline(&text) == expected {
            return text;
        }
        first.get_or_insert(text);
    }
    first.unwrap_or_default()
}

// ---- blocks: the twin of markdownToNodes -------------------------------------

/// A node to write, before it is written.
#[derive(Debug, Clone, PartialEq)]
pub enum PNode {
    El {
        tag: &'static str,
        attrs: Vec<(&'static str, Any)>,
        children: Vec<PNode>,
    },
    Text(Vec<Run>),
}

fn el(tag: &'static str, attrs: Vec<(&'static str, Any)>, children: Vec<PNode>) -> PNode {
    PNode::El {
        tag,
        attrs,
        children,
    }
}

/// JavaScript's `\s` (and what `String.prototype.trim` strips).
fn js_space(c: char) -> bool {
    (c.is_whitespace() && c != '\u{85}') || c == '\u{feff}'
}

fn js_trim(s: &str) -> &str {
    s.trim_matches(js_space)
}

/// `schema.text` for each run, with the schema's exclusions: `code` excludes
/// every other mark in Tiptap, so a run carrying it keeps only it. The browser
/// gets this from `Mark.addToSet`.
fn inline_nodes(text: &str) -> Vec<PNode> {
    let runs: Vec<Run> = parse_inline(text)
        .into_iter()
        .map(|mut r| {
            if r.marks.contains(&Mark::Code) {
                r.marks = vec![Mark::Code];
            }
            r
        })
        .collect();
    let runs = merge(runs);
    if runs.is_empty() {
        Vec::new()
    } else {
        vec![PNode::Text(runs)]
    }
}

fn paragraph(text: &str) -> PNode {
    el("paragraph", vec![], inline_nodes(text))
}

/// Whether a line holds `<table …>`, `</details>` and the like — the TS chunker's
/// `/<(\/?)(?:table|details)(?:\s[^>]*|)>/gi`.
fn has_html_block_tag(line: &str) -> bool {
    let lower = line.to_lowercase();
    let bytes: Vec<char> = lower.chars().collect();
    for (i, &c) in bytes.iter().enumerate() {
        if c != '<' {
            continue;
        }
        let mut p = i + 1;
        if bytes.get(p) == Some(&'/') {
            p += 1;
        }
        for name in ["table", "details"] {
            let n: Vec<char> = name.chars().collect();
            if bytes.len() >= p + n.len() && bytes[p..p + n.len()] == n[..] {
                let after = p + n.len();
                match bytes.get(after) {
                    Some('>') => return true,
                    Some(&c) if js_space(c) && bytes[after..].contains(&'>') => return true,
                    _ => {}
                }
            }
        }
    }
    false
}

/// The TS `proposalChunks`, minus the HTML depth tracking: a tag that would
/// have started it is refused instead (see `parse_blocks`).
fn chunks(markdown: &str) -> Result<Vec<String>, String> {
    let normalized = markdown.replace("\r\n", "\n").replace('\r', "\n");
    let mut out = Vec::new();
    let mut lines: Vec<&str> = Vec::new();
    let mut fenced = false;
    let flush = |lines: &mut Vec<&str>, out: &mut Vec<String>| {
        if !lines.is_empty() {
            out.push(lines.join("\n"));
            lines.clear();
        }
    };
    for line in normalized.split('\n') {
        let trimmed = js_trim(line);
        if !fenced && trimmed.starts_with("```") {
            flush(&mut lines, &mut out);
            fenced = true;
            lines.push(line);
            continue;
        }
        if fenced && trimmed.starts_with("```") && trimmed[3..].chars().all(js_space) {
            lines.push(line);
            fenced = false;
            flush(&mut lines, &mut out);
            continue;
        }
        if !fenced && has_html_block_tag(line) {
            return Err(
                "it contains an HTML table or <details> block, which only the browser can \
                 parse the way the editor does"
                    .to_string(),
            );
        }
        if trimmed.is_empty() && !fenced {
            flush(&mut lines, &mut out);
        } else {
            lines.push(line);
        }
    }
    flush(&mut lines, &mut out);
    Ok(out)
}

fn starts_with_ci_tag(block: &str, tag: &str) -> bool {
    let lower: String = block
        .chars()
        .take(tag.len() + 2)
        .collect::<String>()
        .to_lowercase();
    lower.starts_with(tag)
        && lower[tag.len()..]
            .chars()
            .next()
            .is_some_and(|c| c == '>' || js_space(c))
}

/// The TS `cells` of `pipeTable`.
fn pipe_cells(line: &str) -> Vec<String> {
    let mut s: Vec<char> = js_trim(line).chars().collect();
    if s.first() == Some(&'|') {
        s.remove(0);
    }
    if s.last() == Some(&'|') && (s.len() < 2 || s[s.len() - 2] != '\\') {
        s.pop();
    }
    let mut cells = Vec::new();
    let mut cur = String::new();
    for (i, &c) in s.iter().enumerate() {
        if c == '|' && (i == 0 || s[i - 1] != '\\') {
            cells.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    cells.push(cur);
    cells
        .into_iter()
        .map(|c| js_trim(&c).replace("\\|", "|"))
        .collect()
}

fn separator(cell: &str) -> bool {
    let inner = cell.strip_prefix(':').unwrap_or(cell);
    let inner = inner.strip_suffix(':').unwrap_or(inner);
    inner.len() >= 3 && inner.chars().all(|c| c == '-')
}

fn pipe_table(block: &str) -> Option<PNode> {
    let lines: Vec<&str> = block.split('\n').collect();
    if lines.len() < 2 || !lines[0].contains('|') {
        return None;
    }
    let headers = pipe_cells(lines[0]);
    let separators = pipe_cells(lines[1]);
    if headers.len() != separators.len() || !separators.iter().all(|s| separator(s)) {
        return None;
    }
    let mut rows = vec![headers.clone()];
    for line in &lines[2..] {
        rows.push(pipe_cells(line));
    }
    if rows.iter().any(|r| r.len() != headers.len()) {
        return None;
    }
    let aligns: Vec<Option<&'static str>> = separators
        .iter()
        .map(|s| match (s.starts_with(':'), s.ends_with(':')) {
            (true, true) => Some("center"),
            (true, false) => Some("left"),
            (false, true) => Some("right"),
            (false, false) => None,
        })
        .collect();
    let rows = rows
        .iter()
        .enumerate()
        .map(|(i, row)| {
            let cells = row
                .iter()
                .enumerate()
                .map(|(col, text)| {
                    // `tableCell`/`tableHeader` defaults, as ProseMirror fills
                    // them in: colspan 1, rowspan 1, no colwidth (null is
                    // never written), align only when there is one.
                    let mut attrs =
                        vec![("colspan", Any::Number(1.0)), ("rowspan", Any::Number(1.0))];
                    if let Some(align) = aligns[col] {
                        attrs.push(("align", Any::from(align)));
                    }
                    el(
                        if i == 0 { "tableHeader" } else { "tableCell" },
                        attrs,
                        vec![paragraph(text)],
                    )
                })
                .collect();
            el("tableRow", vec![], cells)
        })
        .collect();
    Some(el("table", vec![], rows))
}

/// `^```([^\n]*)\n([\s\S]*?)\n?```$` on a trimmed block.
fn fence(block: &str) -> Option<(String, String)> {
    let rest = block.strip_prefix("```")?;
    let nl = rest.find('\n')?;
    let language = &rest[..nl];
    let body = rest[nl + 1..].strip_suffix("```")?;
    let body = body.strip_suffix('\n').unwrap_or(body);
    Some((language.to_string(), body.to_string()))
}

/// `^(#{1,6})\s+(.*)$` — `.` stops at line terminators, `$` is the end.
fn heading(block: &str) -> Option<(usize, String)> {
    let hashes = block.chars().take_while(|&c| c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = &block[hashes..];
    let text = rest.trim_start_matches(js_space);
    if text.len() == rest.len() {
        return None;
    }
    if text.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
        return None;
    }
    Some((hashes, text.to_string()))
}

fn strip_quote(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('>')?;
    let mut chars = rest.chars();
    Some(match chars.next() {
        Some(c) if js_space(c) => chars.as_str(),
        _ => rest,
    })
}

/// `^[-*]\s+` or `^\d+\.\s+`, returning the item text.
fn list_item(line: &str, ordered: bool) -> Option<&str> {
    let rest = if ordered {
        let digits = line.chars().take_while(|c| c.is_ascii_digit()).count();
        if digits == 0 {
            return None;
        }
        line[digits..].strip_prefix('.')?
    } else {
        line.strip_prefix(['-', '*'])?
    };
    let text = rest.trim_start_matches(js_space);
    (text.len() < rest.len()).then_some(text)
}

/// Markdown → nodes, exactly as `markdownToNodes` builds them for the editor's
/// schema. `Err` names what cannot be matched; nothing is half-parsed.
pub fn parse_blocks(markdown: &str) -> Result<Vec<PNode>, String> {
    let mut out = Vec::new();
    for chunk in chunks(markdown)? {
        let block = js_trim(&chunk);
        if block.is_empty() {
            continue;
        }
        if starts_with_ci_tag(block, "<table") || starts_with_ci_tag(block, "<details") {
            return Err(
                "it contains an HTML table or <details> block, which only the browser can \
                 parse the way the editor does"
                    .to_string(),
            );
        }
        if let Some(table) = pipe_table(block) {
            out.push(table);
            continue;
        }
        if let Some((language, code)) = fence(block) {
            let language = js_trim(&language);
            let attrs = if language.is_empty() {
                vec![]
            } else {
                vec![("language", Any::from(language))]
            };
            let children = if code.is_empty() {
                vec![]
            } else {
                vec![PNode::Text(vec![Run::plain(&code)])]
            };
            out.push(el("codeBlock", attrs, children));
            continue;
        }
        if let Some((level, text)) = heading(block) {
            out.push(el(
                "heading",
                vec![("level", Any::Number(level as f64))],
                inline_nodes(&text),
            ));
            continue;
        }
        if block.len() >= 3 && block.chars().all(|c| c == '-') {
            out.push(el("horizontalRule", vec![], vec![]));
            continue;
        }
        let lines: Vec<&str> = block.split('\n').collect();
        if lines.iter().all(|l| l.starts_with('>')) {
            let text: Vec<&str> = lines.iter().filter_map(|l| strip_quote(l)).collect();
            out.push(el("blockquote", vec![], vec![paragraph(&text.join("\n"))]));
            continue;
        }
        let bullets = lines.iter().all(|l| list_item(l, false).is_some());
        let ordered = lines.iter().all(|l| list_item(l, true).is_some());
        if bullets || ordered {
            let items = lines
                .iter()
                .map(|l| {
                    let text = list_item(l, !bullets).unwrap_or_default();
                    el("listItem", vec![], vec![paragraph(text)])
                })
                .collect();
            out.push(if bullets {
                el("bulletList", vec![], items)
            } else {
                // ProseMirror's `start` default is written like any other
                // non-null attribute.
                el("orderedList", vec![("start", Any::Number(1.0))], items)
            });
            continue;
        }
        out.push(paragraph(&lines.join(" ")));
    }
    if out.is_empty() {
        out.push(paragraph(""));
    }
    Ok(out)
}

// ---- writing nodes into the CRDT ---------------------------------------------

fn mark_attrs(marks: &[Mark]) -> Attrs {
    let mut attrs = Attrs::new();
    for m in marks {
        let (key, value) = match m {
            // Tiptap's Link defaults, which `mark.attrs` carries in full and
            // y-prosemirror stores as the attribute's value.
            Mark::Link(href) => (
                "link",
                Any::Map(Arc::new(HashMap::from([
                    ("href".to_string(), Any::from(href.as_str())),
                    ("target".to_string(), Any::from("_blank")),
                    ("rel".to_string(), Any::from("noopener noreferrer nofollow")),
                    ("class".to_string(), Any::Null),
                    ("title".to_string(), Any::Null),
                ]))),
            ),
            Mark::Bold => ("bold", Any::Map(Arc::new(HashMap::new()))),
            Mark::Italic => ("italic", Any::Map(Arc::new(HashMap::new()))),
            Mark::Strike => ("strike", Any::Map(Arc::new(HashMap::new()))),
            Mark::Code => ("code", Any::Map(Arc::new(HashMap::new()))),
        };
        attrs.insert(key.into(), value);
    }
    attrs
}

fn write_children<F: XmlFragment>(txn: &mut TransactionMut, parent: &F, children: &[PNode]) {
    for child in children {
        match child {
            PNode::El {
                tag,
                attrs,
                children,
            } => {
                let node = parent.push_back(txn, XmlElementPrelim::empty(*tag));
                for (k, v) in attrs {
                    node.insert_attribute(txn, *k, v.clone());
                }
                write_children(txn, &node, children);
            }
            PNode::Text(runs) => {
                let text = parent.push_back(txn, XmlTextPrelim::new(""));
                // `applyDelta` with explicit attributes, as y-prosemirror
                // writes a text node: a plain run carries none, so nothing
                // is inherited from the run before it.
                text.apply_delta(
                    txn,
                    runs.iter().map(|r| {
                        Delta::Inserted(
                            Any::from(r.text.as_str()),
                            Some(Box::new(mark_attrs(&r.marks))),
                        )
                    }),
                );
            }
        }
    }
}

fn insert_top(txn: &mut TransactionMut, frag: &XmlFragmentRef, index: u32, node: &PNode, id: &str) {
    let PNode::El {
        tag,
        attrs,
        children,
    } = node
    else {
        return;
    };
    let el = frag.insert(txn, index, XmlElementPrelim::empty(*tag));
    el.insert_attribute(txn, "id", id);
    for (k, v) in attrs {
        el.insert_attribute(txn, *k, v.clone());
    }
    write_children(txn, &el, children);
}

// ---- applying ops: the twin of applyOps --------------------------------------

/// One op of a stored proposal, its markdown already parsed.
pub struct PreparedOp {
    pub kind: String,
    pub id: String,
    pub nodes: Vec<PNode>,
}

/// Parse every op of a stored proposal record, or refuse the lot.
pub fn prepare(record: &Value) -> ApiResult<Vec<PreparedOp>> {
    let ops = record
        .get("ops")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let kind = op
            .get("op")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let id = op
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let nodes = if kind == "delete" {
            Vec::new()
        } else {
            let markdown = op
                .get("markdown")
                .and_then(Value::as_str)
                .unwrap_or_default();
            parse_blocks(markdown).map_err(|why| {
                ApiError::validation(
                    "validation.proposal_unsupported",
                    format!(
                        "Operation {i} ({kind} {id}) cannot be applied by the server: {why}. \
                         Nothing was changed and the proposal is still pending."
                    ),
                )
                .remedy(
                    "Leave it for a person to accept in the document view, where the editor \
                     parses it, or propose the table as a Markdown pipe table instead."
                        .to_string(),
                )
            })?
        };
        out.push(PreparedOp { kind, id, nodes });
    }
    Ok(out)
}

fn attr_id<T: ReadTxn>(txn: &T, el: &yrs::XmlElementRef) -> Option<String> {
    match el.get_attribute(txn, "id")? {
        Out::Any(Any::String(s)) => Some(s.to_string()),
        _ => None,
    }
}

fn find_block<T: ReadTxn>(txn: &T, frag: &XmlFragmentRef, id: &str) -> Option<u32> {
    frag.children(txn)
        .enumerate()
        .find_map(|(i, node)| match node {
            XmlOut::Element(el) if attr_id(txn, &el).as_deref() == Some(id) => Some(i as u32),
            _ => None,
        })
}

/// Apply prepared ops to a live fragment, returning how many applied and one
/// sentence per skipped op — worded exactly as the browser's `applyOps` words
/// it, because both end up on the same record in `dropped`.
///
/// `mint` issues a fresh block id for every new top-level node other than the
/// first node of a `replace`, which keeps the id it replaces — what the
/// browser's BlockId plugin does a transaction later.
pub fn apply(
    txn: &mut TransactionMut,
    frag: &XmlFragmentRef,
    ops: &[PreparedOp],
    mint: &mut dyn FnMut() -> String,
) -> (usize, Vec<String>) {
    let mut skipped = Vec::new();
    let mut applied = 0;
    let mut trailing: HashMap<String, u32> = HashMap::new();
    for op in ops {
        let Some(pos) = find_block(txn, frag, &op.id) else {
            skipped.push(format!(
                "{} {}: that block is no longer in the document",
                op.kind, op.id
            ));
            continue;
        };
        if op.kind == "delete" {
            frag.remove_range(txn, pos, 1);
            trailing.remove(&op.id);
            applied += 1;
            continue;
        }
        let carried = trailing.get(&op.id).copied().unwrap_or(0);
        let count = op.nodes.len() as u32;
        if op.kind == "replace" {
            frag.remove_range(txn, pos, 1);
            for (i, node) in op.nodes.iter().enumerate() {
                let id = if i == 0 { op.id.clone() } else { mint() };
                insert_top(txn, frag, pos + i as u32, node, &id);
            }
            trailing.insert(op.id.clone(), carried + count.saturating_sub(1));
        } else {
            let len = frag.len(txn);
            let at = (pos + 1 + carried).min(len);
            for (i, node) in op.nodes.iter().enumerate() {
                let id = mint();
                insert_top(txn, frag, at + i as u32, node, &id);
            }
            trailing.insert(op.id.clone(), carried + count);
        }
        applied += 1;
    }
    (applied, skipped)
}

// ---- deciding: accept or reject a stored proposal ----------------------------

/// What a caller decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Accept,
    Reject,
}

impl Decision {
    pub fn status(self) -> &'static str {
        match self {
            Decision::Accept => "accepted",
            Decision::Reject => "rejected",
        }
    }
}

/// The outcome: the record as now stored, and what an accept could not apply.
pub struct Decided {
    pub record: Value,
    pub applied: usize,
    pub skipped: Vec<String>,
}

/// Accept or reject one proposal in a live replica, exactly as the browser's
/// `onAccept`/`onReject` and `decideProposal` do:
///
/// - Only a `pending` proposal is decided; anything else is
///   `conflict.proposal_decided`, so a second decision never overwrites the
///   first one's `decided_by`.
/// - Rejecting only marks the record. Accepting applies the ops and marks it,
///   in ONE transaction, with ops whose block has gone skipped and written onto
///   the record as `dropped` — "accepted" and "accepted and landed whole" stay
///   distinguishable after the fact.
/// - An accept where no op applied is not an acceptance
///   (`conflict.proposal_stale`) and leaves the proposal pending, which is the
///   browser's rule too: a durable claim that somebody accepted a change the
///   document never received is worse than no claim.
/// - Every op is parsed before anything is written, so a refusal
///   (`validation.proposal_unsupported`) changes nothing.
///
/// `fragment` finds the prose the proposal addresses — a plan section's
/// fragment, or a document's — and answers `None` when it no longer exists.
pub fn decide(
    doc: &yrs::Doc,
    proposal: &str,
    decision: Decision,
    actor: &str,
    now: i64,
    fragment: impl FnOnce(&yrs::Doc, &Value) -> ApiResult<Option<XmlFragmentRef>>,
) -> ApiResult<Decided> {
    let map = doc.get_or_insert_map(crate::api::docprops::PROPOSALS_FIELD);
    let current = {
        let txn = yrs::Transact::transact(doc);
        match map.get(&txn, proposal) {
            Some(Out::Any(Any::String(s))) => serde_json::from_str::<Value>(&s).ok(),
            _ => None,
        }
    };
    let Some(mut record) = current.filter(Value::is_object) else {
        return Err(ApiError::not_found("proposal", proposal));
    };
    let status = record
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if status != "pending" {
        let by = record
            .get("decided_by")
            .and_then(Value::as_str)
            .unwrap_or("somebody");
        return Err(ApiError::conflict(
            "conflict.proposal_decided",
            format!("Proposal '{proposal}' is already {status} (by {by}); only a pending proposal can be decided."),
        )
        .current_state(status.clone())
        .remedy(
            "Read the proposals again to see the decision. To change the text now, propose \
             against the section as it stands."
                .to_string(),
        ));
    }

    let (applied, skipped, txn) = match decision {
        Decision::Reject => (0, Vec::new(), yrs::Transact::transact_mut(doc)),
        Decision::Accept => {
            let ops = prepare(&record)?;
            let target = fragment(doc, &record)?;
            let mut txn = yrs::Transact::transact_mut(doc);
            let (applied, skipped) = match &target {
                Some(frag) => apply(&mut txn, frag, &ops, &mut crate::ids::block_id),
                None => (
                    0,
                    ops.iter()
                        .map(|op| {
                            format!(
                                "{} {}: that block is no longer in the document",
                                op.kind, op.id
                            )
                        })
                        .collect(),
                ),
            };
            if applied == 0 {
                // Nothing was written: every op was skipped before touching
                // the fragment, so dropping the transaction changes nothing.
                return Err(ApiError::conflict(
                    "conflict.proposal_stale",
                    format!(
                        "None of the proposal's operations could be applied: {}. It stays pending.",
                        if skipped.is_empty() {
                            "it has no operations".to_string()
                        } else {
                            skipped.join("; ")
                        }
                    ),
                )
                .remedy(
                    "The blocks it addresses are gone. Reject it, and propose again against \
                     the section as it stands now."
                        .to_string(),
                ));
            }
            (applied, skipped, txn)
        }
    };
    let mut txn = txn;
    let obj = record.as_object_mut().expect("checked above");
    obj.insert("status".into(), json!(decision.status()));
    obj.insert("decided_by".into(), json!(actor));
    obj.insert("decided_at".into(), json!(now));
    if !skipped.is_empty() {
        obj.insert("dropped".into(), json!(skipped));
    }
    map.insert(&mut txn, proposal.to_string(), record.to_string());
    drop(txn);
    Ok(Decided {
        record,
        applied,
        skipped,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yrs::{Doc, GetString, Transact};

    fn any_json(v: &Any) -> Value {
        match v {
            Any::Null | Any::Undefined => Value::Null,
            Any::Bool(b) => json!(b),
            Any::Number(n) if n.fract() == 0.0 => json!(*n as i64),
            Any::Number(n) => json!(n),
            Any::BigInt(n) => json!(n),
            Any::String(s) => json!(s.as_ref()),
            Any::Array(a) => Value::Array(a.iter().map(any_json).collect()),
            Any::Map(m) => Value::Object(m.iter().map(|(k, v)| (k.clone(), any_json(v))).collect()),
            Any::Buffer(_) => Value::Null,
        }
    }

    fn canonical<T: ReadTxn>(txn: &T, node: XmlOut, top: bool, known: &[&str]) -> Value {
        match node {
            XmlOut::Text(text) => {
                let delta: Vec<Value> = text
                    .diff(txn, YChange::identity)
                    .into_iter()
                    .map(|d| {
                        let insert = match &d.insert {
                            Out::Any(Any::String(s)) => s.to_string(),
                            other => other.clone().to_string(txn),
                        };
                        match d.attributes.filter(|a| !a.is_empty()) {
                            Some(a) => json!({"insert": insert, "attributes": Value::Object(
                                a.iter().map(|(k, v)| (k.to_string(), any_json(v))).collect())}),
                            None => json!({"insert": insert}),
                        }
                    })
                    .collect();
                json!({ "text": delta })
            }
            XmlOut::Element(el) => {
                let mut attrs = serde_json::Map::new();
                for (k, v) in el.attributes(txn) {
                    let v = match v {
                        Out::Any(a) => any_json(&a),
                        other => json!(other.to_string(txn)),
                    };
                    attrs.insert(k.to_string(), v);
                }
                if top {
                    if let Some(Value::String(id)) = attrs.get("id").cloned() {
                        if !known.contains(&id.as_str()) {
                            assert!(id.starts_with("blk_"), "fresh id {id}");
                            attrs.insert("id".into(), json!("<fresh>"));
                        }
                    }
                }
                let content: Vec<Value> = el
                    .children(txn)
                    .map(|c| canonical(txn, c, false, known))
                    .collect();
                json!({ "type": el.tag().as_ref(), "attrs": attrs, "content": content })
            }
            XmlOut::Fragment(_) => Value::Null,
        }
    }

    fn start(doc: &Doc, blocks: &[(String, String)]) -> XmlFragmentRef {
        let frag = doc.get_or_insert_xml_fragment("prose");
        let mut txn = doc.transact_mut();
        for (id, text) in blocks {
            let p = frag.push_back(&mut txn, XmlElementPrelim::empty("paragraph"));
            p.insert_attribute(&mut txn, "id", id.as_str());
            p.push_back(&mut txn, XmlTextPrelim::new(text.as_str()));
        }
        frag
    }

    /// The parity proof: every case the browser recorded, replayed here.
    #[test]
    fn matches_the_browser_fixture() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/proposal-markdown.json"))
                .expect("fixture parses");
        let blocks: Vec<(String, String)> = fixture["start"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| (b[0].as_str().unwrap().into(), b[1].as_str().unwrap().into()))
            .collect();
        let known: Vec<&str> = blocks.iter().map(|b| b.0.as_str()).collect();
        let cases = fixture["cases"].as_array().unwrap();
        assert!(cases.len() >= 20, "the fixture lost its cases");
        for case in cases {
            let name = case["name"].as_str().unwrap();
            let doc = Doc::new();
            let frag = start(&doc, &blocks);
            let ops = prepare(&json!({ "ops": case["ops"] })).expect(name);
            let (applied, skipped) = {
                let mut txn = doc.transact_mut();
                apply(&mut txn, &frag, &ops, &mut crate::ids::block_id)
            };
            assert_eq!(json!(applied), case["applied"], "{name}: applied");
            assert_eq!(json!(skipped), case["skipped"], "{name}: skipped");
            let txn = doc.transact();
            let got: Vec<Value> = frag
                .children(&txn)
                .map(|n| canonical(&txn, n, true, &known))
                .collect();
            assert_eq!(
                Value::Array(got.clone()),
                case["fragment"],
                "{name}: the server wrote a different tree than the browser.\n got: {}",
                serde_json::to_string_pretty(&got).unwrap()
            );
        }
    }

    /// The inline grammar alone, over hand-picked edge cases and a seeded
    /// random soup of every marker, as `parseInline` read them.
    #[test]
    fn inline_parsing_matches_the_browser_fixture() {
        let fixture: Value =
            serde_json::from_str(include_str!("../../tests/fixtures/proposal-markdown.json"))
                .expect("fixture parses");
        let cases = fixture["inline"].as_array().expect("inline cases");
        assert!(cases.len() > 400, "the fixture lost its inline cases");
        for case in cases {
            let text = case["text"].as_str().unwrap();
            let got: Vec<Value> = parse_inline(text)
                .iter()
                .map(|r| {
                    let marks: Vec<Value> = r
                        .marks
                        .iter()
                        .map(|m| match m {
                            Mark::Link(href) => json!({"type": "link", "href": href}),
                            Mark::Bold => json!({"type": "bold"}),
                            Mark::Italic => json!({"type": "italic"}),
                            Mark::Strike => json!({"type": "strike"}),
                            Mark::Code => json!({"type": "code"}),
                        })
                        .collect();
                    json!({"text": r.text, "marks": marks})
                })
                .collect();
            assert_eq!(Value::Array(got), case["runs"], "inline {text:?}");
        }
    }

    #[test]
    fn html_tables_and_details_are_refused_not_approximated() {
        for md in [
            "<table><tr><td>x</td></tr></table>",
            "Intro\n\n<TABLE class=\"x\">\n<tr><td>a</td></tr>\n</table>",
            "<details><summary>More</summary><p>x</p></details>",
            "<table\nfoo",
        ] {
            assert!(parse_blocks(md).is_err(), "{md}");
            let err = prepare(&json!({"ops": [{"op": "replace", "id": "blk_a", "markdown": md}]}))
                .err()
                .expect("refused");
            assert_eq!(err.body.code, "validation.proposal_unsupported");
        }
        // Inside a code fence it is code, not a table.
        assert!(parse_blocks("```html\n<table><tr><td>x</td></tr></table>\n```").is_ok());
        // `<tablex>` is not a table tag.
        assert!(parse_blocks("a <tablex> b").is_ok());
    }

    fn runs(parts: &[(&str, &[Mark])]) -> Vec<Run> {
        parts
            .iter()
            .map(|(t, m)| Run {
                text: t.to_string(),
                marks: m.to_vec(),
            })
            .collect()
    }

    #[test]
    fn inline_parsing_follows_the_closed_grammar() {
        use Mark::*;
        assert_eq!(
            parse_inline("a **b** c"),
            runs(&[("a ", &[]), ("b", &[Bold]), (" c", &[])])
        );
        assert_eq!(parse_inline("\\*x\\*"), runs(&[("*x*", &[])]));
        assert_eq!(parse_inline("`**x**`"), runs(&[("**x**", &[Code])]));
        assert_eq!(
            parse_inline("snake_case_name"),
            runs(&[("snake_case_name", &[])])
        );
        assert_eq!(parse_inline("2 * 3 * 4"), runs(&[("2 * 3 * 4", &[])]));
        assert_eq!(
            parse_inline("[x](javascript:alert(1))"),
            runs(&[("[x](javascript:alert(1))", &[])])
        );
        assert_eq!(
            parse_inline("~~a~~ [**b**](https://e.com)"),
            runs(&[
                ("a", &[Strike]),
                (" ", &[]),
                ("b", &[Link("https://e.com".into()), Bold]),
            ])
        );
        // Pathological input stays linear enough to finish at the op cap.
        let long = "*a ".repeat(10_000);
        assert_eq!(parse_inline(&long).len(), 1);
    }

    #[test]
    fn inline_markdown_round_trips_through_the_parser() {
        use Mark::*;
        let cases: Vec<Vec<Run>> = vec![
            runs(&[
                ("plain ", &[]),
                ("bold", &[Bold]),
                (" and ", &[]),
                ("it", &[Italic]),
            ]),
            runs(&[
                ("bold ", &[Bold]),
                ("both", &[Bold, Italic]),
                (" end", &[Bold]),
            ]),
            runs(&[("a *literal* b", &[])]),
            runs(&[("snake_case", &[]), ("x", &[Italic]), ("y_z", &[])]),
            runs(&[
                ("see ", &[]),
                ("docs", &[Link("https://d.io/a_b".into())]),
                (".", &[]),
            ]),
            runs(&[("tick ` in", &[Code])]),
            runs(&[("gone", &[Strike]), (" \\ back", &[])]),
            runs(&[("Größe", &[Bold]), (" über", &[])]),
        ];
        for expected in cases {
            let md = inline_markdown(&expected);
            assert_eq!(parse_inline(&md), expected, "{md}");
        }
        // Readable when it can be.
        assert_eq!(
            inline_markdown(&runs(&[("a ", &[]), ("b", &[Bold]), (" c", &[Italic])])),
            "a **b** _c_"
        );
        assert_eq!(inline_markdown(&runs(&[("2 * 3", &[])])), "2 * 3");
        assert_eq!(inline_markdown(&runs(&[("a *b* c", &[])])), "a \\*b\\* c");
        // Spaces move outside emphasis rather than breaking it.
        assert_eq!(
            inline_markdown(&runs(&[("x", &[]), (" bold ", &[Bold]), ("y", &[])])),
            "x **bold** y"
        );
    }

    #[test]
    fn a_marked_section_reads_back_and_a_same_text_replace_is_a_no_op() {
        let doc = Doc::new();
        let frag = start(&doc, &[("blk_a".into(), "Alpha".into())]);
        let md = "Some **bold**, _it_, `code`, ~~gone~~ and [a link](https://e.com).";
        let ops = prepare(&json!({"ops": [
            {"op": "replace", "id": "blk_a", "markdown": md},
            {"op": "insert_after", "id": "blk_a", "markdown": "- **one**\n- two\n\n## A *head*\n\n> q **b**"},
        ]}))
        .unwrap();
        {
            let mut txn = doc.transact_mut();
            let (applied, skipped) = apply(&mut txn, &frag, &ops, &mut crate::ids::block_id);
            assert_eq!((applied, skipped.len()), (2, 0));
        }
        let txn = doc.transact();
        let blocks = crate::api::docprops::read_blocks(&txn, &frag);
        let annotated = crate::api::docprops::annotate(&blocks);
        assert!(annotated.contains(md), "{annotated}");
        assert!(annotated.contains("- **one**\n- two"), "{annotated}");
        assert!(annotated.contains("## A _head_"), "{annotated}");
        assert!(annotated.contains("> q **b**"), "{annotated}");
        assert!(frag.get_string(&txn).contains("bold"));
        let same = crate::api::docprops::validate_ops(
            &json!([{"op": "replace", "id": "blk_a", "markdown": md}]),
            &blocks,
            None,
            "takomo_document_read",
        );
        assert_eq!(
            same.err().expect("unchanged").body.code,
            "validation.document_unchanged"
        );
    }
}
