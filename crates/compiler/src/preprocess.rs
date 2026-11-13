//! Normalizes Mermaid source before detection: line endings, attribute quotes, front matter,
//! directives, and comments. Port of Pele's `src/preprocess.ts`.

use std::borrow::Cow;
use std::ops::Range;

use crate::error::{Code, Error};
use crate::lines::{self, Lines};
use crate::util::chars::{has, is_line_end, is_space, is_word};
use crate::yaml::{self, Value};

/// Configuration from front matter and init directives, as JavaScript object properties in order.
pub(crate) type Config = Vec<(String, Value)>;

/// Source with front matter, directives, and comments removed.
#[derive(Debug)]
pub(crate) struct Preprocessed {
    /// Configuration from front matter and init directives.
    pub(crate) config: Config,
    /// The number of front matter lines before `with_comments`.
    pub(crate) line_offset: u32,
    /// The authored lines of `text`, when they were asked for.
    pub(crate) lines: Option<Lines>,
    /// The authored lines of `with_comments`, when they were asked for.
    pub(crate) lines_with_comments: Option<Lines>,
    /// The diagram text without comment lines, trimmed at the start.
    pub(crate) text: String,
    /// The front matter title.
    pub(crate) title: Option<String>,
    /// The diagram text before its comment lines are removed.
    pub(crate) with_comments: String,
}

const UNSAFE_KEYS: [&str; 3] = ["__proto__", "constructor", "prototype"];

fn space(c: char) -> bool {
    is_space(u32::from(c))
}

fn line_end(c: char) -> bool {
    is_line_end(u32::from(c))
}

fn word(c: char) -> bool {
    is_word(u32::from(c))
}

/// Removes comment lines (`%%` not followed by `{`) and leading whitespace.
pub(crate) fn cleanup_comments(text: &str) -> String {
    let stripped = if has(text, "%%") {
        Cow::Owned(lines::without(text, &comment_ranges(text)))
    } else {
        Cow::Borrowed(text)
    };
    stripped.trim_start_matches(space).to_owned()
}

// The ranges `text.replace(/^\s*%%(?!{)[^\n]+\n?/gm, '')` removes, in order.
fn comment_ranges(text: &str) -> Vec<Range<usize>> {
    let mut removed = Vec::new();
    let mut p = 0;
    loop {
        let line_start = p == 0
            || text
                .get(..p)
                .and_then(|head| head.chars().next_back())
                .is_some_and(line_end);
        if line_start {
            if let Some(end) = comment_at(text, p) {
                removed.push(p..end);
                p = end;
                continue;
            }
            // Every line start inside this whitespace run reaches the same `%%` test and fails.
            p = text
                .get(p..)
                .map_or(p, |rest| text.len() - rest.trim_start_matches(space).len());
        }

        let Some((offset, c)) = text
            .get(p..)
            .and_then(|rest| rest.char_indices().find(|&(_, c)| line_end(c)))
        else {
            break;
        };
        p += offset + c.len_utf8();
    }
    removed
}

// The end of a comment matched at line start `p`. `\s*` cannot give back characters to `%%`, so
// its greedy run is the only candidate.
fn comment_at(text: &str, p: usize) -> Option<usize> {
    let rest = text.get(p..)?;
    let body = rest.trim_start_matches(space).strip_prefix("%%")?;
    if body.starts_with(['{', '\n']) || body.is_empty() {
        return None;
    }

    let start = text.len() - body.len();
    Some(match body.find('\n') {
        Some(at) => start + at + 1,
        None => text.len(),
    })
}

// The byte length of the character at `at` when it is whitespace.
fn space_at(text: &str, at: usize) -> Option<usize> {
    let c = text.get(at..)?.chars().next()?;
    space(c).then_some(c.len_utf8())
}

fn is_break(text: &str, at: usize) -> bool {
    matches!(text.as_bytes().get(at), Some(b'\n' | b'\r'))
}

// The start of the last `needle` that starts at or before `from`.
fn last_index_of(text: &str, needle: &str, from: usize) -> Option<usize> {
    let end = from.saturating_add(needle.len()).min(text.len());
    text.as_bytes()
        .get(..end)?
        .windows(needle.len())
        .rposition(|window| window == needle.as_bytes())
}

// The start of the first `needle` that starts at or after `from`.
fn index_of(text: &str, needle: &str, from: usize) -> Option<usize> {
    text.as_bytes()
        .get(from..)?
        .windows(needle.len())
        .position(|window| window == needle.as_bytes())
        .map(|at| from + at)
}

struct FrontMatter<'a> {
    body: &'a str,
    indent: &'a str,
    length: usize,
}

// Finds a `---` fenced YAML block at the start of the text the way Mermaid's front matter regex does.
fn match_front_matter(text: &str) -> Option<FrontMatter<'_>> {
    let mut cursor = 0;
    while let Some(length) = space_at(text, cursor)
        && !is_break(text, cursor)
    {
        cursor += length;
    }
    let indent = text.get(..cursor)?;
    if !text.get(cursor..)?.starts_with("---") {
        return None;
    }

    let mut opening_ends = Vec::new();
    let mut i = cursor + 3;
    while let Some(length) = space_at(text, i) {
        if is_break(text, i) {
            opening_ends.push(i);
        }
        i += length;
    }
    let first = *opening_ends.first()?;

    let mut fence = String::from(indent);
    fence.push_str("---");
    let closing_end = |i: usize| -> Option<usize> {
        if !is_break(text, i) || !text.get(i + 1..).is_some_and(|t| t.starts_with(&fence)) {
            return None;
        }
        let mut last = None;
        let mut j = i + 1 + fence.len();
        while let Some(length) = space_at(text, j) {
            if is_break(text, j) {
                last = Some(j);
            }
            j += length;
        }
        last
    };

    let mut needle = String::from("\n");
    needle.push_str(&fence);
    let mut last_fence = None;
    let mut at = last_index_of(text, &needle, text.len());
    while let Some(i) = at
        && i > first
    {
        if closing_end(i).is_some() {
            last_fence = Some(i);
            break;
        }
        at = last_index_of(text, &needle, i - 1);
    }
    let last_fence = last_fence?;

    let opening_end = opening_ends
        .iter()
        .copied()
        .rev()
        .find(|&candidate| candidate < last_fence)
        .unwrap_or(first);

    let body_start = opening_end + 1;
    let mut at = index_of(text, &needle, body_start - 1);
    while let Some(i) = at
        && i <= last_fence
    {
        if i >= body_start
            && let Some(end) = closing_end(i)
        {
            return Some(FrontMatter {
                body: text.get(body_start..i)?,
                indent,
                length: end + 1,
            });
        }
        at = index_of(text, &needle, i + 1);
    }
    None
}

fn get<'a>(object: &'a [(String, Value)], key: &str) -> Option<&'a Value> {
    object
        .iter()
        .find(|entry| entry.0 == key)
        .map(|entry| &entry.1)
}

fn merge(target: &mut Config, source: &[(String, Value)]) {
    for (key, value) in source {
        if UNSAFE_KEYS.contains(&key.as_str()) {
            continue;
        }
        let current = target.iter_mut().find(|entry| entry.0 == *key);
        match (value, current) {
            (Value::Object(inner), Some((_, Value::Object(current)))) => merge(current, inner),
            (Value::Object(inner), _) => {
                let mut copy = Config::new();
                merge(&mut copy, inner);
                yaml::set(target, key.clone(), Value::Object(copy));
            }
            _ => yaml::set(target, key.clone(), value.clone()),
        }
    }
}

/// One match of Mermaid's directive pattern,
/// `/%{2}{\s*(?:(\w+)\s*:|(\w+))\s*(?:(\w+)|((?:(?!}%{2}).|\r?\n)*))?\s*(?:}%{2})?/gi`.
struct Directive<'a> {
    /// The fourth group: the text after `type:` up to `}%%`, when it is not a single word.
    args: Option<&'a str>,
    end: usize,
    /// The first or second group.
    kind: &'a str,
    start: usize,
}

fn find_directive(s: &str, from: usize) -> Option<Directive<'_>> {
    let mut at = from;
    loop {
        let start = index_of(s, "%%{", at)?;
        if let Some(directive) = directive_at(s, start) {
            return Some(directive);
        }
        at = start + 1;
    }
}

// Every part after the type is optional, so the first greedy choice at each step is the match.
fn directive_at(s: &str, start: usize) -> Option<Directive<'_>> {
    let skip_space = |at: usize| {
        s.get(at..)
            .map_or(at, |rest| s.len() - rest.trim_start_matches(space).len())
    };
    let skip_word = |at: usize| {
        s.get(at..)
            .map_or(at, |rest| s.len() - rest.trim_start_matches(word).len())
    };

    let word_start = skip_space(start + 3);
    let word_end = skip_word(word_start);
    if word_end == word_start {
        return None;
    }
    let kind = s.get(word_start..word_end)?;

    let mut pos = skip_space(word_end);
    if s.as_bytes().get(pos) == Some(&b':') {
        pos += 1;
    } else {
        pos = word_end;
    }
    pos = skip_space(pos);

    let mut args = None;
    let value_end = skip_word(pos);
    if value_end > pos {
        pos = value_end;
    } else {
        let args_start = pos;
        while let Some(rest) = s.get(pos..) {
            if rest.starts_with("}%%") {
                break;
            }
            if rest.starts_with("\r\n") {
                pos += 2;
                continue;
            }
            match rest.chars().next() {
                Some('\n') => pos += 1,
                Some(c) if !line_end(c) => pos += c.len_utf8(),
                _ => break,
            }
        }
        args = s.get(args_start..pos);
    }

    pos = skip_space(pos);
    if s.get(pos..).is_some_and(|rest| rest.starts_with("}%%")) {
        pos += 3;
    }
    Some(Directive {
        args,
        end: pos,
        kind,
        start,
    })
}

// Reads directives into `config`, and returns `text` without them and the ranges they held.
fn directives(text: &str, config: &mut Config) -> (String, Vec<Range<usize>>) {
    if !has(text, "%%{") {
        return (text.to_owned(), Vec::new());
    }

    let normalized = text
        .trim_matches(space)
        .chars()
        .map(|c| if c == '\'' { '"' } else { c })
        .collect::<String>();
    let mut from = 0;
    while let Some(directive) = find_directive(&normalized, from) {
        from = directive.end;
        match directive.kind {
            "wrap" => yaml::set(config, "wrap".to_owned(), Value::Bool(true)),
            "init" | "initialize" => {
                // Mermaid ignores an init directive whose body is not valid JSON.
                if let Some(args) = directive.args.filter(|args| !args.is_empty())
                    && let Some(Value::Object(args)) = json(args.trim_matches(space))
                {
                    merge(config, &args);
                }
            }
            _ => {}
        }
    }

    let mut removed = Vec::new();
    let mut last = 0;
    while let Some(directive) = find_directive(text, last) {
        removed.push(directive.start..directive.end);
        last = directive.end;
    }
    (lines::without(text, &removed), removed)
}

/// Deepest nesting `JSON.parse` reads here. V8 has no fixed limit, but a deeper directive is
/// ignored instead of overflowing the stack.
const MAX_JSON_DEPTH: usize = 1000;

/// `JSON.parse`, or `None` where it throws.
pub(crate) fn json(text: &str) -> Option<Value> {
    let mut parser = Json { pos: 0, text };
    let value = parser.value(0)?;
    parser.skip();
    (parser.pos == text.len()).then_some(value)
}

struct Json<'a> {
    pos: usize,
    text: &'a str,
}

impl Json<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.pos).copied()
    }

    fn skip(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.pos += 1;
        }
    }

    fn eat(&mut self, literal: &str) -> bool {
        let found = self
            .text
            .get(self.pos..)
            .is_some_and(|rest| rest.starts_with(literal));
        if found {
            self.pos += literal.len();
        }
        found
    }

    fn value(&mut self, depth: usize) -> Option<Value> {
        if depth > MAX_JSON_DEPTH {
            return None;
        }

        self.skip();
        match self.peek()? {
            b'{' => {
                self.pos += 1;
                let mut out = Vec::new();
                self.skip();
                if self.eat("}") {
                    return Some(Value::Object(out));
                }
                loop {
                    self.skip();
                    let key = self.string()?;
                    self.skip();
                    if !self.eat(":") {
                        return None;
                    }
                    let value = self.value(depth + 1)?;
                    yaml::set(&mut out, key, value);
                    self.skip();
                    if self.eat("}") {
                        return Some(Value::Object(out));
                    }
                    if !self.eat(",") {
                        return None;
                    }
                }
            }
            b'[' => {
                self.pos += 1;
                let mut out = Vec::new();
                self.skip();
                if self.eat("]") {
                    return Some(Value::Array(out));
                }
                loop {
                    out.push(self.value(depth + 1)?);
                    self.skip();
                    if self.eat("]") {
                        return Some(Value::Array(out));
                    }
                    if !self.eat(",") {
                        return None;
                    }
                }
            }
            b'"' => self.string().map(Value::String),
            b't' => self.eat("true").then_some(Value::Bool(true)),
            b'f' => self.eat("false").then_some(Value::Bool(false)),
            b'n' => self.eat("null").then_some(Value::Null),
            _ => self.number(),
        }
    }

    fn string(&mut self) -> Option<String> {
        if !self.eat("\"") {
            return None;
        }

        // Escapes may pair surrogates, so the text is built from UTF-16 units.
        let mut out: Vec<u16> = Vec::new();
        loop {
            let rest = self.text.get(self.pos..)?;
            let c = rest.chars().next()?;
            self.pos += c.len_utf8();
            match c {
                '"' => return Some(String::from_utf16_lossy(&out)),
                '\\' => {
                    let e = self.peek()?;
                    self.pos += 1;
                    let unit = match e {
                        b'"' => 0x22,
                        b'\\' => 0x5c,
                        b'/' => 0x2f,
                        b'b' => 8,
                        b'f' => 12,
                        b'n' => 10,
                        b'r' => 13,
                        b't' => 9,
                        b'u' => {
                            let hex = self.text.get(self.pos..self.pos + 4)?;
                            self.pos += 4;
                            if !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
                                return None;
                            }
                            u16::from_str_radix(hex, 16).ok()?
                        }
                        _ => return None,
                    };
                    out.push(unit);
                }
                c if u32::from(c) < 0x20 => return None,
                c => out.extend_from_slice(c.encode_utf16(&mut [0; 2])),
            }
        }
    }

    fn number(&mut self) -> Option<Value> {
        let start = self.pos;
        let digits = |parser: &mut Self| {
            let from = parser.pos;
            while parser.peek().is_some_and(|b| b.is_ascii_digit()) {
                parser.pos += 1;
            }
            parser.pos - from
        };

        self.eat("-");
        if !self.eat("0") && digits(self) == 0 {
            return None;
        }
        if self.eat(".") && digits(self) == 0 {
            return None;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if !self.eat("+") {
                self.eat("-");
            }
            if digits(self) == 0 {
                return None;
            }
        }
        let number = crate::util::float::parse(self.text.get(start..self.pos)?)?;
        Some(Value::Number(number))
    }
}

// `/<(\w+)([^>]*)>/g` with `="..."` attribute quotes turned to single quotes.
fn quote_attributes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last = 0;
    let mut at = 0;
    while let Some(offset) = text.get(at..).and_then(|rest| rest.find('<')) {
        let open = at + offset;
        let name_end = text.get(open + 1..).map_or(open + 1, |rest| {
            text.len() - rest.trim_start_matches(word).len()
        });
        let close = text.get(name_end..).and_then(|rest| rest.find('>'));
        let (true, Some(close)) = (name_end > open + 1, close) else {
            at = open + 1;
            continue;
        };
        let close = name_end + close;
        out.push_str(text.get(last..name_end).unwrap_or(""));
        push_single_quoted(&mut out, text.get(name_end..close).unwrap_or(""));
        out.push('>');
        last = close + 1;
        at = last;
    }
    out.push_str(text.get(last..).unwrap_or(""));
    out
}

// `attrs.replace(/="([^"]*)"/g, "='$1'")`.
fn push_single_quoted(out: &mut String, attrs: &str) {
    let mut last = 0;
    let mut at = 0;
    while let Some(offset) = attrs.get(at..).and_then(|rest| rest.find("=\"")) {
        let open = at + offset;
        let Some(length) = attrs.get(open + 2..).and_then(|rest| rest.find('"')) else {
            at = open + 1;
            continue;
        };
        out.push_str(attrs.get(last..open).unwrap_or(""));
        out.push_str("='");
        out.push_str(attrs.get(open + 2..open + 2 + length).unwrap_or(""));
        out.push('\'');
        last = open + 3 + length;
        at = last;
    }
    out.push_str(attrs.get(last..).unwrap_or(""));
}

/// Normalizes line endings and attribute quotes, reads front matter and directives into the title
/// and config, and removes comments. When `track` is set, also maps the lines of the results to
/// the lines of `source`.
pub(crate) fn preprocess(source: &str, track: bool) -> Result<Preprocessed, Error> {
    let mut text = if source.contains('\r') {
        let mut normalized = String::with_capacity(source.len());
        let mut chars = source.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '\r' {
                chars.next_if_eq(&'\n');
                normalized.push('\n');
            } else {
                normalized.push(c);
            }
        }
        normalized
    } else {
        source.to_owned()
    };

    // A tag needs a closing bracket, so nothing after the last one can match.
    let last_close = if text.contains('<') {
        text.rfind('>')
    } else {
        None
    };
    if let Some(last_close) = last_close
        && last_close > 0
    {
        let mut quoted = quote_attributes(text.get(..=last_close).unwrap_or(""));
        quoted.push_str(text.get(last_close + 1..).unwrap_or(""));
        text = quoted;
    }

    // Line endings are one `\n` each by now, and quoting attributes adds no line breaks.
    let mut lines = track.then(|| Lines::new(&text));
    let mut config = Config::new();
    let mut title = None;
    let mut line_offset = 0;

    let front = if has(&text, "---") {
        match_front_matter(&text)
    } else {
        None
    };
    if let Some(front) = front {
        let body = if front.indent.is_empty() {
            front.body.to_owned()
        } else {
            let mut body = String::with_capacity(front.body.len());
            for (i, line) in front.body.split('\n').enumerate() {
                if i > 0 {
                    body.push('\n');
                }
                body.push_str(line.strip_prefix(front.indent).unwrap_or(line));
            }
            body
        };

        let parsed = yaml::parse(&body).map_err(|message| {
            let mut full = String::from("Front matter is not valid YAML. ");
            full.push_str(&message);
            Error::new(Code::Syntax, full)
        })?;
        if let Value::Object(parsed) = &parsed {
            if let Some(value) = get(parsed, "title")
                && value.truthy()
            {
                let mut out = String::new();
                value.push_js_string(&mut out);
                title = Some(out);
            }
            if let Some(Value::Object(inner)) = get(parsed, "config") {
                merge(&mut config, inner);
            }
            if let Some(value) = get(parsed, "displayMode")
                && value.truthy()
            {
                let mut mode = String::new();
                value.push_js_string(&mut mode);
                let entry = ("displayMode".to_owned(), Value::String(mode));
                match config.iter_mut().find(|entry| entry.0 == "gantt") {
                    Some((_, Value::Object(gantt))) => yaml::set(gantt, entry.0, entry.1),
                    _ => yaml::set(&mut config, "gantt".to_owned(), Value::Object(vec![entry])),
                }
            }
        }

        let head = text.get(..front.length).unwrap_or("");
        line_offset = u32::try_from(head.matches('\n').count()).unwrap_or(u32::MAX);
        if let Some(lines) = &mut lines {
            lines.remove(&text, std::slice::from_ref(&(0..front.length)));
        }
        text = text.get(front.length..).unwrap_or("").to_owned();
    }

    let (with_comments, removed) = directives(&text, &mut config);
    let lines_with_comments = lines.map(|mut lines| {
        lines.remove(&text, &removed);
        lines
    });
    let text = cleanup_comments(&with_comments);
    let lines = lines_with_comments.as_ref().map(|lines| {
        let mut lines = lines.clone();
        let removed = if has(&with_comments, "%%") {
            comment_ranges(&with_comments)
        } else {
            Vec::new()
        };
        let stripped = lines::without(&with_comments, &removed);
        lines.remove(&with_comments, &removed);
        // `cleanup_comments` trims the start of what the comments leave.
        let trimmed = 0..stripped.len().saturating_sub(text.len());
        lines.remove(&stripped, std::slice::from_ref(&trimmed));
        lines
    });

    Ok(Preprocessed {
        config,
        line_offset,
        lines,
        lines_with_comments,
        text,
        title,
        with_comments,
    })
}

// Does what Mermaid's `text.replace(/keyword.*:\S*#.*;/g, drop the last character)` does: on a
// line where the keyword is followed by a colon, a color-like `#`, and later a semicolon, the last
// semicolon goes. Mermaid's regex backtracks without bound; this is one pass.
fn drop_color_semicolons<'a>(text: &'a str, keyword: &str) -> Cow<'a, str> {
    let Some(mut from) = text.find(keyword) else {
        return Cow::Borrowed(text);
    };

    let mut out = String::new();
    let mut last = 0;
    loop {
        let (end, width) = text
            .get(from..)
            .and_then(|rest| rest.char_indices().find(|&(_, c)| line_end(c)))
            .map_or((text.len(), 0), |(at, c)| (from + at, c.len_utf8()));

        let mut after_colon = false;
        let mut hash = None;
        let scanned = text.get(from + keyword.len()..end).unwrap_or("");
        for (j, c) in scanned.char_indices() {
            if c == ':' {
                after_colon = true;
            } else if c == '#' {
                if after_colon {
                    hash = Some(from + keyword.len() + j);
                    break;
                }
            } else if space(c) {
                after_colon = false;
            }
        }
        if let Some(hash) = hash
            && let Some(semi) = text.get(..end).and_then(|head| head.rfind(';'))
            && semi > hash
        {
            out.push_str(text.get(last..semi).unwrap_or(""));
            last = semi + 1;
        }

        if end >= text.len() {
            break;
        }
        let next = end + width;
        match text.get(next..).and_then(|rest| rest.find(keyword)) {
            Some(at) => from = next + at,
            None => break,
        }
    }

    if last == 0 {
        return Cow::Borrowed(text);
    }
    out.push_str(text.get(last..).unwrap_or(""));
    Cow::Owned(out)
}

/// Hides `#name;` entity codes behind Mermaid's placeholder characters so `#` and `;` survive
/// parsing.
pub(crate) fn encode_entities(text: &str) -> Cow<'_, str> {
    if !text.contains('#') {
        return Cow::Borrowed(text);
    }

    let styled = drop_color_semicolons(text, "style");
    let dropped = drop_color_semicolons(&styled, "classDef");
    let mut out = String::with_capacity(dropped.len() + 16);
    let mut last = 0;
    let mut at = 0;
    while let Some(offset) = dropped.get(at..).and_then(|rest| rest.find('#')) {
        let hash = at + offset;
        let rest = dropped.get(hash + 1..).unwrap_or("");
        let inner = rest
            .get(..rest.len() - rest.trim_start_matches(word).len())
            .unwrap_or("");
        if inner.is_empty()
            || !rest
                .get(inner.len()..)
                .is_some_and(|tail| tail.starts_with(';'))
        {
            at = hash + 1;
            continue;
        }

        out.push_str(dropped.get(last..hash).unwrap_or(""));
        out.push_str(if inner.bytes().all(|b| b.is_ascii_digit()) {
            "\u{fb02}\u{b0}\u{b0}"
        } else {
            "\u{fb02}\u{b0}"
        });
        out.push_str(inner);
        out.push_str("\u{b6}\u{df}");
        last = hash + inner.len() + 2;
        at = last;
    }
    out.push_str(dropped.get(last..).unwrap_or(""));
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    //! Expectations come from running Pele's `preprocess`, `cleanupComments`, `encodeEntities`, and
    //! `detect` at 458322a, over hand-picked inputs and every source in `test/vectors`.

    use super::{Value, cleanup_comments, encode_entities, json, preprocess};
    use crate::detect::detect;
    use crate::yaml::tests::{push_json, push_json_string};

    fn summary(source: &str) -> String {
        match preprocess(source, false) {
            Ok(result) => {
                let mut out = String::from("[");
                push_json_string(&mut out, &result.text);
                out.push(',');
                match &result.title {
                    Some(title) => push_json_string(&mut out, title),
                    None => out.push_str("null"),
                }
                out.push(',');
                push_json(&mut out, &Value::Object(result.config));
                out.push(',');
                push_json_string(&mut out, &result.with_comments);
                out.push(',');
                out.push_str(&result.line_offset.to_string());
                out.push(']');
                out
            }
            Err(error) => format!("ERR:{}", error.message),
        }
    }

    // FNV-1a over UTF-16 units.
    fn fnv(text: &str) -> String {
        let hash = text.encode_utf16().fold(0x811c_9dc5_u32, |hash, unit| {
            (hash ^ u32::from(unit)).wrapping_mul(0x0100_0193)
        });
        format!("{hash:08x}")
    }

    #[test]
    fn preprocess_matches_pele() {
        for &(source, expected) in CASES {
            assert_eq!(summary(source), expected, "{source:?}");
        }
    }

    #[test]
    fn cleanup_comments_matches_pele() {
        for &(source, expected) in CLEANUP {
            assert_eq!(cleanup_comments(source), expected, "{source:?}");
        }
    }

    #[test]
    fn encode_entities_matches_pele() {
        for &(source, expected) in ENCODE {
            assert_eq!(encode_entities(source), expected, "{source:?}");
        }
    }

    #[test]
    fn encode_entities_stays_linear() {
        let long = format!("graph TD\nstyle A {}", "style:x#".repeat(20000));
        assert_eq!(encode_entities(&long), long);
    }

    #[test]
    fn cleanup_comments_skips_blank_runs_once() {
        let blank = format!("A{}%% c\nB", "\n".repeat(200_000));
        assert_eq!(cleanup_comments(&blank), "A\nB");
    }

    #[test]
    fn corpus_matches_pele() {
        let mut sources = Vec::new();
        for file in CORPUS_FILES {
            let Some(Value::Array(items)) = json(file) else {
                panic!("corpus file is not an array");
            };
            for item in items {
                let Value::String(source) = item else {
                    panic!("corpus entry is not a string");
                };
                sources.push(source);
            }
        }
        let Some(Value::Array(groups)) = json(include_str!("../../../test/vectors/samples.json"))
        else {
            panic!("samples.json is not an array");
        };
        for group in groups {
            let Value::Object(group) = group else {
                panic!("sample group is not an object");
            };
            let Some((_, Value::Array(samples))) = group.into_iter().find(|e| e.0 == "samples")
            else {
                panic!("sample group has no samples");
            };
            for sample in samples {
                let Value::Object(sample) = sample else {
                    panic!("sample is not an object");
                };
                let Some((_, Value::String(source))) = sample.into_iter().find(|e| e.0 == "source")
                else {
                    panic!("sample has no source");
                };
                sources.push(source);
            }
        }

        let expected = CORPUS_EXPECTED.split('\n').collect::<Vec<_>>();
        assert_eq!(sources.len(), expected.len());
        for (source, expected) in sources.iter().zip(expected) {
            let actual = match preprocess(source, false) {
                Ok(result) => {
                    let mut config = String::new();
                    push_json(&mut config, &Value::Object(result.config));
                    format!(
                        "{}\t{}\t{}\t{}\t{}\t{config}",
                        detect(&result.text).unwrap_or("null"),
                        fnv(&result.text),
                        fnv(&result.with_comments),
                        result.line_offset,
                        result.title.unwrap_or_default(),
                    )
                }
                Err(error) => format!("ERR\t{}", error.message),
            };
            assert_eq!(actual, expected, "{source:?}");
        }
    }

    const CASES: &[(&str, &str)] = &[
        (
            "---\ntitle: My chart\nconfig:\n  flowchart:\n    curve: linear\n---\ngraph TD\n  A",
            "[\"graph TD\\n  A\",\"My chart\",{\"flowchart\":{\"curve\":\"linear\"}},\"graph TD\\n  A\",6]",
        ),
        (
            "---\nconfig:\n  flowchart:\n    curve: linear\n    rankSpacing: 10\n---\n%%{init: {'flowchart': {'rankSpacing': 99}}}%%\ngraph TD\n  A",
            "[\"graph TD\\n  A\",null,{\"flowchart\":{\"curve\":\"linear\",\"rankSpacing\":99}},\"\\ngraph TD\\n  A\",6]",
        ),
        (
            "%%{init: {\"__proto__\": {\"polluted\": true}, \"constructor\": {\"prototype\": {\"polluted\": true}}}}%%\ngraph TD\nA",
            "[\"graph TD\\nA\",null,{},\"\\ngraph TD\\nA\",0]",
        ),
        (
            "---\nconfig:\n  __proto__:\n    polluted: true\n  ok: 1\n---\ngraph TD\nA",
            "[\"graph TD\\nA\",null,{\"ok\":1},\"graph TD\\nA\",6]",
        ),
        (
            "graph TD\r\nA[\"<span class=\"x\">hi</span>\"]\r",
            "[\"graph TD\\nA[\\\"<span class='x'>hi</span>\\\"]\\n\",null,{},\"graph TD\\nA[\\\"<span class='x'>hi</span>\\\"]\\n\",0]",
        ),
        (
            "graph TD\nA[\"<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a\"]",
            "[\"graph TD\\nA[\\\"<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a\\\"]\",null,{},\"graph TD\\nA[\\\"<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a<a\\\"]\",0]",
        ),
        (
            "---\ntitle: 3.14\ndisplayMode: compact\nconfig:\n  gantt: 5\n---\ngantt",
            "[\"gantt\",\"3.14\",{\"gantt\":{\"displayMode\":\"compact\"}},\"gantt\",6]",
        ),
        (
            "---\ndisplayMode: compact\nconfig:\n  gantt:\n    barHeight: 3\n---\ngantt",
            "[\"gantt\",null,{\"gantt\":{\"barHeight\":3,\"displayMode\":\"compact\"}},\"gantt\",6]",
        ),
        (
            "---\ndisplayMode: 0\n---\ngantt",
            "[\"gantt\",null,{},\"gantt\",3]",
        ),
        (
            "---\ntitle: [a, [b, null], {c: 1}, true, 1e21]\n---\nA",
            "[\"A\",\"a,b,,[object Object],true,1e+21\",{},\"A\",3]",
        ),
        ("---\ntitle: 0\n---\nA", "[\"A\",null,{},\"A\",3]"),
        ("---\ntitle: \"\"\n---\nA", "[\"A\",null,{},\"A\",3]"),
        (
            "---\ntitle: {a: 1}\n---\nA",
            "[\"A\",\"[object Object]\",{},\"A\",3]",
        ),
        ("---\ntitle: .nan\n---\nA", "[\"A\",null,{},\"A\",3]"),
        (
            "  ---\n  title: x\n  config:\n    a: 1\n  ---\nA",
            "[\"A\",\"x\",{\"a\":1},\"A\",5]",
        ),
        ("\t---\n\ttitle: x\n\t---\nA", "[\"A\",\"x\",{},\"A\",3]"),
        (
            "---\ntitle: x\n---",
            "[\"---\\ntitle: x\\n---\",null,{},\"---\\ntitle: x\\n---\",0]",
        ),
        ("---\ntitle: x\n---\n", "[\"\",\"x\",{},\"\",3]"),
        (
            "---\n---\ngraph",
            "[\"---\\n---\\ngraph\",null,{},\"---\\n---\\ngraph\",0]",
        ),
        (
            "---\na: [1\n---\ngraph",
            "ERR:Front matter is not valid YAML. YAML: unexpected end of the stream within a flow collection",
        ),
        (
            "---\na: \"\\q\"\n---\ngraph",
            "ERR:Front matter is not valid YAML. YAML: unknown escape sequence",
        ),
        (
            "---\na: \"\\xZZ\"\n---\ngraph",
            "ERR:Front matter is not valid YAML. Invalid code point NaN",
        ),
        (
            "--- \n a: 1\n--- \n\ngraph",
            "[\"graph\",null,{},\"graph\",4]",
        ),
        (
            "---\na: 1\n---\nb: 2\n---\ngraph",
            "[\"b: 2\\n---\\ngraph\",null,{},\"b: 2\\n---\\ngraph\",3]",
        ),
        (
            "---\na: 1\n  ---\ngraph",
            "[\"---\\na: 1\\n  ---\\ngraph\",null,{},\"---\\na: 1\\n  ---\\ngraph\",0]",
        ),
        (
            "\n\n---\ntitle: t\n---\nA",
            "[\"---\\ntitle: t\\n---\\nA\",null,{},\"\\n\\n---\\ntitle: t\\n---\\nA\",0]",
        ),
        (
            "---title: x\n---\nA",
            "[\"---title: x\\n---\\nA\",null,{},\"---title: x\\n---\\nA\",0]",
        ),
        (
            "---\n\n\ntitle: x\n\n---\n\n\nA\nB",
            "[\"A\\nB\",\"x\",{},\"A\\nB\",8]",
        ),
        (
            "---\r\ntitle: crlf\r\n---\r\nA\r\nB",
            "[\"A\\nB\",\"crlf\",{},\"A\\nB\",3]",
        ),
        ("---\ntitle: x\n---  \t\nA", "[\"A\",\"x\",{},\"A\",3]"),
        (
            "---\ntitle: x\n----\n---\nA",
            "ERR:Front matter is not valid YAML. YAML: can not read a block mapping entry",
        ),
        (
            "---\ntitle: x\nconfig: 5\n---\nA",
            "[\"A\",\"x\",{},\"A\",4]",
        ),
        ("---\n- a\n- b\n---\nA", "[\"A\",null,{},\"A\",4]"),
        ("---\njust text\n---\nA", "[\"A\",null,{},\"A\",3]"),
        (
            "---\ntitle: a\n---\n---\ntitle: b\n---\nA",
            "[\"---\\ntitle: b\\n---\\nA\",\"a\",{},\"---\\ntitle: b\\n---\\nA\",3]",
        ),
        (
            "%%{init: {'theme':'dark'}}%%\n%%{wrap}%%\ngraph",
            "[\"graph\",null,{\"theme\":\"dark\",\"wrap\":true},\"\\n\\ngraph\",0]",
        ),
        (
            "%%{initialize: {\"a\": {\"b\": 1}}}%%\ngraph",
            "[\"graph\",null,{\"a\":{\"b\":1}},\"\\ngraph\",0]",
        ),
        (
            "%%{init: {'a': 1}}%% %%{init: {'a': {'c': 2}}}%%\ngraph",
            "[\"graph\",null,{\"a\":{\"c\":2}},\" \\ngraph\",0]",
        ),
        (
            "%%{init: {'a': {'c': 2}}}%% %%{init: {'a': 1}}%%\ngraph",
            "[\"graph\",null,{\"a\":1},\" \\ngraph\",0]",
        ),
        (
            "%%{init: bad json}%%\ngraph",
            "[\"json}%%\\ngraph\",null,{},\"json}%%\\ngraph\",0]",
        ),
        (
            "%%{ init : { \"x\" : [1, 2.5e3, -0, true, null, \"s\"] } }%%\ngraph",
            "[\"graph\",null,{\"x\":[1,2500,0,true,null,\"s\"]},\"\\ngraph\",0]",
        ),
        (
            "%%{config: {\"a\":1}}%%\ngraph",
            "[\"graph\",null,{},\"\\ngraph\",0]",
        ),
        (
            "%%{init: abc def}%%\ngraph",
            "[\"def}%%\\ngraph\",null,{},\"def}%%\\ngraph\",0]",
        ),
        (
            "%%{init: {\"__proto__\": {\"p\": 1}, \"constructor\": 1, \"ok\": 1}}%%\ngraph",
            "[\"graph\",null,{\"ok\":1},\"\\ngraph\",0]",
        ),
        (
            "%%{init: {\"2\": 1, \"b\": 2, \"1\": 3, \"01\": 4}}%%\ngraph",
            "[\"graph\",null,{\"1\":3,\"2\":1,\"b\":2,\"01\":4},\"\\ngraph\",0]",
        ),
        (
            "%%{init:\n{\"a\":\n1}\n}%%\ngraph",
            "[\"graph\",null,{\"a\":1},\"\\ngraph\",0]",
        ),
        (
            "%%{init: {\"s\": \"\\u00e9\\ud83d\\ude00\\n\\\"q\\\"\"}}%%\ngraph",
            "[\"graph\",null,{\"s\":\"é😀\\n\\\"q\\\"\"},\"\\ngraph\",0]",
        ),
        (
            "%%{wrap}%%%%{init: {\"a\":1}}%%\ngraph",
            "[\"graph\",null,{\"wrap\":true,\"a\":1},\"\\ngraph\",0]",
        ),
        ("%%{init: {\"a\": 1}\ngraph", "[\"\",null,{},\"\",0]"),
        (
            "%%{init: {\"a\": 01}}%%\ngraph",
            "[\"graph\",null,{},\"\\ngraph\",0]",
        ),
        (
            "%%{init: {\"a\": 1,}}%%\ngraph",
            "[\"graph\",null,{},\"\\ngraph\",0]",
        ),
        (
            "%%{init: [1, 2]}%%\ngraph",
            "[\"graph\",null,{},\"\\ngraph\",0]",
        ),
        (
            "%%{init: \"x\"}%%\ngraph",
            "[\"graph\",null,{},\"\\ngraph\",0]",
        ),
        ("%%{init}%%\ngraph", "[\"graph\",null,{},\"\\ngraph\",0]"),
        (
            "%%{ }%%\ngraph",
            "[\"%%{ }%%\\ngraph\",null,{},\"%%{ }%%\\ngraph\",0]",
        ),
        (
            "%%{wrap: true}%%\ngraph",
            "[\"graph\",null,{\"wrap\":true},\"\\ngraph\",0]",
        ),
        (
            "graph\n  %%{init: {\"a\": 1}}%%\n  A",
            "[\"graph\\n  \\n  A\",null,{\"a\":1},\"graph\\n  \\n  A\",0]",
        ),
        (
            "graph %%{init: {\"a\": 1}}%% A",
            "[\"graph  A\",null,{\"a\":1},\"graph  A\",0]",
        ),
        (
            "%%{init: {'label': 'it''s'}}%%\ngraph",
            "[\"graph\",null,{},\"\\ngraph\",0]",
        ),
        (
            "%%{init: {\"a\": \"x}%%y\"}}%%\ngraph",
            "[\"y\\\"}}%%\\ngraph\",null,{},\"y\\\"}}%%\\ngraph\",0]",
        ),
        (
            "%%{init: {\"a\": {\"b\": 1, \"c\": 2}}}%%\n%%{init: {\"a\": {\"b\": 3}}}%%\ngraph",
            "[\"graph\",null,{\"a\":{\"b\":3,\"c\":2}},\"\\n\\ngraph\",0]",
        ),
        (
            "---\nconfig:\n  a:\n    b: 1\n---\n%%{init: {\"a\": {\"c\": 2}, \"wrap\": false}}%%\n%%{wrap}%%\ngraph",
            "[\"graph\",null,{\"a\":{\"b\":1,\"c\":2},\"wrap\":true},\"\\n\\ngraph\",5]",
        ),
        (
            "A[\"<b class=\"x\" id=\"y\">t</b>\"] --> B<br/>",
            "[\"A[\\\"<b class='x' id='y'>t</b>\\\"] --> B<br/>\",null,{},\"A[\\\"<b class='x' id='y'>t</b>\\\"] --> B<br/>\",0]",
        ),
        ("<<a b=\"c\">", "[\"<<a b='c'>\",null,{},\"<<a b='c'>\",0]"),
        ("a < b > c", "[\"a < b > c\",null,{},\"a < b > c\",0]"),
        (
            "<a href=\"x>",
            "[\"<a href=\\\"x>\",null,{},\"<a href=\\\"x>\",0]",
        ),
        (
            "x <tag attr=\"no close>\"",
            "[\"x <tag attr=\\\"no close>\\\"\",null,{},\"x <tag attr=\\\"no close>\\\"\",0]",
        ),
        (
            "<a x=\"1\" y=\"2\" z='3'>",
            "[\"<a x='1' y='2' z='3'>\",null,{},\"<a x='1' y='2' z='3'>\",0]",
        ),
        ("<1 a=\"b\">", "[\"<1 a='b'>\",null,{},\"<1 a='b'>\",0]"),
        (
            "<é a=\"b\">",
            "[\"<é a=\\\"b\\\">\",null,{},\"<é a=\\\"b\\\">\",0]",
        ),
        (
            "A[\"<i a=\"\"></i>\"]",
            "[\"A[\\\"<i a=''></i>\\\"]\",null,{},\"A[\\\"<i a=''></i>\\\"]\",0]",
        ),
        (">", "[\">\",null,{},\">\",0]"),
        ("<>", "[\"<>\",null,{},\"<>\",0]"),
        (
            "%% one\ngraph TD\n  %% two\n  A --> B\n",
            "[\"graph TD\\n  A --> B\\n\",null,{},\"%% one\\ngraph TD\\n  %% two\\n  A --> B\\n\",0]",
        ),
        ("%%\ngraph", "[\"%%\\ngraph\",null,{},\"%%\\ngraph\",0]"),
        ("%%{x}\ngraph", "[\"\",null,{},\"\",0]"),
        (
            "  \n\n  %% c\nA",
            "[\"A\",null,{},\"  \\n\\n  %% c\\nA\",0]",
        ),
        (
            "A %% not a comment",
            "[\"A %% not a comment\",null,{},\"A %% not a comment\",0]",
        ),
        ("A\r\n%% c\r\nB", "[\"A\\nB\",null,{},\"A\\n%% c\\nB\",0]"),
        ("a\rb\r\nc", "[\"a\\nb\\nc\",null,{},\"a\\nb\\nc\",0]"),
        ("%% c", "[\"\",null,{},\"%% c\",0]"),
        (
            "%% c\n%% d\n\n%% e\nA",
            "[\"A\",null,{},\"%% c\\n%% d\\n\\n%% e\\nA\",0]",
        ),
        ("   graph", "[\"graph\",null,{},\"   graph\",0]"),
        (
            "\n\n\ngraph TD",
            "[\"graph TD\",null,{},\"\\n\\n\\ngraph TD\",0]",
        ),
        (
            "graph\n%%% x\nA",
            "[\"graph\\nA\",null,{},\"graph\\n%%% x\\nA\",0]",
        ),
        (
            "graph\n %%\n A",
            "[\"graph\\n %%\\n A\",null,{},\"graph\\n %%\\n A\",0]",
        ),
        (
            "graph\n%% a\u{2028}b\nA",
            "[\"graph\\nA\",null,{},\"graph\\n%% a\u{2028}b\\nA\",0]",
        ),
    ];

    const CLEANUP: &[(&str, &str)] = &[
        ("%% x", ""),
        (" \n %% y\nz", "z"),
        ("%%{init}%%\n%% c\nx", "%%{init}%%\nx"),
        ("a\u{2028}%% c\nb", "a\u{2028}b"),
        ("%%x", ""),
        ("a\n%%", "a\n%%"),
        ("a\n%%\n", "a\n%%\n"),
        ("%%%%", ""),
        ("\u{a0}graph", "graph"),
        ("x %% y", "x %% y"),
        ("%% a\n\n%% b", ""),
        ("\t%% a\nb", "b"),
    ];

    const ENCODE: &[(&str, &str)] = &[
        (
            "style A fill:#f9f,stroke:#333;",
            "style A fill:#f9f,stroke:#333",
        ),
        (
            "style A fill:#f9f;stroke:#333;\nB --> C;",
            "style A fill:ﬂ°f9f¶ßstroke:#333\nB --> C;",
        ),
        (
            "classDef green fill:#9f6,stroke:#333,stroke-width:2px;",
            "classDef green fill:#9f6,stroke:#333,stroke-width:2px",
        ),
        (
            "A[\"a #35; b #quot;c#quot;\"]",
            "A[\"a ﬂ°°35¶ß b ﬂ°quot¶ßcﬂ°quot¶ß\"]",
        ),
        ("style A color: #fff;", "style A color: ﬂ°fff¶ß"),
        ("style A fill:red;", "style A fill:red;"),
        (
            "mystyle x:#1; classDef y:#2; z;",
            "mystyle x:ﬂ°°1¶ß classDef y:#2 z",
        ),
        ("style a:#b\n;", "style a:#b\n;"),
        ("style a b :c#d e;f;g", "style a b :c#d e;fg"),
        ("#;# ; #a ;#a;", "#;# ; #a ;ﬂ°a¶ß"),
        (
            "style A fill:#f9f;\u{2028}x;",
            "style A fill:#f9f\u{2028}x;",
        ),
        ("style A fill:#f9f\u{2028}x;", "style A fill:#f9f\u{2028}x;"),
        ("classDef c fill:#f00 ;é;", "classDef c fill:#f00 ;é"),
        (
            "A[\"#9829; #infin; #a_b; #12a; #+1; #é;\"]",
            "A[\"ﬂ°°9829¶ß ﬂ°infin¶ß ﬂ°a_b¶ß ﬂ°12a¶ß #+1; #é;\"]",
        ),
        ("no hash here", "no hash here"),
        ("style\nstyle a:#1;", "style\nstyle a:#1"),
        ("stylestyle:#;;", "stylestyle:#;"),
        (
            "#:classDef:\r\u{2028}:# \n9style",
            "#:classDef:\r\u{2028}:# \n9style",
        ),
        (
            "style#a#f9f;style\nclassDef\u{2028}classDef",
            "style#aﬂ°f9f¶ßstyle\nclassDef\u{2028}classDef",
        ),
        (":style\n\t:\téfill style:#a", ":style\n\t:\téfill style:#a"),
        (
            "9a:# #stylefillfill#:#\u{2028}:#",
            "9a:# #stylefillfill#:#\u{2028}:#",
        ),
        ("", ""),
        (":#\t99#f9f; #é\u{2028}", ":#\t99ﬂ°f9f¶ß #é\u{2028}"),
        ("\n", "\n"),
        (
            ":#:\t\t#f9f;\nclassDefé#f9f;",
            ":#:\t\tﬂ°f9f¶ß\nclassDeféﬂ°f9f¶ß",
        ),
        ("\r ;:#fill:#", "\r ;:#fill:#"),
        (
            "\u{2028}fillclassDef#f9f;classDef#\tclassDef:#\u{2028}",
            "\u{2028}fillclassDefﬂ°f9f¶ßclassDef#\tclassDef:#\u{2028}",
        ),
        (":#:aclassDef\u{2028}", ":#:aclassDef\u{2028}"),
        ("#\t\nastyle\t:#", "#\t\nastyle\t:#"),
        (":a\t", ":a\t"),
        (
            ":#astylestylefill9\néclassDefclassDef\té",
            ":#astylestylefill9\néclassDefclassDef\té",
        ),
        ("é\nclassDef\t#f9f; :#", "é\nclassDef\tﬂ°f9f¶ß :#"),
        ("style;#f9f;#é:#", "style;ﬂ°f9f¶ß#é:#"),
        (" \n\r\t\u{2028} ", " \n\r\t\u{2028} "),
        ("classDef\n\r", "classDef\n\r"),
        ("\tclassDef", "\tclassDef"),
        ("a#\tstyle\né:;", "a#\tstyle\né:;"),
        ("style:#", "style:#"),
        ("#é##f9f;a ", "#é#ﬂ°f9f¶ßa "),
        (":;", ":;"),
        ("", ""),
        ("#style \nstyle9fill", "#style \nstyle9fill"),
        (
            "\n afill:#fill;stylefill\rstyle9:",
            "\n afill:ﬂ°fill¶ßstylefill\rstyle9:",
        ),
        ("fill#é#\r\t\t\ta;\r", "fill#é#\r\t\t\ta;\r"),
        ("style9\r\u{2028}\té\ra:\t ", "style9\r\u{2028}\té\ra:\t "),
        ("", ""),
        (
            "\ra9#;:#fill#f9f;classDef\r",
            "\ra9#;:#fillﬂ°f9f¶ßclassDef\r",
        ),
        ("fill;styleé9:#:##a", "fill;styleé9:#:##a"),
        ("a", "a"),
        ("classDef", "classDef"),
        ("", ""),
        ("#f9f;\nstyle:#fill", "ﬂ°f9f¶ß\nstyle:#fill"),
        ("\t", "\t"),
        (
            ";# \n\u{2028}\n#f9f;#fillclassDef",
            ";# \n\u{2028}\nﬂ°f9f¶ß#fillclassDef",
        ),
        ("9", "9"),
        ("\u{2028}classDefé:\rfill", "\u{2028}classDefé:\rfill"),
        ("\t", "\t"),
        (
            ":fill\u{2028}style \r#9\n\rclassDefa",
            ":fill\u{2028}style \r#9\n\rclassDefa",
        ),
        (
            "9\u{2028}aa#f9f;9\n\u{2028}:",
            "9\u{2028}aaﬂ°f9f¶ß9\n\u{2028}:",
        ),
        (
            "#f9f;#f9f;:##f9f;#f9f;:aclassDef:#",
            "ﬂ°f9f¶ßﬂ°f9f¶ß:#ﬂ°f9f¶ßﬂ°f9f¶ß:aclassDef:#",
        ),
        (
            "\t\u{2028}astyleé\t##f9f; a:\r",
            "\t\u{2028}astyleé\t#ﬂ°f9f¶ß a:\r",
        ),
        (
            ":#classDef\r:#:style 9\u{2028}",
            ":#classDef\r:#:style 9\u{2028}",
        ),
        ("fill#é:#style:#é\tstylefill", "fill#é:#style:#é\tstylefill"),
        (":#a\rstylestylefill", ":#a\rstylestylefill"),
        (
            "a\n\n\nclassDef\rclassDefstyleclassDef:",
            "a\n\n\nclassDef\rclassDefstyleclassDef:",
        ),
        ("style #:#", "style #:#"),
        ("fill\n:\u{2028}", "fill\n:\u{2028}"),
        (" style;style;#f9f;:é\n#", " style;style;ﬂ°f9f¶ß:é\n#"),
        ("::fillclassDef9classDef9;", "::fillclassDef9classDef9;"),
        ("", ""),
        ("#f9f;stylefill:#a", "ﬂ°f9f¶ßstylefill:#a"),
        ("afill###f9f;style\t\n:# ", "afill##ﬂ°f9f¶ßstyle\t\n:# "),
        ("#f9f;", "ﬂ°f9f¶ß"),
        ("\tstyle9\r#", "\tstyle9\r#"),
        (
            "\u{2028}\r\t\u{2028}a##f9f;9\n classDef",
            "\u{2028}\r\t\u{2028}a#ﬂ°f9f¶ß9\n classDef",
        ),
        (
            "fill#f9f;\t#\nclassDef#f9f;éé",
            "fillﬂ°f9f¶ß\t#\nclassDefﬂ°f9f¶ßéé",
        ),
        (" 9\r\ta", " 9\r\ta"),
        ("", ""),
        ("\ra##f9f;", "\ra#ﬂ°f9f¶ß"),
        (":", ":"),
        ("#a\r9;style#é99 :#classDef", "#a\r9;style#é99 :#classDef"),
        (
            "#classDef\t; \néa\t\u{2028}fill",
            "#classDef\t; \néa\t\u{2028}fill",
        ),
        (
            "9\nfillfill\r\u{2028}:#classDef",
            "9\nfillfill\r\u{2028}:#classDef",
        ),
        (":#style# 9\r", ":#style# 9\r"),
        ("style\r", "style\r"),
        (
            "#f9f;#f9f;9fill\nfill9\u{2028}#f9f;classDef",
            "ﬂ°f9f¶ßﬂ°f9f¶ß9fill\nfill9\u{2028}ﬂ°f9f¶ßclassDef",
        ),
        ("style", "style"),
        (";;:##:#\u{2028}\t:\u{2028}", ";;:##:#\u{2028}\t:\u{2028}"),
        (
            " #f9f;9:# \u{2028}styleéa:#",
            " ﬂ°f9f¶ß9:# \u{2028}styleéa:#",
        ),
        ("classDefa\n\r:#f9f;", "classDefa\n\r:ﬂ°f9f¶ß"),
        (
            "style\u{2028}fill:#classDefa\t:;\t; é",
            "style\u{2028}fill:#classDefa\t:;\t; é",
        ),
        ("#classDeffillclassDef#;\t", "#classDeffillclassDef#;\t"),
        (" ;", " ;"),
        (
            "fill:#\n\n é\r9classDef\u{2028}filla",
            "fill:#\n\n é\r9classDef\u{2028}filla",
        ),
        (
            "fillé\r\ra\u{2028}\r\rstyle\r#f9f;;",
            "fillé\r\ra\u{2028}\r\rstyle\rﬂ°f9f¶ß;",
        ),
        (
            "\rclassDefstyle:\rfillclassDef",
            "\rclassDefstyle:\rfillclassDef",
        ),
        ("", ""),
        ("\rstyle", "\rstyle"),
        ("fill", "fill"),
        (" classDefstyle\u{2028}:", " classDefstyle\u{2028}:"),
        ("\na9a", "\na9a"),
        ("stylea#classDef:filla;", "stylea#classDef:filla;"),
        ("\na\r :\r", "\na\r :\r"),
        (":\n;", ":\n;"),
        ("é:é:\tclassDef", "é:é:\tclassDef"),
        ("é9:#éfill;#f9f;#\t", "é9:#éfill;ﬂ°f9f¶ß#\t"),
        (
            "#\n\r\r#f9f; classDef classDeffill\u{2028} \t",
            "#\n\r\rﬂ°f9f¶ß classDef classDeffill\u{2028} \t",
        ),
        ("#9\tstylestyle#", "#9\tstylestyle#"),
        ("é\té#", "é\té#"),
        ("\r", "\r"),
        ("\r\u{2028} ::::#:\n:fill:", "\r\u{2028} ::::#:\n:fill:"),
        (
            "\u{2028}é\u{2028}:#:fillclassDef\u{2028}#",
            "\u{2028}é\u{2028}:#:fillclassDef\u{2028}#",
        ),
        ("a99\r 9", "a99\r 9"),
        (";\u{2028}\n9:#fillfill é:#", ";\u{2028}\n9:#fillfill é:#"),
        (
            "fill\r:\u{2028}a#\u{2028}#f9f;\rfill classDef",
            "fill\r:\u{2028}a#\u{2028}ﬂ°f9f¶ß\rfill classDef",
        ),
        (";;é", ";;é"),
        ("", ""),
        (
            ":\r9é#f9f;classDefé#f9f;:#",
            ":\r9éﬂ°f9f¶ßclassDeféﬂ°f9f¶ß:#",
        ),
        ("#style:\téfill9classDef", "#style:\téfill9classDef"),
        ("style\r99é", "style\r99é"),
        ("style;fill\tstyle#:#\r9", "style;fill\tstyle#:#\r9"),
        (
            ":#fill#f9f;style classDef#f9f;a\rclassDef;\r;",
            ":#fillﬂ°f9f¶ßstyle classDefﬂ°f9f¶ßa\rclassDef;\r;",
        ),
        ("\t:#aclassDef style\r", "\t:#aclassDef style\r"),
        ("#", "#"),
        ("9", "9"),
        ("9;\néfill", "9;\néfill"),
        (";", ";"),
        (
            "#f9f;\tclassDefclassDef##f9f;",
            "ﬂ°f9f¶ß\tclassDefclassDef#ﬂ°f9f¶ß",
        ),
        ("\u{2028}: ", "\u{2028}: "),
        (
            ":#:#classDef fill:##f9f;stylestyle :#\n",
            ":#:#classDef fill:##f9fstylestyle :#\n",
        ),
        ("", ""),
        (":#\n", ":#\n"),
        (":#\n;;9\r\n;fill", ":#\n;;9\r\n;fill"),
        (
            "9#f9f;:#style;;\u{2028}fill\t:",
            "9ﬂ°f9f¶ß:ﬂ°style¶ß;\u{2028}fill\t:",
        ),
        ("\tstylefill:\r;#f9f;aé", "\tstylefill:\r;ﬂ°f9f¶ßaé"),
        ("\t#f9f;éaéclassDef#f9f;a", "\tﬂ°f9f¶ßéaéclassDefﬂ°f9f¶ßa"),
        (":\r", ":\r"),
        (
            ":#classDefa#f9f;stylefill\n  9",
            ":#classDefaﬂ°f9f¶ßstylefill\n  9",
        ),
        (
            ":classDef9:classDef\u{2028}:#é:",
            ":classDef9:classDef\u{2028}:#é:",
        ),
        ("#style\n#f9f;", "#style\nﬂ°f9f¶ß"),
        ("\r:\n\rfillstyle\n ", "\r:\n\rfillstyle\n "),
        ("é:a9##f9f;classDefstyle", "é:a9#ﬂ°f9f¶ßclassDefstyle"),
        ("stylefill\n:", "stylefill\n:"),
        ("#\n\n\n", "#\n\n\n"),
        (
            "fill:##:#fillstylefill\nclassDef fill",
            "fill:##:#fillstylefill\nclassDef fill",
        ),
        (" fill\t#;", " fill\t#;"),
        ("\n :", "\n :"),
        (":#:style;9", ":#:style;9"),
        ("", ""),
        ("é#;9:#\n99", "é#;9:#\n99"),
        ("\t#f9f;\u{2028} ", "\tﬂ°f9f¶ß\u{2028} "),
        (
            "style\n#f9f;\u{2028};;fill:#a\t:#\tfill",
            "style\nﬂ°f9f¶ß\u{2028};;fill:#a\t:#\tfill",
        ),
        (
            "\nclassDef\t:##f9f;\t\u{2028}#\u{2028}:#f9f;",
            "\nclassDef\t:##f9f\t\u{2028}#\u{2028}:ﬂ°f9f¶ß",
        ),
        (
            "fill:#\u{2028}fillclassDef:#:#\rfill;",
            "fill:#\u{2028}fillclassDef:#:#\rfill;",
        ),
        ("classDef9é#f9f;a\n; ", "classDef9éﬂ°f9f¶ßa\n; "),
        ("style;#:#fill", "style;#:#fill"),
        ("afill:#fill;a\r9afill:#", "afill:ﬂ°fill¶ßa\r9afill:#"),
        (";#", ";#"),
        ("9#f9f;filla", "9ﬂ°f9f¶ßfilla"),
        ("\u{2028}:#a", "\u{2028}:#a"),
        ("#f9f;aa:9;", "ﬂ°f9f¶ßaa:9;"),
        ("classDef\u{2028}\t\n\t", "classDef\u{2028}\t\n\t"),
        ("#classDeffill", "#classDeffill"),
        ("\té:#classDef ;", "\té:#classDef ;"),
        (
            "#f9f;fill\nclassDef\u{2028}:\n#f9f;classDef:##f9f;",
            "ﬂ°f9f¶ßfill\nclassDef\u{2028}:\nﬂ°f9f¶ßclassDef:##f9f",
        ),
        ("#f9f;ééa", "ﬂ°f9f¶ßééa"),
        ("", ""),
        ("styleé", "styleé"),
        ("", ""),
        (";\u{2028}éstyle ", ";\u{2028}éstyle "),
        (" styleéfillé", " styleéfillé"),
        (
            ":# stylestyle\u{2028}\r\n;fill:",
            ":# stylestyle\u{2028}\r\n;fill:",
        ),
        ("fill fill:9\u{2028}##", "fill fill:9\u{2028}##"),
        (
            "#classDef\r9;a:#fill\n\r:style",
            "#classDef\r9;a:#fill\n\r:style",
        ),
        ("#\t", "#\t"),
        ("\t\n\rstyle \u{2028} é", "\t\n\rstyle \u{2028} é"),
        ("9classDeffill", "9classDeffill"),
        (
            "9fillfill\u{2028}#f9f;#f9f; \u{2028}:#",
            "9fillfill\u{2028}ﬂ°f9f¶ßﬂ°f9f¶ß \u{2028}:#",
        ),
        ("classDef\n99:\r;;:#a", "classDef\n99:\r;;:#a"),
        ("\nclassDef\n9", "\nclassDef\n9"),
        ("9", "9"),
        ("\nstyle ;9\r#f9f; a9", "\nstyle ;9\rﬂ°f9f¶ß a9"),
        (":", ":"),
        (
            "fill styleclassDef#f9f;\u{2028}classDefa:#stylestyle9style",
            "fill styleclassDefﬂ°f9f¶ß\u{2028}classDefa:#stylestyle9style",
        ),
        ("\u{2028} ", "\u{2028} "),
        (
            "#f9f;\t\na\t\u{2028}classDef",
            "ﬂ°f9f¶ß\t\na\t\u{2028}classDef",
        ),
        ("é\tstyle\r", "é\tstyle\r"),
        ("", ""),
        (
            "\r\u{2028}#: \u{2028}é\nclassDef\u{2028}",
            "\r\u{2028}#: \u{2028}é\nclassDef\u{2028}",
        ),
        (
            "##fill\n\t\rclassDef\n#9fill",
            "##fill\n\t\rclassDef\n#9fill",
        ),
        (
            "classDef classDefé\u{2028};##f9f;;\u{2028};#f9f;",
            "classDef classDefé\u{2028};#ﬂ°f9f¶ß;\u{2028};ﬂ°f9f¶ß",
        ),
        ("", ""),
        (
            "\nstyle\u{2028}\r\r#é\n:#a\n\r#f9f;",
            "\nstyle\u{2028}\r\r#é\n:#a\n\rﬂ°f9f¶ß",
        ),
        (";\t\t", ";\t\t"),
        ("#f9f;\r\ta;;stylefill", "ﬂ°f9f¶ß\r\ta;;stylefill"),
        ("", ""),
        ("\u{2028} 9:#\u{2028}", "\u{2028} 9:#\u{2028}"),
        ("\t#f9f;style:a;fill9", "\tﬂ°f9f¶ßstyle:a;fill9"),
        (
            "\n\t;\u{2028}9classDef\u{2028}9fillé\u{2028}",
            "\n\t;\u{2028}9classDef\u{2028}9fillé\u{2028}",
        ),
        ("\r", "\r"),
        (
            "classDef\rfilla:\t\t##é##f9f;",
            "classDef\rfilla:\t\t##é#ﬂ°f9f¶ß",
        ),
        ("\r\r#fill;;éclassDef:#f9f;", "\r\rﬂ°fill¶ß;éclassDef:#f9f"),
        (
            "style\u{2028}99:é\u{2028}éfillé9\r",
            "style\u{2028}99:é\u{2028}éfillé9\r",
        ),
        (
            "style\n:#\na:#:##f9f;style\u{2028}",
            "style\n:#\na:#:#ﬂ°f9f¶ßstyle\u{2028}",
        ),
        (";\nfill#f9f;#\r\ra", ";\nfillﬂ°f9f¶ß#\r\ra"),
        (
            "a9##classDeffill é\u{2028}\n",
            "a9##classDeffill é\u{2028}\n",
        ),
        ("#f9f;:\t\u{2028}\n styleé ", "ﬂ°f9f¶ß:\t\u{2028}\n styleé "),
        ("é \t éclassDef9:", "é \t éclassDef9:"),
        (
            "classDef##\rfillastyle#f9f;:é \t",
            "classDef##\rfillastyleﬂ°f9f¶ß:é \t",
        ),
        ("", ""),
        ("#f9f;style9fill", "ﬂ°f9f¶ßstyle9fill"),
        (
            "classDef#9 \n\naclassDef\ré\u{2028}:#",
            "classDef#9 \n\naclassDef\ré\u{2028}:#",
        ),
        ("#99a\r ", "#99a\r "),
        ("\ré#f9f;classDef", "\réﬂ°f9f¶ßclassDef"),
        (
            "fillstyle:fill fillclassDefclassDefé9styleaa",
            "fillstyle:fill fillclassDefclassDefé9styleaa",
        ),
        ("", ""),
        ("\n#f9f;#\r\t", "\nﬂ°f9f¶ß#\r\t"),
        ("9 é", "9 é"),
        ("#fill:##f9f;", "#fill:#ﬂ°f9f¶ß"),
        (
            "fill\n\rstyleclassDef9#f9f;\n",
            "fill\n\rstyleclassDef9ﬂ°f9f¶ß\n",
        ),
        (
            "#\n\t:#fillfill:#classDef\n::a:#",
            "#\n\t:#fillfill:#classDef\n::a:#",
        ),
        ("classDef9é#f9f;\t\n ", "classDef9éﬂ°f9f¶ß\t\n "),
        (" classDef\r", " classDef\r"),
        (
            "\u{2028}classDefclassDefa\n:\u{2028}9é 9classDef9",
            "\u{2028}classDefclassDefa\n:\u{2028}9é 9classDef9",
        ),
        ("#\ré:9 #:##", "#\ré:9 #:##"),
        (
            "classDef\u{2028}fill;\tstyle\u{2028}styleé#;",
            "classDef\u{2028}fill;\tstyle\u{2028}styleé#;",
        ),
        ("", ""),
        ("fill\u{2028}classDef", "fill\u{2028}classDef"),
        (
            ":#f9f;#afill\u{2028}é\né\nstyle9",
            ":ﬂ°f9f¶ß#afill\u{2028}é\né\nstyle9",
        ),
        (
            "#f9f;:##\u{2028}classDef\nclassDefa",
            "ﬂ°f9f¶ß:##\u{2028}classDef\nclassDefa",
        ),
        ("fillstyle:\ré:#;\u{2028} ", "fillstyle:\ré:#;\u{2028} "),
        (
            "astyle\néé\n#f9f;#f9f;\t\u{2028}",
            "astyle\néé\nﬂ°f9f¶ßﬂ°f9f¶ß\t\u{2028}",
        ),
        (
            "\u{2028}a\tafill9#f9f;\té;classDef: ",
            "\u{2028}a\tafill9ﬂ°f9f¶ß\té;classDef: ",
        ),
        ("\u{2028}éstyle", "\u{2028}éstyle"),
        ("", ""),
        (
            "9\u{2028}9\n\t9:style\r\u{2028}9classDef",
            "9\u{2028}9\n\t9:style\r\u{2028}9classDef",
        ),
        ("#:#;\u{2028}\r", "#:#;\u{2028}\r"),
        (":", ":"),
        (" ;\t\t#f9f; style", " ;\t\tﬂ°f9f¶ß style"),
        ("\t\rclassDef\t\rstyle", "\t\rclassDef\t\rstyle"),
        ("\n9styleéé:#f9f;fill", "\n9styleéé:#f9ffill"),
        (
            "style style9#f9f;\t#f9f;\u{2028}",
            "style style9ﬂ°f9f¶ß\tﬂ°f9f¶ß\u{2028}",
        ),
        (";fill", ";fill"),
        ("#f9f;style:é", "ﬂ°f9f¶ßstyle:é"),
        ("\u{2028}#f9f;#classDef\n\n", "\u{2028}ﬂ°f9f¶ß#classDef\n\n"),
        ("styleaéstyle9:#:\r", "styleaéstyle9:#:\r"),
        (
            "\t9\tclassDef\r:;classDef\t:#a\n ",
            "\t9\tclassDef\r:;classDef\t:#a\n ",
        ),
        ("styleastyle 9:fillfill\t:#", "styleastyle 9:fillfill\t:#"),
        ("fill 9", "fill 9"),
        ("; style:#f9f;:\tstyle9", "; style:#f9f:\tstyle9"),
        (
            "9\u{2028}\t\tstyleclassDeffillclassDef",
            "9\u{2028}\t\tstyleclassDeffillclassDef",
        ),
        ("", ""),
        ("9\u{2028}\t", "9\u{2028}\t"),
        ("", ""),
        ("9;\r#f9f;\n; \t", "9;\rﬂ°f9f¶ß\n; \t"),
        ("a\r\t#f9f; ", "a\r\tﬂ°f9f¶ß "),
        (
            "a;style;9:#:#9:#\t\u{2028}\r#",
            "a;style;9:#:#9:#\t\u{2028}\r#",
        ),
        (
            ":styleclassDef:##:#9\r::fill:\u{2028}",
            ":styleclassDef:##:#9\r::fill:\u{2028}",
        ),
        (";", ";"),
        (
            "style;style:\n#f9f;a#f9f;:#f9f;classDef#é",
            "style;style:\nﬂ°f9f¶ßaﬂ°f9f¶ß:ﬂ°f9f¶ßclassDef#é",
        ),
        (" \n a\nstyle:#: ", " \n a\nstyle:#: "),
        (
            " :fill\u{2028}classDef\tclassDef\u{2028}\t",
            " :fill\u{2028}classDef\tclassDef\u{2028}\t",
        ),
        (
            "\n\t\tfill\r:fill;9:fill\u{2028}",
            "\n\t\tfill\r:fill;9:fill\u{2028}",
        ),
        ("\n", "\n"),
        ("", ""),
        ("é#f9f;#f9f;\n;é9", "éﬂ°f9f¶ßﬂ°f9f¶ß\n;é9"),
        (
            "9:# aafill##f9f;:#\t\n;\u{2028}",
            "9:# aafill#ﬂ°f9f¶ß:#\t\n;\u{2028}",
        ),
        ("\n#:#é", "\n#:#é"),
        ("é#f9f;", "éﬂ°f9f¶ß"),
        ("", ""),
        (
            "aclassDef\u{2028}; ::classDef#f9f;#\rstyleé",
            "aclassDef\u{2028}; ::classDefﬂ°f9f¶ß#\rstyleé",
        ),
        ("a#f9f;\u{2028}", "aﬂ°f9f¶ß\u{2028}"),
        (
            "fill#\t\tstyleclassDef fillé:#\té",
            "fill#\t\tstyleclassDef fillé:#\té",
        ),
        ("\u{2028}\t", "\u{2028}\t"),
        (
            ":#\r\u{2028}fillé\ta \n\u{2028}é#classDef",
            ":#\r\u{2028}fillé\ta \n\u{2028}é#classDef",
        ),
        ("9\u{2028}style; 9", "9\u{2028}style; 9"),
        ("\n:#\t\n", "\n:#\t\n"),
        (" #:#\t", " #:#\t"),
        (
            "style\r\rfill#a\n#fillclassDef",
            "style\r\rfill#a\n#fillclassDef",
        ),
        (" 9\nstylefill9:#:#", " 9\nstylefill9:#:#"),
        ("", ""),
        (":#99 9 #f9f;", ":#99 9 ﬂ°f9f¶ß"),
        ("\n\u{2028}\n#9", "\n\u{2028}\n#9"),
        (
            "styleé9:#9\n\u{2028} \u{2028}#f9f;",
            "styleé9:#9\n\u{2028} \u{2028}ﬂ°f9f¶ß",
        ),
        ("9\t\u{2028}\né\rstyle\r#\r", "9\t\u{2028}\né\rstyle\r#\r"),
        ("classDef#f9f;#:#\u{2028}", "classDefﬂ°f9f¶ß#:#\u{2028}"),
        ("styleclassDefstyle", "styleclassDefstyle"),
        (
            "a\t;stylefillstylestylestyle;##f9f;\r\r",
            "a\t;stylefillstylestylestyle;#ﬂ°f9f¶ß\r\r",
        ),
        ("é :#fill", "é :#fill"),
        (" 9#", " 9#"),
        (
            "\rstyle\u{2028}:\n\n\ré\u{2028}::#fill:#",
            "\rstyle\u{2028}:\n\n\ré\u{2028}::#fill:#",
        ),
        ("9 \u{2028}é;:#\n#f9f;\t\r", "9 \u{2028}é;:#\nﬂ°f9f¶ß\t\r"),
        (
            "#f9f;style\u{2028}#f9f;\rstylefill#\n",
            "ﬂ°f9f¶ßstyle\u{2028}ﬂ°f9f¶ß\rstylefill#\n",
        ),
        (";;\n;", ";;\n;"),
        ("", ""),
        (
            "\r:\rfillclassDefclassDef:fill\té",
            "\r:\rfillclassDefclassDef:fill\té",
        ),
        (
            "éstyleclassDef é:9 \u{2028}style;\n\r",
            "éstyleclassDef é:9 \u{2028}style;\n\r",
        ),
        ("", ""),
        ("\nstyle\r", "\nstyle\r"),
        ("\ta:\r stylestyle\tstyle", "\ta:\r stylestyle\tstyle"),
        (
            ";classDef\r\n\u{2028}\nfill\u{2028} #f9f;\t",
            ";classDef\r\n\u{2028}\nfill\u{2028} ﬂ°f9f¶ß\t",
        ),
        (
            "classDef:fill:fillfill9\t;:#é",
            "classDef:fill:fillfill9\t;:#é",
        ),
        (
            "#:fill;\n;:#classDef\r##f9f;9",
            "#:fill;\n;:#classDef\r#ﬂ°f9f¶ß9",
        ),
        (
            "#f9f;:\u{2028}9:a:#style#\r\u{2028}\r",
            "ﬂ°f9f¶ß:\u{2028}9:a:#style#\r\u{2028}\r",
        ),
        ("a", "a"),
        ("\tstyle\u{2028}\u{2028}", "\tstyle\u{2028}\u{2028}"),
        ("\tstyle\néaé\t9", "\tstyle\néaé\t9"),
        ("\t éé", "\t éé"),
        ("é9\u{2028}#;:é\u{2028}", "é9\u{2028}#;:é\u{2028}"),
        ("fill:", "fill:"),
        (
            "fill#\tclassDef#f9f;\u{2028};\t\r\r9",
            "fill#\tclassDefﬂ°f9f¶ß\u{2028};\t\r\r9",
        ),
        ("style:\u{2028}:# #", "style:\u{2028}:# #"),
        ("classDefclassDef;:#\r", "classDefclassDef;:#\r"),
        ("#f9f;9\u{2028}", "ﬂ°f9f¶ß9\u{2028}"),
        (":#style#;9 #f9f;", ":#style#;9 ﬂ°f9f¶ß"),
        ("", ""),
        ("", ""),
        (
            "é\t#f9f;\u{2028}é9:#classDefclassDefclassDef",
            "é\tﬂ°f9f¶ß\u{2028}é9:#classDefclassDefclassDef",
        ),
        (":#", ":#"),
        ("#a:fill:#classDef;9\n", "#a:fill:ﬂ°classDef¶ß9\n"),
        ("classDef:\u{2028}", "classDef:\u{2028}"),
        ("9a\t#\r", "9a\t#\r"),
        (
            "fill\r :classDef\tfillclassDef",
            "fill\r :classDef\tfillclassDef",
        ),
        (
            "\u{2028}\r# \rclassDef\n#f9f;",
            "\u{2028}\r# \rclassDef\nﬂ°f9f¶ß",
        ),
        (" classDef :#:#", " classDef :#:#"),
        ("#f9f;#f9f;fill#", "ﬂ°f9f¶ßﬂ°f9f¶ßfill#"),
        ("é", "é"),
        ("\t;é classDef\n;\r#é:", "\t;é classDef\n;\r#é:"),
        (
            ":#classDef:#f9f;classDef:#\rfillé#",
            ":#classDef:#f9fclassDef:#\rfillé#",
        ),
        (" 9stylea:classDef", " 9stylea:classDef"),
        (
            "\u{2028}9\u{2028};classDefa \t\n",
            "\u{2028}9\u{2028};classDefa \t\n",
        ),
        (
            "classDefstyleclassDefé#f9f;#f9f;\n#style\n",
            "classDefstyleclassDeféﬂ°f9f¶ßﬂ°f9f¶ß\n#style\n",
        ),
        ("éstyle\raa\r:#fill#", "éstyle\raa\r:#fill#"),
        ("", ""),
        ("é\t", "é\t"),
        (" :\t9::#classDefa9", " :\t9::#classDefa9"),
        ("éclassDefa", "éclassDefa"),
        ("", ""),
        (
            "styleclassDefstyle\u{2028}9 fillfillstylestyle",
            "styleclassDefstyle\u{2028}9 fillfillstylestyle",
        ),
        (
            " #f9f;;;\u{2028}style:#classDef",
            " ﬂ°f9f¶ß;;\u{2028}style:#classDef",
        ),
        ("classDef", "classDef"),
        (":#fill", ":#fill"),
        ("a", "a"),
        ("éé:#aa\tastyle\u{2028}:", "éé:#aa\tastyle\u{2028}:"),
        ("", ""),
        (": ", ": "),
        (
            "#f9f;\u{2028}é#;\u{2028}fillé#f9f; ",
            "ﬂ°f9f¶ß\u{2028}é#;\u{2028}filléﬂ°f9f¶ß ",
        ),
        ("a\n\u{2028}\u{2028}", "a\n\u{2028}\u{2028}"),
        ("#é\r", "#é\r"),
        (
            "a\u{2028}classDeffill\u{2028}\u{2028}#f9f;;:#styleé\u{2028}9",
            "a\u{2028}classDeffill\u{2028}\u{2028}ﬂ°f9f¶ß;:#styleé\u{2028}9",
        ),
        ("", ""),
        ("\u{2028}", "\u{2028}"),
        ("\t\n#::::#", "\t\n#::::#"),
        (
            "#é\t\t\r#f9f;classDefstyleclassDefstyle\u{2028}",
            "#é\t\t\rﬂ°f9f¶ßclassDefstyleclassDefstyle\u{2028}",
        ),
        (
            "#:#:#classDef:#;;classDef:#\n",
            "#:#:#classDef:#;classDef:#\n",
        ),
        (
            "style\r \rstyle\t\ré \n\nstyle",
            "style\r \rstyle\t\ré \n\nstyle",
        ),
        (
            "#\r\u{2028}\n:#\u{2028}a\t:9",
            "#\r\u{2028}\n:#\u{2028}a\t:9",
        ),
        ("styleé", "styleé"),
        ("fillé:##a:#\t\r#f9f;;", "fillé:##a:#\t\rﬂ°f9f¶ß;"),
        ("style;fill", "style;fill"),
        ("fillé\n\n", "fillé\n\n"),
        (
            ":#styleé9fill#f9f;\tfillfilléfill\r ",
            ":#styleé9fillﬂ°f9f¶ß\tfillfilléfill\r ",
        ),
        ("9", "9"),
        ("9é\t\u{2028} #f9f;9", "9é\t\u{2028} ﬂ°f9f¶ß9"),
        ("\u{2028} :\t#f9f;::#:#", "\u{2028} :\tﬂ°f9f¶ß::#:#"),
        ("99#f9f;é:\r", "99ﬂ°f9f¶ßé:\r"),
        (
            "éclassDef#  classDef:#;\u{2028} ;#",
            "éclassDef#  classDef:#\u{2028} ;#",
        ),
        (" :#\u{2028} ", " :#\u{2028} "),
        ("\r#\tclassDefclassDef\ré", "\r#\tclassDefclassDef\ré"),
        ("", ""),
        (
            "fill\u{2028}\ré\n#f9f; \u{2028}é\u{2028}\r",
            "fill\u{2028}\ré\nﬂ°f9f¶ß \u{2028}é\u{2028}\r",
        ),
        ("#9\t", "#9\t"),
        ("é\r:#", "é\r:#"),
        (
            "\t;#f9f;9\r\u{2028}\t\n#f9f;fillfilla#f9f;",
            "\t;ﬂ°f9f¶ß9\r\u{2028}\t\nﬂ°f9f¶ßfillfillaﬂ°f9f¶ß",
        ),
        ("", ""),
        ("\r:\n\na9fillé:é\t\u{2028}", "\r:\n\na9fillé:é\t\u{2028}"),
        (
            "stylea#f9f;\r\u{2028}a;9;\t\r",
            "styleaﬂ°f9f¶ß\r\u{2028}a;9;\t\r",
        ),
        ("#f9f;:", "ﬂ°f9f¶ß:"),
        ("\u{2028}\r \n\n\t", "\u{2028}\r \n\n\t"),
        ("\t\r;fill\nclassDefstyle", "\t\r;fill\nclassDefstyle"),
        (
            "\nstyle\u{2028}#; é\u{2028}classDef",
            "\nstyle\u{2028}#; é\u{2028}classDef",
        ),
        ("", ""),
        ("\u{2028}classDef\u{2028}", "\u{2028}classDef\u{2028}"),
        ("9", "9"),
        ("::#; \u{2028}", "::#; \u{2028}"),
        (
            "é#stylestyle#fill:#\rstyle\t",
            "é#stylestyle#fill:#\rstyle\t",
        ),
        ("9a\u{2028}a", "9a\u{2028}a"),
        (":#\u{2028}:\t\t\n:#:#", ":#\u{2028}:\t\t\n:#:#"),
        (" 9:", " 9:"),
        (
            ":#style9\t\u{2028}classDef:\t  ",
            ":#style9\t\u{2028}classDef:\t  ",
        ),
        ("\t;9#f9f;:fill\u{2028}", "\t;9ﬂ°f9f¶ß:fill\u{2028}"),
        ("", ""),
        ("# 9", "# 9"),
        (
            "classDef9fill:;#;classDeffill9",
            "classDef9fill:;#classDeffill9",
        ),
        (
            "#f9f;\nclassDeféstyle#classDef#:\n",
            "ﬂ°f9f¶ß\nclassDeféstyle#classDef#:\n",
        ),
        ("#f9f;fill\r;\t", "ﬂ°f9f¶ßfill\r;\t"),
        (
            "#\u{2028}:classDef#; é9\téstyle#f9f;",
            "#\u{2028}:classDef#; é9\téstyleﬂ°f9f¶ß",
        ),
        (
            "classDef:#é#\u{2028} :\u{2028}\r\r",
            "classDef:#é#\u{2028} :\u{2028}\r\r",
        ),
        ("\r", "\r"),
        ("#a:# 9\u{2028}#", "#a:# 9\u{2028}#"),
        (
            " classDeffill:##\r\u{2028}\t",
            " classDeffill:##\r\u{2028}\t",
        ),
        (" é\u{2028}\u{2028}:#9a9aa", " é\u{2028}\u{2028}:#9a9aa"),
        ("#f9f;", "ﬂ°f9f¶ß"),
        ("9fill\n#9;\u{2028}é\r#:9", "9fill\nﬂ°°9¶ß\u{2028}é\r#:9"),
        (":\r\r#f9f;\n:##", ":\r\rﬂ°f9f¶ß\n:##"),
        ("", ""),
        (
            "style\n\n;fillstyle\n#f9f;fill\n:#",
            "style\n\n;fillstyle\nﬂ°f9f¶ßfill\n:#",
        ),
        ("9#f9f;9éé:#:fillclassDef:9", "9ﬂ°f9f¶ß9éé:#:fillclassDef:9"),
        (
            "\t\t éé classDef\r\u{2028}\tstyle#",
            "\t\t éé classDef\r\u{2028}\tstyle#",
        ),
        ("", ""),
        ("style;\n", "style;\n"),
        (
            "classDef##f9f;:#\t\n\u{2028}",
            "classDef#ﬂ°f9f¶ß:#\t\n\u{2028}",
        ),
        ("#f9f;classDef:#fill\t;", "ﬂ°f9f¶ßclassDef:#fill\t"),
        ("a", "a"),
        ("9style9\n;\t", "9style9\n;\t"),
        (":#classDef\ré\nstylefilla", ":#classDef\ré\nstylefilla"),
        (
            "é;#f9f;style#f9f;classDefé 9a:a",
            "é;ﬂ°f9f¶ßstyleﬂ°f9f¶ßclassDefé 9a:a",
        ),
    ];

    const CORPUS_FILES: [&str; 33] = [
        include_str!("../../../test/vectors/corpus/agentflow.json"),
        include_str!("../../../test/vectors/corpus/architecture.json"),
        include_str!("../../../test/vectors/corpus/block.json"),
        include_str!("../../../test/vectors/corpus/c4.json"),
        include_str!("../../../test/vectors/corpus/class.json"),
        include_str!("../../../test/vectors/corpus/cynefin.json"),
        include_str!("../../../test/vectors/corpus/er.json"),
        include_str!("../../../test/vectors/corpus/eventmodeling.json"),
        include_str!("../../../test/vectors/corpus/flowchart.json"),
        include_str!("../../../test/vectors/corpus/gantt.json"),
        include_str!("../../../test/vectors/corpus/git.json"),
        include_str!("../../../test/vectors/corpus/info.json"),
        include_str!("../../../test/vectors/corpus/ishikawa.json"),
        include_str!("../../../test/vectors/corpus/journey.json"),
        include_str!("../../../test/vectors/corpus/kanban.json"),
        include_str!("../../../test/vectors/corpus/mindmap.json"),
        include_str!("../../../test/vectors/corpus/packet.json"),
        include_str!("../../../test/vectors/corpus/pie.json"),
        include_str!("../../../test/vectors/corpus/quadrant.json"),
        include_str!("../../../test/vectors/corpus/radar.json"),
        include_str!("../../../test/vectors/corpus/railroad.json"),
        include_str!("../../../test/vectors/corpus/requirement.json"),
        include_str!("../../../test/vectors/corpus/sankey.json"),
        include_str!("../../../test/vectors/corpus/sequence.json"),
        include_str!("../../../test/vectors/corpus/state.json"),
        include_str!("../../../test/vectors/corpus/swimlane.json"),
        include_str!("../../../test/vectors/corpus/timeline.json"),
        include_str!("../../../test/vectors/corpus/treemap.json"),
        include_str!("../../../test/vectors/corpus/treeview.json"),
        include_str!("../../../test/vectors/corpus/usecase.json"),
        include_str!("../../../test/vectors/corpus/venn.json"),
        include_str!("../../../test/vectors/corpus/wardley.json"),
        include_str!("../../../test/vectors/corpus/xychart.json"),
    ];

    const CORPUS_EXPECTED: &str = "agentflow\t36fb7c1d\t36fb7c1d\t0\t\t{}\nagentflow\tb8d70f82\tb8d70f82\t0\t\t{}\nagentflow\tb8d70f82\tb8d70f82\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nagentflow\t4ad0f60c\t4ad0f60c\t0\t\t{}\nagentflow\t1b7f9478\t1b7f9478\t0\t\t{}\nagentflow\tb836c81b\tb836c81b\t0\t\t{}\nagentflow\t0d44dc10\t0d44dc10\t0\t\t{}\nagentflow\t33b1a180\t33b1a180\t0\t\t{}\nagentflow\t15030233\t15030233\t0\t\t{}\nagentflow\t3c37ceda\t3c37ceda\t0\t\t{}\nagentflow\t3841c537\t3841c537\t6\t\t{\"agentflow\":{\"nodeSpacing\":70,\"rankSpacing\":70}}\nagentflow\t4bd7eaaf\t4bd7eaaf\t4\t\t{\"layout\":\"elk\"}\narchitecture\t010c3eb1\t010c3eb1\t0\t\t{}\narchitecture\tceea4841\tceea4841\t0\t\t{}\narchitecture\tf83b4985\tf83b4985\t0\t\t{}\narchitecture\t731d0f1a\t731d0f1a\t0\t\t{}\narchitecture\t79dc2a28\t79dc2a28\t0\t\t{}\narchitecture\t3e4e3a8c\t3e4e3a8c\t0\t\t{}\nblock\t900d6434\t900d6434\t0\t\t{}\nblock\t0d771d06\t0d771d06\t0\t\t{}\nblock\tcfe4cec6\tcfe4cec6\t0\t\t{}\nblock\td5af6cc5\td5af6cc5\t0\t\t{}\nblock\t6a948499\t6a948499\t0\t\t{}\nblock\t2c3e1703\t9c1c8a89\t0\t\t{}\nblock\t831b4549\t831b4549\t0\t\t{}\nblock\t5e9f46ea\t5e9f46ea\t0\t\t{}\nblock\tf22a5e26\tf22a5e26\t0\t\t{}\nblock\t171bac85\t171bac85\t0\t\t{}\nblock\t69103640\t69103640\t0\t\t{}\nblock\t0eee4722\t0eee4722\t0\t\t{}\nblock\tb35a3654\tb35a3654\t0\t\t{}\nblock\t6a9022e1\t6a9022e1\t0\t\t{}\nblock\ta9824945\ta9824945\t0\t\t{}\nblock\tf154b468\tf154b468\t0\t\t{}\nblock\t75998a3f\t75998a3f\t0\t\t{}\nblock\tcb4077c8\tcb4077c8\t0\t\t{}\nblock\t9fc43ba7\t9fc43ba7\t0\t\t{}\nblock\t28025fbc\t28025fbc\t0\t\t{}\nblock\t8628291a\t8628291a\t0\t\t{}\nblock\t99bb9fd0\t99bb9fd0\t0\t\t{}\nblock\t20b9d781\t20b9d781\t0\t\t{}\nblock\tf9110247\tf9110247\t0\t\t{}\nblock\t8b83e68c\t8b83e68c\t0\t\t{}\nblock\t8c911ce0\t8c911ce0\t0\t\t{}\nblock\tc25a2a64\tc25a2a64\t0\t\t{}\nblock\ted53008a\ted53008a\t0\t\t{}\nblock\te8d69764\t5ff15f6c\t0\t\t{}\nblock\t4595dd96\t4595dd96\t0\t\t{}\nc4\t9d1e0cbc\taf81e1ac\t0\t\t{}\nc4\tcb23a5de\te465532e\t0\t\t{}\nc4\te2be2b28\t0f070818\t0\t\t{}\nc4\t0d1fbbd8\t0a9df668\t0\t\t{}\nc4\tf80cad6a\td314fefa\t0\t\t{}\nc4\t802e5294\tb7d44f44\t0\t\t{}\nclass\t8e5c892b\t8e5c892b\t3\tAnimal example\t{}\nclass\tadd3a3be\tadd3a3be\t0\t\t{}\nclass\tadd3a3be\tadd3a3be\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nclass\t7782e1d5\t7782e1d5\t3\tBank example\t{}\nclass\t89ad72ba\t89ad72ba\t0\t\t{}\nclass\te0b6c861\te0b6c861\t0\t\t{}\nclass\t7d92a1e4\t7d92a1e4\t0\t\t{}\nclass\t314e3bed\t314e3bed\t0\t\t{}\nclass\tc3d64d9f\tc3d64d9f\t0\t\t{}\nclass\t34c3728e\t34c3728e\t0\t\t{}\nclass\tdc20d7e6\tdc20d7e6\t0\t\t{}\nclass\t572672cb\t572672cb\t0\t\t{}\nclass\t531aa71f\t531aa71f\t0\t\t{}\nclass\ta7a69a50\ta7a69a50\t0\t\t{}\nclass\tf7ed1d40\tf7ed1d40\t0\t\t{}\nclass\t923b71e2\t923b71e2\t0\t\t{}\nclass\te40ba704\te40ba704\t0\t\t{}\nclass\tf22998ba\tf22998ba\t0\t\t{}\nclass\t26167917\t26167917\t0\t\t{}\nclass\tc36a7c97\tc36a7c97\t0\t\t{}\nclass\t32c5753c\t32c5753c\t0\t\t{}\nclass\td66b78a3\td66b78a3\t5\t\t{\"class\":{\"hierarchicalNamespaces\":false}}\nclass\t3485bde9\t3485bde9\t0\t\t{}\nclass\td3a4340e\td3a4340e\t0\t\t{}\nclass\t5a9b377b\t5a9b377b\t0\t\t{}\nclass\t4347de28\t4347de28\t0\t\t{}\nclass\t5a5dce04\t3f1a4986\t0\t\t{}\nclass\t4517a059\t4517a059\t0\t\t{}\nclass\t0485377d\t0485377d\t0\t\t{}\nclass\t5dd7bc79\t5dd7bc79\t0\t\t{}\nclass\t3e011360\t3e011360\t0\t\t{}\nclass\t8ff6b8fb\t8ff6b8fb\t0\t\t{}\nclass\t98af529a\t98af529a\t0\t\t{}\nclass\t4681bcbc\t4681bcbc\t0\t\t{}\nclass\t2150f61a\t2150f61a\t0\t\t{}\nclass\te11a7901\te11a7901\t0\t\t{}\nclass\t97c3d6bb\t97c3d6bb\t0\t\t{}\nclass\te818366f\te818366f\t5\t\t{\"class\":{\"hideEmptyMembersBox\":true}}\ncynefin\t2417ab33\t2417ab33\t0\t\t{}\ncynefin\t0d612224\t0d612224\t0\t\t{}\ncynefin\t941ea8ef\t941ea8ef\t0\t\t{}\ner\tb519c6f6\tb519c6f6\t3\tOrder example\t{}\ner\t57faef1b\t57faef1b\t0\t\t{}\ner\t31853fa1\t31853fa1\t0\t\t{}\ner\t31853fa1\t31853fa1\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\ner\t555cdc30\t555cdc30\t0\t\t{}\ner\t12a46bbe\t12a46bbe\t0\t\t{}\ner\t35ad36de\t35ad36de\t0\t\t{}\ner\t09ee1bbb\t09ee1bbb\t0\t\t{}\ner\td81a6a1d\td81a6a1d\t0\t\t{}\ner\t0a6d68ee\t0a6d68ee\t0\t\t{}\ner\te7215b37\te7215b37\t0\t\t{}\ner\t80c2cb54\t80c2cb54\t0\t\t{}\ner\t2a8ea064\t2a8ea064\t0\t\t{}\ner\t19f47e9c\t19f47e9c\t0\t\t{}\ner\t00abc6a6\t00abc6a6\t0\t\t{}\ner\tcb14b32f\tcb14b32f\t0\t\t{}\ner\t803526c0\t803526c0\t0\t\t{}\ner\tba3dcc09\tba3dcc09\t0\t\t{}\ner\tdae580aa\tdae580aa\t0\t\t{}\ner\tf9625fe1\tf9625fe1\t0\t\t{}\ner\t36aac7bb\t36aac7bb\t0\t\t{}\ner\t3f0a9277\t3f0a9277\t0\t\t{}\ner\t75881462\t75881462\t0\t\t{}\ner\t9cb718e8\t9cb718e8\t0\t\t{}\ner\tb519c6f6\tb519c6f6\t5\tOrder example\t{\"layout\":\"dagre\"}\neventmodeling\t1f33c9b0\t1f33c9b0\t0\t\t{}\neventmodeling\t8a59c50c\t8a59c50c\t0\t\t{}\neventmodeling\t95575a80\t95575a80\t0\t\t{}\neventmodeling\tbc71546d\tbc71546d\t0\t\t{}\neventmodeling\t9848114b\t9848114b\t0\t\t{}\neventmodeling\t9a983b66\t9a983b66\t0\t\t{}\neventmodeling\tf07a056c\tf07a056c\t0\t\t{}\neventmodeling\t91afb16a\t91afb16a\t0\t\t{}\neventmodeling\t919bda1b\t919bda1b\t0\t\t{}\neventmodeling\ta2e630c9\ta2e630c9\t0\t\t{}\neventmodeling\tad6523b2\tad6523b2\t0\t\t{}\neventmodeling\tf5f937a0\tf5f937a0\t0\t\t{}\nflowchart\t57acfd08\t57acfd08\t3\tNode\t{}\nflowchart\t0c8e7122\t0c8e7122\t3\tNode with text\t{}\nflowchart\t7948ab91\t7948ab91\t0\t\t{}\nflowchart\t23bcfc42\t23bcfc42\t4\t\t{\"htmlLabels\":false}\nflowchart\t7f992a1d\t7f992a1d\t0\t\t{}\nflowchart\t9e79ad6f\t9e79ad6f\t0\t\t{}\nflowchart\t86ca7817\t86ca7817\t0\t\t{}\nflowchart\t86ca7817\t86ca7817\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nflowchart\te77f6e15\te77f6e15\t0\t\t{}\nflowchart\tbe90dee1\tbe90dee1\t0\t\t{}\nflowchart\t226c008e\t226c008e\t0\t\t{}\nflowchart\ta2ce819d\ta2ce819d\t0\t\t{}\nflowchart\t49e21585\t49e21585\t0\t\t{}\nflowchart\t4348ebe7\t4348ebe7\t0\t\t{}\nflowchart\t86f8da62\t86f8da62\t0\t\t{}\nflowchart\td30e3bce\td30e3bce\t0\t\t{}\nflowchart\tbdba9042\tbdba9042\t0\t\t{}\nflowchart\t8abf0bfe\t8abf0bfe\t0\t\t{}\nflowchart\t3996f2ad\t3996f2ad\t0\t\t{}\nflowchart\tc40473fe\tc40473fe\t0\t\t{}\nflowchart\t02f26346\t02f26346\t0\t\t{}\nflowchart\t6fa348c2\t6fa348c2\t0\t\t{}\nflowchart\t8ddb6ed9\t8ddb6ed9\t0\t\t{}\nflowchart\t4ff8766f\t4ff8766f\t0\t\t{}\nflowchart\t5648c8c4\t5648c8c4\t0\t\t{}\nflowchart\tdc05698d\tdc05698d\t0\t\t{}\nflowchart\t21b2be88\t21b2be88\t0\t\t{}\nflowchart\t429ad379\t429ad379\t0\t\t{}\nflowchart\t6824738e\t6824738e\t0\t\t{}\nflowchart\t52dcaadb\t52dcaadb\t0\t\t{}\nflowchart\t090b3b6d\t090b3b6d\t0\t\t{}\nflowchart\tc0a72f02\tc0a72f02\t0\t\t{}\nflowchart\t4d509ece\t4d509ece\t0\t\t{}\nflowchart\t32f7d28f\t32f7d28f\t0\t\t{}\nflowchart\t016c50e7\t016c50e7\t0\t\t{}\nflowchart\tde63b7ec\tde63b7ec\t0\t\t{}\nflowchart\te989939f\te989939f\t0\t\t{}\nflowchart\t49da8c09\t49da8c09\t0\t\t{}\nflowchart\t25f07438\t25f07438\t0\t\t{}\nflowchart\t21e48ad6\t21e48ad6\t0\t\t{}\nflowchart\t21bd315a\t21bd315a\t0\t\t{}\nflowchart\t4344fecb\t4344fecb\t0\t\t{}\nflowchart\ta8ce528e\ta8ce528e\t0\t\t{}\nflowchart\t4c50e5a5\t4c50e5a5\t0\t\t{}\nflowchart\t7c599b7b\t7c599b7b\t0\t\t{}\nflowchart\t8a9223e0\t8a9223e0\t0\t\t{}\nflowchart\t03397494\t03397494\t0\t\t{}\nflowchart\t1a289f06\t1a289f06\t0\t\t{}\nflowchart\t538a20e2\t538a20e2\t0\t\t{}\nflowchart\t498eaafb\t498eaafb\t0\t\t{}\nflowchart\tae867691\tae867691\t0\t\t{}\nflowchart\td41d4b9f\td41d4b9f\t0\t\t{}\nflowchart\t8b82a443\t8b82a443\t0\t\t{}\nflowchart\t3e90685b\t3e90685b\t0\t\t{}\nflowchart\t329ba8a7\t329ba8a7\t0\t\t{}\nflowchart\t16584c0e\t16584c0e\t0\t\t{}\nflowchart\t0a929427\t0a929427\t0\t\t{}\nflowchart\tbbce6bee\tbbce6bee\t0\t\t{}\nflowchart\t2831f03c\t2831f03c\t0\t\t{}\nflowchart\t8320b844\t8320b844\t0\t\t{}\nflowchart\teb534e37\teb534e37\t0\t\t{}\nflowchart\t96524410\t96524410\t0\t\t{}\nflowchart\t99b93549\t99b93549\t0\t\t{}\nflowchart\t4d5dd225\t4d5dd225\t0\t\t{}\nflowchart\tbdb6142b\tbdb6142b\t0\t\t{}\nflowchart\t20a82ff1\t20a82ff1\t0\t\t{}\nflowchart\t556807c1\t556807c1\t0\t\t{}\nflowchart\tc6f5afc9\tc6f5afc9\t0\t\t{}\nflowchart\t4b275bb0\t4b275bb0\t0\t\t{}\nflowchart\td4c3acec\t64ffcfc7\t0\t\t{}\nflowchart\te929d0fe\te929d0fe\t0\t\t{}\nflowchart\t2a1d3091\t2a1d3091\t0\t\t{}\nflowchart\tfb780100\tfb780100\t0\t\t{}\nflowchart\t6861a8a9\t6861a8a9\t0\t\t{}\nflowchart\t4b96665f\t4b96665f\t0\t\t{}\nflowchart\t48ca6f33\t48ca6f33\t0\t\t{}\nflowchart\t5ab1a1f7\t5ab1a1f7\t0\t\t{}\nflowchart\t634e765d\t634e765d\t0\t\t{}\nflowchart\tebd87860\tebd87860\t0\t\t{}\nflowchart\tcc13781f\tcc13781f\t0\t\t{}\nflowchart\tdea02718\tdea02718\t0\t\t{}\nflowchart\t9c2fd3dd\t9c2fd3dd\t0\t\t{}\nflowchart\t149fd903\t149fd903\t0\t\t{}\nflowchart\tdad7db11\tdad7db11\t0\t\t{}\nflowchart\t8aa370f3\t8aa370f3\t0\t\t{}\nflowchart\t7ddb35ec\t7ddb35ec\t0\t\t{}\nflowchart\t204cc649\t204cc649\t0\t\t{}\nflowchart\t8430227c\t8430227c\t0\t\t{}\nflowchart\t286f91a0\t286f91a0\t0\t\t{}\nflowchart\t5871da13\t5871da13\t0\t\t{}\nflowchart\t08ce3228\t08ce3228\t0\t\t{}\nflowchart\te2287bce\te2287bce\t0\t\t{}\nflowchart\t2e234fa7\t2e234fa7\t0\t\t{}\nflowchart\tcf44c7c3\tcf44c7c3\t0\t\t{}\nflowchart\tf8b0a9f5\tf8b0a9f5\t0\t\t{}\nflowchart\t876ebad0\t837c6fe0\t0\t\t{}\nflowchart\t58fbaa10\t58fbaa10\t0\t\t{}\nflowchart\t1935dc5f\t1935dc5f\t0\t\t{}\nflowchart\t0dd8381f\t0dd8381f\t0\t\t{}\nflowchart\tbb3154dc\tbb3154dc\t0\t\t{}\nflowchart\tbf9c9d73\tcf512fbd\t0\t\t{}\nflowchart\t4478325c\t4478325c\t0\t\t{}\nflowchart\tb1ff7017\tb1ff7017\t4\t\t{\"htmlLabels\":false}\nflowchart\tf9e193e4\tf9e193e4\t0\t\t{}\nflowchart\t7309203b\t7309203b\t0\t\t{}\nflowchart\t9c2fd3dd\tb10ccaa5\t0\t\t{}\nflowchart\t30419d8c\t30419d8c\t0\t\t{}\nflowchart\td2d745d5\td2d745d5\t0\t\t{}\nflowchart\tb2a5f032\tb2a5f032\t0\t\t{}\nflowchart\td9a1ef01\td9a1ef01\t0\t\t{}\nflowchart\t8e997953\t8e997953\t0\t\t{}\nflowchart\t26861c45\t26861c45\t0\t\t{}\nflowchart\ta48af3c0\ta48af3c0\t0\t\t{}\nflowchart\t4db96e89\t4db96e89\t0\t\t{}\ngantt\t47fa1b6b\t47fa1b6b\t0\t\t{}\ngantt\t1384b122\t66295b79\t0\t\t{}\ngantt\tc7df4d73\tc7df4d73\t0\t\t{}\ngantt\tacd6084f\tacd6084f\t0\t\t{}\ngantt\tb1c7aa65\tb1c7aa65\t0\t\t{}\ngantt\t6a5566d6\t6a5566d6\t0\t\t{}\ngantt\t4bd784f0\t4bd784f0\t0\t\t{}\ngantt\td5da20db\td5da20db\t3\t\t{\"gantt\":{\"displayMode\":\"compact\"}}\ngantt\t47fa1b6b\t4a0dfcbf\t0\t\t{}\ngantt\ta0552be7\ta0552be7\t0\t\t{}\ngantt\t7aef81dc\t4e7c2815\t20\tIgnored if specified in chart\t{\"themeCSS\":\" // YAML supports multiline strings using a newline markers: \\n #item36 { fill: CadetBlue }       \\n\\n// Custom marker workaround CSS from forum (below)    \\n rect[id^=workaround] { height: calc(100% - 50px) ; transform: translate(9px, 25px); y: 0; width: 1.5px; stroke: none; fill: red; }   \\n text[id^=workaround] { fill: red; y: 100%; font-size: 15px;} \",\"gantt\":{\"useWidth\":400,\"rightPadding\":0,\"topAxis\":true,\"numberSectionStyles\":2,\"displayMode\":\"compact\"}}\ngitGraph\t7f0dc3ef\t7f0dc3ef\t3\tExample Git diagram\t{}\ngitGraph\t1f600cac\t1256d71c\t0\t\t{}\ngitGraph\t7d10be62\tb4aa8192\t0\t\t{}\ngitGraph\tacb03a1b\t148ad48b\t0\t\t{}\ngitGraph\tb076f562\t3d2fe952\t0\t\t{}\ngitGraph\tf9bf6031\tfa1256c1\t0\t\t{}\ngitGraph\tab2134e2\t8170bc52\t0\t\t{}\ngitGraph\tfc5c081b\t4602d90b\t0\t\t{}\ngitGraph\tc950f9b9\t477f5589\t0\t\t{}\ngitGraph\t3c12f0f6\t387f9e66\t0\t\t{}\ngitGraph\t21acb06a\t1758cf42\t7\t\t{\"logLevel\":\"debug\",\"theme\":\"base\",\"gitGraph\":{\"showBranches\":false}}\ngitGraph\t516c0815\t516c0815\t7\t\t{\"logLevel\":\"debug\",\"theme\":\"base\",\"gitGraph\":{\"rotateCommitLabel\":true}}\ngitGraph\t516c0815\t516c0815\t7\t\t{\"logLevel\":\"debug\",\"theme\":\"base\",\"gitGraph\":{\"rotateCommitLabel\":false}}\ngitGraph\td6aed840\t6c6f8b28\t8\t\t{\"logLevel\":\"debug\",\"theme\":\"base\",\"gitGraph\":{\"showBranches\":false,\"showCommitLabel\":false}}\ngitGraph\t7e7e3740\tb39831c8\t9\t\t{\"logLevel\":\"debug\",\"theme\":\"base\",\"gitGraph\":{\"showBranches\":true,\"showCommitLabel\":true,\"mainBranchName\":\"MetroLine1\"}}\ngitGraph\t96070688\tbf901e50\t8\t\t{\"logLevel\":\"debug\",\"theme\":\"base\",\"gitGraph\":{\"showBranches\":true,\"showCommitLabel\":true}}\ngitGraph\t1a534edc\tc1a9ded4\t9\t\t{\"logLevel\":\"debug\",\"theme\":\"base\",\"gitGraph\":{\"showBranches\":true,\"showCommitLabel\":true,\"mainBranchOrder\":2}}\ngitGraph\tabe6e022\t93cc2372\t0\t\t{}\ngitGraph\tb50a144a\t1b2a631a\t0\t\t{}\ngitGraph\t7f91b4ba\t7d4f7f0a\t0\t\t{}\ngitGraph\t568e5276\t568e5276\t5\t\t{\"gitGraph\":{\"parallelCommits\":false}}\ngitGraph\t568e5276\t568e5276\t5\t\t{\"gitGraph\":{\"parallelCommits\":true}}\ngitGraph\t21acb06a\t1758cf42\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"base\"}\ngitGraph\t21acb06a\t1758cf42\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"forest\"}\ngitGraph\t2b61d294\t30b8cf7c\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"default\"}\ngitGraph\t21acb06a\t1758cf42\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"dark\"}\ngitGraph\t21acb06a\t1758cf42\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"neutral\"}\ngitGraph\t4fab2921\t0c94cfbf\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"default\"}\ngitGraph\t9571ceb1\t8342f9ef\t14\t\t{\"logLevel\":\"debug\",\"theme\":\"default\",\"themeVariables\":{\"git0\":\"#ff0000\",\"git1\":\"#00ff00\",\"git2\":\"#0000ff\",\"git3\":\"#ff00ff\",\"git4\":\"#00ffff\",\"git5\":\"#ffff00\",\"git6\":\"#ff00ff\",\"git7\":\"#00ffff\"}}\ngitGraph\ta998b8e3\tdcb0d84b\t16\t\t{\"logLevel\":\"debug\",\"theme\":\"default\",\"themeVariables\":{\"gitBranchLabel0\":\"#ffffff\",\"gitBranchLabel1\":\"#ffffff\",\"gitBranchLabel2\":\"#ffffff\",\"gitBranchLabel3\":\"#ffffff\",\"gitBranchLabel4\":\"#ffffff\",\"gitBranchLabel5\":\"#ffffff\",\"gitBranchLabel6\":\"#ffffff\",\"gitBranchLabel7\":\"#ffffff\",\"gitBranchLabel8\":\"#ffffff\",\"gitBranchLabel9\":\"#ffffff\"}}\ngitGraph\t9571ceb1\t8342f9ef\t8\t\t{\"logLevel\":\"debug\",\"theme\":\"default\",\"themeVariables\":{\"commitLabelColor\":\"#ff0000\",\"commitLabelBackground\":\"#00ff00\"}}\ngitGraph\t9571ceb1\t8342f9ef\t9\t\t{\"logLevel\":\"debug\",\"theme\":\"default\",\"themeVariables\":{\"commitLabelColor\":\"#ff0000\",\"commitLabelBackground\":\"#00ff00\",\"commitLabelFontSize\":\"16px\"}}\ngitGraph\t9571ceb1\t8342f9ef\t9\t\t{\"logLevel\":\"debug\",\"theme\":\"default\",\"themeVariables\":{\"commitLabelColor\":\"#ff0000\",\"commitLabelBackground\":\"#00ff00\",\"tagLabelFontSize\":\"16px\"}}\ngitGraph\t9571ceb1\t8342f9ef\t9\t\t{\"logLevel\":\"debug\",\"theme\":\"default\",\"themeVariables\":{\"tagLabelColor\":\"#ff0000\",\"tagLabelBackground\":\"#00ff00\",\"tagLabelBorder\":\"#0000ff\"}}\ngitGraph\t9571ceb1\t8342f9ef\t7\t\t{\"logLevel\":\"debug\",\"theme\":\"default\",\"themeVariables\":{\"gitInv0\":\"#ff0000\"}}\ninfo\tc7671c9d\tc7671c9d\t0\t\t{}\ninfo\t1df1e5e0\t1df1e5e0\t0\t\t{}\nishikawa\t7f3ea0df\t7f3ea0df\t0\t\t{}\njourney\tb3182409\tb3182409\t0\t\t{}\nkanban\t48e74608\t48e74608\t0\t\t{}\nkanban\td96df6e1\td96df6e1\t0\t\t{}\nkanban\te66e4a3b\te66e4a3b\t5\t\t{\"kanban\":{\"ticketBaseUrl\":\"https://mermaidchart.atlassian.net/browse/#TICKET#\"}}\nmindmap\tfc358622\tfc358622\t0\t\t{}\nmindmap\t4fa54287\t4fa54287\t0\t\t{}\nmindmap\tba7ccf97\tba7ccf97\t0\t\t{}\nmindmap\t2e660aab\t2e660aab\t0\t\t{}\nmindmap\t192ba6f4\t192ba6f4\t0\t\t{}\nmindmap\t2012c672\t2012c672\t0\t\t{}\nmindmap\te5ea3468\te5ea3468\t0\t\t{}\nmindmap\t2a915d8e\t2a915d8e\t0\t\t{}\nmindmap\t1067a411\t1067a411\t0\t\t{}\nmindmap\t267e8a34\t267e8a34\t0\t\t{}\nmindmap\tdee4894d\tdee4894d\t0\t\t{}\nmindmap\t49998717\t49998717\t0\t\t{}\nmindmap\t7b454237\t7b454237\t0\t\t{}\npacket\t9cd72ace\t9cd72ace\t3\tTCP Packet\t{}\npacket\tb1f395a1\tb1f395a1\t0\t\t{}\npacket\t480e3a55\t480e3a55\t7\t\t{\"packet\":{\"showBits\":true,\"bitOrder\":\"descending\",\"bitsPerRow\":16}}\npacket\td9cdd28e\td9cdd28e\t8\t\t{\"packet\":{\"showBits\":true},\"themeVariables\":{\"packet\":{\"startByteColor\":\"red\"}}}\npie\t46193240\t46193240\t0\t\t{}\npie\t90cd0959\t90cd0959\t9\t\t{\"pie\":{\"textPosition\":0.5,\"donutHole\":0.2,\"highlightSlice\":\"Potassium\"},\"themeVariables\":{\"pieOuterStrokeWidth\":\"5px\"}}\nquadrantChart\te4d3e1ad\te4d3e1ad\t0\t\t{}\nquadrantChart\tb076946c\tb076946c\t8\t\t{\"quadrantChart\":{\"chartWidth\":400,\"chartHeight\":400},\"themeVariables\":{\"quadrant1TextFill\":\"ff0000\"}}\nquadrantChart\t15d42772\t15d42772\t0\t\t{}\nradar\t4022b389\t4022b389\t3\tGrades\t{}\nradar\t27b10ab4\t27b10ab4\t0\t\t{}\nradar\tc3fafcc9\tc3fafcc9\t13\t\t{\"radar\":{\"axisScaleFactor\":0.25,\"curveTension\":0.1},\"theme\":\"base\",\"themeVariables\":{\"cScale0\":\"#FF0000\",\"cScale1\":\"#00FF00\",\"cScale2\":\"#0000FF\",\"radar\":{\"curveOpacity\":0}}}\nrailroad\t6d3e2280\t6d3e2280\t0\t\t{}\nrailroad\t603c30b9\t603c30b9\t0\t\t{}\nrailroad\tcc0fa161\tcc0fa161\t0\t\t{}\nrailroad\tc668763d\tc668763d\t0\t\t{}\nrailroad\tb689ec79\tb689ec79\t0\t\t{}\nrailroad\t22ce1b19\t22ce1b19\t0\t\t{}\nrailroad\tbd64ee07\tbd64ee07\t0\t\t{}\nrailroad\tf0107b3c\tf0107b3c\t0\t\t{}\nrailroad\t50d1abe2\t50d1abe2\t0\t\t{}\nrailroad\t58a4cd5e\t58a4cd5e\t0\t\t{}\nrailroad\t23c1157e\t23c1157e\t0\t\t{}\nrailroad\t35185047\t35185047\t0\t\t{}\nrailroad\t27a4d56b\t27a4d56b\t0\t\t{}\nrailroad\tca60248a\tca60248a\t0\t\t{}\nrequirement\t5997d4dc\t3382140c\t0\t\t{}\nrequirement\t3ba9760e\t3ba9760e\t0\t\t{}\nrequirement\t3ba9760e\t3ba9760e\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nrequirement\t86369750\t86369750\t0\t\t{}\nrequirement\t2de2bdfc\t3c65660c\t0\t\t{}\nrequirement\te1a6aa2f\te1a6aa2f\t0\t\t{}\nrequirement\t2d1aaa3b\t2d1aaa3b\t0\t\t{}\nrequirement\t5f9ec524\t5f9ec524\t0\t\t{}\nrequirement\td6cdbc5e\td6cdbc5e\t0\t\t{}\nsankey\tee0ebbb7\tee0ebbb7\t5\t\t{\"sankey\":{\"showValues\":false}}\nsankey\tc8ad81d9\tc7bea05c\t0\t\t{}\nsankey\tf34a26c5\tf34a26c5\t0\t\t{}\nsankey\t325f7feb\t325f7feb\t0\t\t{}\nsankey\tb670e80b\tb670e80b\t0\t\t{}\nsankey\t70ea3ddf\t70ea3ddf\t6\t\t{\"sankey\":{\"showValues\":false,\"labelStyle\":\"outlined\"}}\nsankey\t70ea3ddf\t70ea3ddf\t7\t\t{\"sankey\":{\"showValues\":false,\"nodeWidth\":15,\"nodePadding\":20}}\nsankey\t70ea3ddf\t70ea3ddf\t9\t\t{\"sankey\":{\"showValues\":false,\"nodeColors\":{\"Electricity grid\":\"#4e79a7\",\"Industry\":\"#e15759\",\"Losses\":\"#bab0ab\"}}}\nsequence\ta0c2d897\ta0c2d897\t0\t\t{}\nsequence\ta0b80c84\ta0b80c84\t0\t\t{}\nsequence\ta0b80c84\ta0b80c84\t5\t\t{\"theme\":\"default\",\"look\":\"classic\"}\nsequence\td927c0b1\td927c0b1\t0\t\t{}\nsequence\t8a0559d1\t8a0559d1\t0\t\t{}\nsequence\t35abe965\t35abe965\t0\t\t{}\nsequence\tc0d16015\tc0d16015\t0\t\t{}\nsequence\t72622699\t72622699\t0\t\t{}\nsequence\td9460974\td9460974\t0\t\t{}\nsequence\tc1a4901d\tc1a4901d\t0\t\t{}\nsequence\t59107bb9\t59107bb9\t0\t\t{}\nsequence\ta4142a40\ta4142a40\t0\t\t{}\nsequence\te6bc0434\te6bc0434\t0\t\t{}\nsequence\t93ef0dd7\t93ef0dd7\t0\t\t{}\nsequence\t967bd34e\t967bd34e\t0\t\t{}\nsequence\t1e80b6d4\t1e80b6d4\t0\t\t{}\nsequence\taaacb3a1\t3cccce11\t0\t\t{}\nsequence\t1c00c3bd\t1c00c3bd\t0\t\t{}\nsequence\t67796496\t67796496\t0\t\t{}\nsequence\td792213d\td792213d\t0\t\t{}\nsequence\tbe0989ae\tbe0989ae\t0\t\t{}\nsequence\td3dc1c23\td3dc1c23\t0\t\t{}\nsequence\t0fff5ee6\t0fff5ee6\t0\t\t{}\nsequence\t20d2da69\t20d2da69\t0\t\t{}\nsequence\t312b01a2\t312b01a2\t0\t\t{}\nsequence\t93852493\t93852493\t0\t\t{}\nsequence\t7afb93fd\t7afb93fd\t0\t\t{}\nsequence\tacad7bae\tacad7bae\t0\t\t{}\nsequence\tfa793134\tfa793134\t0\t\t{}\nsequence\t2b259338\t2b259338\t0\t\t{}\nsequence\t31949952\t31949952\t0\t\t{}\nsequence\tb9b448b7\tb9b448b7\t0\t\t{}\nsequence\ta64fcf96\ta64fcf96\t0\t\t{}\nsequence\t9c619d19\te5772d7b\t0\t\t{}\nsequence\td895a998\td895a998\t0\t\t{}\nsequence\t3613d291\t3613d291\t0\t\t{}\nsequence\te28b3e3c\te28b3e3c\t0\t\t{}\nsequence\tf32f9e93\tf32f9e93\t0\t\t{}\nstate\tcaff735c\tcaff735c\t3\tSimple sample\t{}\nstate\t4f35fc37\t4f35fc37\t0\t\t{}\nstate\t08d97933\t08d97933\t0\t\t{}\nstate\t08d97933\t08d97933\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nstate\te9b15c3a\te9b15c3a\t0\t\t{}\nstate\td75d2096\td75d2096\t0\t\t{}\nstate\tc1b1cbaf\tc1b1cbaf\t0\t\t{}\nstate\t9ab4525d\t9ab4525d\t0\t\t{}\nstate\t43096eb5\t43096eb5\t0\t\t{}\nstate\t6cac7690\t6cac7690\t0\t\t{}\nstate\t561e8206\t561e8206\t0\t\t{}\nstate\t8b941a2b\t8b941a2b\t0\t\t{}\nstate\t0759428d\t0759428d\t0\t\t{}\nstate\tbf1ec1c4\tbf1ec1c4\t0\t\t{}\nstate\t5ce013d6\t93bd5dec\t0\t\t{}\nstate\tb227146b\t7d7e321b\t0\t\t{}\nstate\t3c5f8537\t3c5f8537\t0\t\t{}\nstate\t4412a298\t4412a298\t0\t\t{}\nstate\tde163a8e\t828ca9fc\t0\t\t{}\nstate\t10cafb4a\t7f439784\t0\t\t{}\nstate\t3df1e32b\t3df1e32b\t0\t\t{}\nstate\t9de05457\t9de05457\t0\t\t{}\nswimlane\t2e99bf24\t2e99bf24\t0\t\t{}\nswimlane\t2e99bf24\t2e99bf24\t5\t\t{\"theme\":\"default\",\"look\":\"classic\"}\nswimlane\tae3bd4fb\tae3bd4fb\t0\t\t{}\nswimlane\t46a6db51\t46a6db51\t0\t\t{}\nswimlane\tf57728a8\tf57728a8\t0\t\t{}\nswimlane\tf545f1de\tf545f1de\t0\t\t{}\nswimlane\t26f2baf8\t26f2baf8\t0\t\t{}\nswimlane\t9abc10b2\t9abc10b2\t0\t\t{}\nswimlane\tfdfef640\tfdfef640\t0\t\t{}\nswimlane\t35e1c477\t35e1c477\t0\t\t{}\nswimlane\tc240666f\tc240666f\t0\t\t{}\nswimlane\t8e9ba545\t8e9ba545\t0\t\t{}\nswimlane\t1fc95993\t1fc95993\t0\t\t{}\ntimeline\tdfd24e09\tdfd24e09\t0\t\t{}\ntimeline\t98182ef1\t98182ef1\t0\t\t{}\ntimeline\tb21681db\tb21681db\t0\t\t{}\ntimeline\tba5adcc2\tba5adcc2\t0\t\t{}\ntimeline\tcf8fa2c1\tcf8fa2c1\t0\t\t{}\ntimeline\t6c4b6989\t6c4b6989\t0\t\t{}\ntimeline\t3b011e61\te0900cd1\t0\t\t{}\ntimeline\t3b011e61\te0900cd1\t7\t\t{\"logLevel\":\"debug\",\"theme\":\"base\",\"timeline\":{\"disableMulticolor\":true}}\ntimeline\t187c586d\t797f833f\t11\t\t{\"logLevel\":\"debug\",\"theme\":\"default\",\"themeVariables\":{\"cScale0\":\"#ff0000\",\"cScaleLabel0\":\"#ffffff\",\"cScale1\":\"#00ff00\",\"cScale2\":\"#0000ff\",\"cScaleLabel2\":\"#ffffff\"}}\ntimeline\t4e327df5\t1c35ff45\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"base\"}\ntimeline\t4e327df5\t1c35ff45\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"forest\"}\ntimeline\t4e327df5\t1c35ff45\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"dark\"}\ntimeline\t4e327df5\t1c35ff45\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"default\"}\ntimeline\t4e327df5\t1c35ff45\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"neutral\"}\ntreemap\tbb56e46b\tbb56e46b\t0\t\t{}\ntreemap\t00c4ad6c\t00c4ad6c\t0\t\t{}\ntreemap\t4b069166\t4b069166\t0\t\t{}\ntreemap\t0a2af022\t0a2af022\t0\t\t{}\ntreemap\tbb56e46b\tbb56e46b\t4\t\t{\"theme\":\"forest\"}\ntreemap\tbb56e46b\tbb56e46b\t5\t\t{\"treemap\":{\"diagramPadding\":200}}\ntreemap\t6c5978b4\t6c5978b4\t5\t\t{\"treemap\":{\"valueFormat\":\"$0,0\"}}\ntreemap\t723777bd\t723777bd\t5\t\t{\"treemap\":{\"valueFormat\":\"$.1%\"}}\ntreeView\tebe4ff6e\tebe4ff6e\t0\t\t{}\ntreeView\tb6f22ab9\tb6f22ab9\t0\t\t{}\ntreeView\t84eccba8\t84eccba8\t0\t\t{}\ntreeView\t7351ee4b\t7351ee4b\t0\t\t{}\ntreeView\t2ef21965\t2ef21965\t0\t\t{}\ntreeView\t365d222d\t365d222d\t5\t\t{\"treeView\":{\"showIcons\":true}}\ntreeView\t8400f678\t8400f678\t12\t\t{\"treeView\":{\"showIcons\":true,\"defaultIconPack\":\"material-icon-theme\",\"filenameIcons\":{\"Dockerfile\":\"docker\"},\"extensionIcons\":{\".ts\":\"typescript\",\".tsx\":\"react-ts\",\".txt\":\"none\"}}}\ntreeView\t0417db00\t0417db00\t0\t\t{}\ntreeView\td237669b\td237669b\t5\t\t{\"treeView\":{\"showIcons\":true}}\ntreeView\tc209a9a8\tc209a9a8\t0\t\t{}\ntreeView\t8aceac1a\t8aceac1a\t0\t\t{}\ntreeView\t6a66342d\t6a66342d\t0\t\t{}\ntreeView\t8aceac1a\t8aceac1a\t11\t\t{\"treeView\":{\"rowIndent\":80,\"lineThickness\":3},\"themeVariables\":{\"treeView\":{\"labelFontSize\":\"20px\",\"labelColor\":\"#FF0000\",\"lineColor\":\"#00FF00\"}}}\nusecase\te23d60b5\te23d60b5\t0\t\t{}\nusecase\t14fdca24\t14fdca24\t0\t\t{}\nusecase\t14fdca24\t14fdca24\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nusecase\t7e9c7dd2\t7e9c7dd2\t0\t\t{}\nusecase\t52b79d2f\t52b79d2f\t0\t\t{}\nusecase\tc1223856\tc1223856\t0\t\t{}\nusecase\t71190a83\t71190a83\t0\t\t{}\nusecase\t1ce2c5bb\t1ce2c5bb\t0\t\t{}\nusecase\td15af1ca\t5b23441b\t0\t\t{}\nusecase\t97f58e46\t97f58e46\t0\t\t{}\nusecase\t1edab795\t1edab795\t0\t\t{}\nusecase\tb9874eb3\tb9874eb3\t0\t\t{}\nusecase\tde4d86b5\tde4d86b5\t0\t\t{}\nusecase\t8ffc8e75\t8ffc8e75\t0\t\t{}\nusecase\t41cbda4e\t41cbda4e\t0\t\t{}\nusecase\t9861741d\t9861741d\t0\t\t{}\nusecase\t3c621498\t3c621498\t0\t\t{}\nusecase\t42ac0557\t42ac0557\t0\t\t{}\nusecase\t0c08b519\t0c08b519\t4\t\t{\"theme\":\"redux-color\"}\nusecase\t87ad397f\t87ad397f\t6\t\t{\"theme\":\"redux-color\",\"usecase\":{\"colorScheme\":\"rotate\"}}\nusecase\te3fcd999\te3fcd999\t15\t\t{\"usecase\":{\"actorFontSize\":16,\"actorFontFamily\":\"Arial\",\"actorFontWeight\":\"bold\",\"usecaseFontSize\":14,\"usecaseFontFamily\":\"Georgia\",\"usecaseFontWeight\":\"normal\",\"nodeSpacing\":60,\"rankSpacing\":70,\"diagramPadding\":24,\"colorScheme\":\"role\",\"useMaxWidth\":false}}\nusecase\tfdb4ab69\tfdb4ab69\t0\t\t{}\nusecase\t1d8bc71f\t1d8bc71f\t0\t\t{}\nusecase\t89555ad6\t89555ad6\t0\t\t{}\nvenn\tc5ba5180\tc5ba5180\t0\t\t{}\nvenn\tc5ba5180\tc5ba5180\t5\t\t{\"theme\":\"default\",\"look\":\"classic\"}\nvenn\t591f610b\t591f610b\t0\t\t{}\nvenn\tc6e8ce8f\tc6e8ce8f\t0\t\t{}\nvenn\te9035191\te9035191\t0\t\t{}\nvenn\teae50661\teae50661\t0\t\t{}\nvenn\td12a6f90\td12a6f90\t0\t\t{}\nvenn\t70825725\t70825725\t0\t\t{}\nwardley\t1c0bbe4a\t1c0bbe4a\t0\t\t{}\nwardley\t62acbf9e\t62acbf9e\t0\t\t{}\nwardley\t3a4c2c97\t3a4c2c97\t0\t\t{}\nwardley\tccdca08f\tccdca08f\t0\t\t{}\nwardley\t57b41092\t57b41092\t0\t\t{}\nwardley\tb8a137e2\tb8a137e2\t0\t\t{}\nwardley\t0947cc21\t0947cc21\t0\t\t{}\nwardley\t0dad3970\t0dad3970\t0\t\t{}\nwardley\t25be3598\t25be3598\t0\t\t{}\nwardley\tb6b574d4\tb6b574d4\t0\t\t{}\nwardley\t0bcd3289\t0bcd3289\t0\t\t{}\nwardley\t8e666067\t8e666067\t0\t\t{}\nwardley\tb14ae374\tb14ae374\t0\t\t{}\nwardley\t9ea94d16\t9ea94d16\t0\t\t{}\nwardley\tdb6c2fcc\tdb6c2fcc\t0\t\t{}\nwardley\t55cdf233\t55cdf233\t0\t\t{}\nwardley\ta94dafab\ta94dafab\t0\t\t{}\nwardley\t1d346980\t1d346980\t0\t\t{}\nwardley\tedb9d838\tedb9d838\t0\t\t{}\nxychart\t26ce85c4\t26ce85c4\t0\t\t{}\nxychart\te9a4a843\te9a4a843\t0\t\t{}\nxychart\tba1a5b40\t4fdd9931\t6\t\t{\"themeVariables\":{\"xyChart\":{\"plotColorPalette\":\"#000000, #0000FF, #00FF00, #FF0000\"}}}\nxychart\t56fad8b3\t56fad8b3\t5\t\t{\"xyChart\":{\"showDataLabel\":true}}\nxychart\t56fad8b3\t56fad8b3\t6\t\t{\"xyChart\":{\"showDataLabel\":true,\"showDataLabelOutsideBar\":true}}\nxychart\tf26ad289\tf26ad289\t0\t\t{}\nxychart\t76b00374\t76b00374\t0\t\t{}\nxychart\t26ce85c4\t26ce85c4\t10\t\t{\"xyChart\":{\"width\":900,\"height\":600,\"showDataLabel\":true},\"themeVariables\":{\"xyChart\":{\"titleColor\":\"#ff0000\"}}}\nflowchart\t86ca7817\t86ca7817\t0\t\t{}\nflowchart\t86ca7817\t86ca7817\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nflowchart\tbf9c9d73\tcf512fbd\t0\t\t{}\nflowchart\tb1ff7017\tb1ff7017\t4\t\t{\"htmlLabels\":false}\nflowchart\tf9e193e4\tf9e193e4\t0\t\t{}\nflowchart\t7309203b\t7309203b\t0\t\t{}\nblock\t900d6434\t900d6434\t0\t\t{}\nblock\tcb4077c8\tcb4077c8\t0\t\t{}\nblock\t20b9d781\t20b9d781\t0\t\t{}\nblock\tf9110247\tf9110247\t0\t\t{}\nblock\t8c911ce0\t8c911ce0\t0\t\t{}\nblock\tc25a2a64\tc25a2a64\t0\t\t{}\nc4\t0760b980\taec01bf0\t0\t\t{}\nc4\tb2d6508a\t8bfcaa5a\t0\t\t{}\nc4\te2be2b28\t0f070818\t0\t\t{}\nc4\t0d1fbbd8\t0a9df668\t0\t\t{}\nc4\tf80cad6a\td314fefa\t0\t\t{}\nc4\t802e5294\tb7d44f44\t0\t\t{}\nclass\t10bcb468\t10bcb468\t5\t\t{\"class\":{\"hierarchicalNamespaces\":false}}\nclass\te5eb7517\te5eb7517\t3\tReef animal example\t{}\nclass\t8e5c892b\t8e5c892b\t3\tAnimal example\t{}\nclass\tb211c445\tb211c445\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nclass\t32c5753c\t32c5753c\t0\t\t{}\nclass\td66b78a3\td66b78a3\t5\t\t{\"class\":{\"hierarchicalNamespaces\":false}}\ner\t31853fa1\t31853fa1\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\ner\t0168cfe4\t0168cfe4\t0\t\t{}\ner\t2a8ea064\t2a8ea064\t0\t\t{}\ner\t19f47e9c\t19f47e9c\t0\t\t{}\ner\t00abc6a6\t00abc6a6\t0\t\t{}\ner\t9cb718e8\t9cb718e8\t0\t\t{}\neventmodeling\t95575a80\t95575a80\t0\t\t{}\neventmodeling\tbc71546d\tbc71546d\t0\t\t{}\neventmodeling\t9848114b\t9848114b\t0\t\t{}\neventmodeling\t9a983b66\t9a983b66\t0\t\t{}\neventmodeling\tf07a056c\tf07a056c\t0\t\t{}\neventmodeling\t919bda1b\t919bda1b\t0\t\t{}\ngantt\t47fa1b6b\t47fa1b6b\t0\t\t{}\ngantt\t1384b122\t66295b79\t0\t\t{}\ngantt\td5da20db\td5da20db\t3\t\t{\"gantt\":{\"displayMode\":\"compact\"}}\ngantt\t47fa1b6b\t4a0dfcbf\t0\t\t{}\ngantt\ta0552be7\ta0552be7\t0\t\t{}\ngantt\t7aef81dc\t02791088\t20\tIgnored if specified in chart\t{\"themeCSS\":\" // YAML supports multiline strings using a newline markers: \\n #item36 { fill: CadetBlue }       \\n\\n// Custom marker workaround CSS from forum (below)    \\n rect[id^=workaround] { height: calc(100% - 50px) ; transform: translate(9px, 25px); y: 0; width: 1.5px; stroke: none; fill: red; }   \\n text[id^=workaround] { fill: red; y: 100%; font-size: 15px;} \",\"gantt\":{\"useWidth\":400,\"rightPadding\":0,\"topAxis\":true,\"numberSectionStyles\":2,\"displayMode\":\"compact\"}}\ngitGraph\t21acb06a\t1758cf42\t7\t\t{\"logLevel\":\"debug\",\"theme\":\"base\",\"gitGraph\":{\"showBranches\":false}}\ngitGraph\td6aed840\t6c6f8b28\t8\t\t{\"logLevel\":\"debug\",\"theme\":\"base\",\"gitGraph\":{\"showBranches\":false,\"showCommitLabel\":false}}\ngitGraph\t21acb06a\t1758cf42\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"base\"}\ngitGraph\t21acb06a\t1758cf42\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"forest\"}\ngitGraph\t2b61d294\t30b8cf7c\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"default\"}\ngitGraph\t21acb06a\t1758cf42\t5\t\t{\"logLevel\":\"debug\",\"theme\":\"neutral\"}\ninfo\tc7671c9d\tc7671c9d\t0\t\t{}\ninfo\t1df1e5e0\t1df1e5e0\t0\t\t{}\njourney\tb3182409\tb3182409\t0\t\t{}\nkanban\t48e74608\t48e74608\t0\t\t{}\nkanban\td96df6e1\td96df6e1\t0\t\t{}\nkanban\te66e4a3b\te66e4a3b\t5\t\t{\"kanban\":{\"ticketBaseUrl\":\"https://mermaidchart.atlassian.net/browse/#TICKET#\"}}\nmindmap\t51d17bc0\t51d17bc0\t0\t\t{}\nmindmap\tc1c13312\tc1c13312\t0\t\t{}\nmindmap\tac91c0a1\tac91c0a1\t0\t\t{}\nmindmap\t267e8a34\t267e8a34\t0\t\t{}\nmindmap\tdee4894d\tdee4894d\t0\t\t{}\nmindmap\t7b454237\t7b454237\t0\t\t{}\npacket\t9cd72ace\t9cd72ace\t3\tTCP Packet\t{}\npacket\tb1f395a1\tb1f395a1\t0\t\t{}\npacket\t480e3a55\t480e3a55\t7\t\t{\"packet\":{\"showBits\":true,\"bitOrder\":\"descending\",\"bitsPerRow\":16}}\npacket\td9cdd28e\td9cdd28e\t8\t\t{\"packet\":{\"showBits\":true},\"themeVariables\":{\"packet\":{\"startByteColor\":\"red\"}}}\npie\t39524549\t39524549\t0\t\t{}\npie\t46193240\t46193240\t0\t\t{}\npie\t90cd0959\t90cd0959\t9\t\t{\"pie\":{\"textPosition\":0.5,\"donutHole\":0.2,\"highlightSlice\":\"Potassium\"},\"themeVariables\":{\"pieOuterStrokeWidth\":\"5px\"}}\nquadrantChart\te4d3e1ad\te4d3e1ad\t0\t\t{}\nquadrantChart\tb076946c\tb076946c\t8\t\t{\"quadrantChart\":{\"chartWidth\":400,\"chartHeight\":400},\"themeVariables\":{\"quadrant1TextFill\":\"ff0000\"}}\nquadrantChart\t15d42772\t15d42772\t0\t\t{}\nrequirement\t4d380f9d\t4d380f9d\t0\t\t{}\nrequirement\t4d380f9d\t4d380f9d\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nrequirement\t2de2bdfc\t3c65660c\t0\t\t{}\nrequirement\t2d1aaa3b\t2d1aaa3b\t0\t\t{}\nrequirement\t5f9ec524\t5f9ec524\t0\t\t{}\nrequirement\td6cdbc5e\td6cdbc5e\t0\t\t{}\nsankey\tee0ebbb7\tee0ebbb7\t5\t\t{\"sankey\":{\"showValues\":false}}\nsankey\tc8ad81d9\tc7bea05c\t0\t\t{}\nsankey\tb670e80b\tb670e80b\t0\t\t{}\nsankey\t70ea3ddf\t70ea3ddf\t6\t\t{\"sankey\":{\"showValues\":false,\"labelStyle\":\"outlined\"}}\nsankey\t70ea3ddf\t70ea3ddf\t7\t\t{\"sankey\":{\"showValues\":false,\"nodeWidth\":15,\"nodePadding\":20}}\nsankey\t70ea3ddf\t70ea3ddf\t9\t\t{\"sankey\":{\"showValues\":false,\"nodeColors\":{\"Electricity grid\":\"#4e79a7\",\"Industry\":\"#e15759\",\"Losses\":\"#bab0ab\"}}}\nsequence\t82030f1f\t82030f1f\t0\t\t{}\nsequence\t82030f1f\t82030f1f\t5\t\t{\"theme\":\"default\",\"look\":\"classic\"}\nsequence\t93ef0dd7\t93ef0dd7\t0\t\t{}\nsequence\ta64fcf96\ta64fcf96\t0\t\t{}\nsequence\te28b3e3c\te28b3e3c\t0\t\t{}\nsequence\tf32f9e93\tf32f9e93\t0\t\t{}\nstate\t08d97933\t08d97933\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nstate\t561e8206\t561e8206\t0\t\t{}\nstate\t3c5f8537\t3c5f8537\t0\t\t{}\nstate\t10cafb4a\t7f439784\t0\t\t{}\nstate\t3df1e32b\t3df1e32b\t0\t\t{}\nstate\t9de05457\t9de05457\t0\t\t{}\ntimeline\t9186dc5f\t9186dc5f\t0\t\t{}\ntimeline\tbc027511\tbc027511\t0\t\t{}\ntimeline\tb21681db\tb21681db\t0\t\t{}\ntimeline\tba5adcc2\tba5adcc2\t0\t\t{}\ntimeline\tcf8fa2c1\tcf8fa2c1\t0\t\t{}\ntimeline\t187c586d\t797f833f\t11\t\t{\"logLevel\":\"debug\",\"theme\":\"default\",\"themeVariables\":{\"cScale0\":\"#ff0000\",\"cScaleLabel0\":\"#ffffff\",\"cScale1\":\"#00ff00\",\"cScale2\":\"#0000ff\",\"cScaleLabel2\":\"#ffffff\"}}\nxychart\tf5dafdb9\tf5dafdb9\t0\t\t{}\nxychart\t16525762\t16525762\t6\t\t{\"xyChart\":{\"showDataLabel\":true,\"showDataLabelOutsideBar\":true}}\nxychart\t26ce85c4\t26ce85c4\t0\t\t{}\nxychart\tba1a5b40\t4fdd9931\t6\t\t{\"themeVariables\":{\"xyChart\":{\"plotColorPalette\":\"#000000, #0000FF, #00FF00, #FF0000\"}}}\nxychart\t56fad8b3\t56fad8b3\t6\t\t{\"xyChart\":{\"showDataLabel\":true,\"showDataLabelOutsideBar\":true}}\nxychart\t26ce85c4\t26ce85c4\t10\t\t{\"xyChart\":{\"width\":900,\"height\":600,\"showDataLabel\":true},\"themeVariables\":{\"xyChart\":{\"titleColor\":\"#ff0000\"}}}\nagentflow\tb8d70f82\tb8d70f82\t0\t\t{}\nagentflow\tb8d70f82\tb8d70f82\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nagentflow\t4ad0f60c\t4ad0f60c\t0\t\t{}\nagentflow\t5f427ced\t5f427ced\t0\t\t{}\nagentflow\t33b1a180\t33b1a180\t0\t\t{}\nagentflow\t3c37ceda\t3c37ceda\t0\t\t{}\narchitecture\t010c3eb1\t010c3eb1\t0\t\t{}\narchitecture\tceea4841\tceea4841\t0\t\t{}\narchitecture\tf83b4985\tf83b4985\t0\t\t{}\narchitecture\t731d0f1a\t731d0f1a\t0\t\t{}\narchitecture\t79dc2a28\t79dc2a28\t0\t\t{}\narchitecture\t3e4e3a8c\t3e4e3a8c\t0\t\t{}\ncynefin\t2417ab33\t2417ab33\t0\t\t{}\ncynefin\t0d612224\t0d612224\t0\t\t{}\ncynefin\t941ea8ef\t941ea8ef\t0\t\t{}\nishikawa\t7f3ea0df\t7f3ea0df\t0\t\t{}\nradar\t4022b389\t4022b389\t3\tGrades\t{}\nradar\t27b10ab4\t27b10ab4\t0\t\t{}\nradar\tc3fafcc9\tc3fafcc9\t13\t\t{\"radar\":{\"axisScaleFactor\":0.25,\"curveTension\":0.1},\"theme\":\"base\",\"themeVariables\":{\"cScale0\":\"#FF0000\",\"cScale1\":\"#00FF00\",\"cScale2\":\"#0000FF\",\"radar\":{\"curveOpacity\":0}}}\nrailroad\tf0107b3c\tf0107b3c\t0\t\t{}\nrailroad\t50d1abe2\t50d1abe2\t0\t\t{}\nrailroad\t58a4cd5e\t58a4cd5e\t0\t\t{}\nrailroad\t35185047\t35185047\t0\t\t{}\nrailroad\t27a4d56b\t27a4d56b\t0\t\t{}\nrailroad\tca60248a\tca60248a\t0\t\t{}\nswimlane\t227f14f2\t227f14f2\t5\t\t{\"theme\":\"default\",\"look\":\"classic\"}\nswimlane\tae3bd4fb\tae3bd4fb\t0\t\t{}\nswimlane\t9abc10b2\t9abc10b2\t0\t\t{}\nswimlane\tfdfef640\tfdfef640\t0\t\t{}\nswimlane\t35e1c477\t35e1c477\t0\t\t{}\nswimlane\tc240666f\tc240666f\t0\t\t{}\ntreemap\t00c4ad6c\t00c4ad6c\t0\t\t{}\ntreemap\t4b069166\t4b069166\t0\t\t{}\ntreemap\t0a2af022\t0a2af022\t0\t\t{}\ntreemap\tbb56e46b\tbb56e46b\t5\t\t{\"treemap\":{\"diagramPadding\":200}}\ntreemap\t6c5978b4\t6c5978b4\t5\t\t{\"treemap\":{\"valueFormat\":\"$0,0\"}}\ntreemap\t723777bd\t723777bd\t5\t\t{\"treemap\":{\"valueFormat\":\"$.1%\"}}\ntreeView\tb6f22ab9\tb6f22ab9\t0\t\t{}\ntreeView\t84eccba8\t84eccba8\t0\t\t{}\ntreeView\t8400f678\t8400f678\t12\t\t{\"treeView\":{\"showIcons\":true,\"defaultIconPack\":\"material-icon-theme\",\"filenameIcons\":{\"Dockerfile\":\"docker\"},\"extensionIcons\":{\".ts\":\"typescript\",\".tsx\":\"react-ts\",\".txt\":\"none\"}}}\ntreeView\tc209a9a8\tc209a9a8\t0\t\t{}\ntreeView\t6a66342d\t6a66342d\t0\t\t{}\ntreeView\t8aceac1a\t8aceac1a\t11\t\t{\"treeView\":{\"rowIndent\":80,\"lineThickness\":3},\"themeVariables\":{\"treeView\":{\"labelFontSize\":\"20px\",\"labelColor\":\"#FF0000\",\"lineColor\":\"#00FF00\"}}}\nusecase\t423fc54e\t423fc54e\t6\t\t{\"theme\":\"default\",\"look\":\"classic\",\"layout\":\"dagre\"}\nusecase\t52b79d2f\t52b79d2f\t0\t\t{}\nusecase\t3c621498\t3c621498\t0\t\t{}\nusecase\t42ac0557\t42ac0557\t0\t\t{}\nusecase\te3fcd999\te3fcd999\t15\t\t{\"usecase\":{\"actorFontSize\":16,\"actorFontFamily\":\"Arial\",\"actorFontWeight\":\"bold\",\"usecaseFontSize\":14,\"usecaseFontFamily\":\"Georgia\",\"usecaseFontWeight\":\"normal\",\"nodeSpacing\":60,\"rankSpacing\":70,\"diagramPadding\":24,\"colorScheme\":\"role\",\"useMaxWidth\":false}}\nusecase\t1d8bc71f\t1d8bc71f\t0\t\t{}\nvenn\tc5ba5180\tc5ba5180\t0\t\t{}\nvenn\tc5ba5180\tc5ba5180\t5\t\t{\"theme\":\"default\",\"look\":\"classic\"}\nvenn\t591f610b\t591f610b\t0\t\t{}\nvenn\te9035191\te9035191\t0\t\t{}\nvenn\td12a6f90\td12a6f90\t0\t\t{}\nvenn\t70825725\t70825725\t0\t\t{}\nwardley\t284ce625\t284ce625\t0\t\t{}\nwardley\t3a4c2c97\t3a4c2c97\t0\t\t{}\nwardley\t0dad3970\t0dad3970\t0\t\t{}\nwardley\tb14ae374\tb14ae374\t0\t\t{}\nwardley\t55cdf233\t55cdf233\t0\t\t{}\nwardley\tedb9d838\tedb9d838\t0\t\t{}";
}
