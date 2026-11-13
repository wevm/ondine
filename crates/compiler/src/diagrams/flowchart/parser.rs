//! The flowchart parser: a hand-written equivalent of Mermaid's Jison grammar that reports the
//! same errors at the same tokens. Port of Pele's `src/diagrams/flowchart/parser.ts`.

use std::borrow::Cow;
use std::ops::Range;

use super::db::{FlowDb, js_trim};
use super::lexer::{Token, tokenize};
use super::tokens::T;
use super::types::{DocItem, FlowText, LabelType, LinkInfo};
use crate::error::Error;
use crate::lines::{self, Lines, Locator};
use crate::util::chars::is_space;

/// Tokens that continue an id.
fn is_id(t: T) -> bool {
    matches!(
        t,
        T::Num
            | T::NodeString
            | T::Down
            | T::Minus
            | T::Default
            | T::Comma
            | T::Colon
            | T::Amp
            | T::Brkt
            | T::Mult
            | T::UnicodeText
    )
}

/// Tokens of an alphanumeric name, such as a callback or curve.
fn is_alpha_num(t: T) -> bool {
    matches!(
        t,
        T::Num
            | T::UnicodeText
            | T::NodeString
            | T::Dir
            | T::Down
            | T::Minus
            | T::Comma
            | T::Colon
            | T::Amp
            | T::Brkt
            | T::Mult
    )
}

/// Tokens of a subgraph id written without brackets.
fn is_no_tags(t: T) -> bool {
    matches!(
        t,
        T::Num
            | T::NodeString
            | T::Space
            | T::Minus
            | T::Amp
            | T::UnicodeText
            | T::Colon
            | T::Mult
            | T::Brkt
            | T::Style
            | T::LinkStyle
            | T::ClassDef
            | T::Class
            | T::Click
            | T::Graph
            | T::Dir
            | T::Subgraph
            | T::SubgraphEnd
            | T::Down
            | T::Up
            | T::StartLink
    )
}

/// Tokens of one style declaration.
fn is_style_part(t: T) -> bool {
    matches!(
        t,
        T::Num | T::NodeString | T::Colon | T::Space | T::Brkt | T::Style
    )
}

/// Tokens that continue a node label.
fn is_text(t: T) -> bool {
    matches!(t, T::Text | T::TagStart | T::TagEnd | T::UnicodeText)
}

const TEXT_TOKENS: &str = "'UNICODE_TEXT', 'TEXT', 'TAGSTART'";

const STATEMENT_START: &str = "'SEMI', 'NEWLINE', 'SPACE', 'EOF', 'subgraph', 'acc_title', 'acc_descr', 'acc_descr_multiline_value', 'STYLE', 'LINKSTYLE', 'CLASSDEF', 'CLASS', 'CLICK', 'NODE_STRING'";

/// An open subgraph: its header and the body of its parent.
struct Frame {
    collect: bool,
    id: Option<FlowText>,
    /// The authored line of the `subgraph` keyword.
    line: Option<u32>,
    list: Vec<DocItem>,
    /// Whether the title is the id itself, which Pele tests by object identity.
    same: bool,
    title: Option<FlowText>,
}

struct Parser<'s, 'd> {
    db: &'d mut FlowDb,
    i: usize,
    /// Finds the authored lines of `src`, when the host asks for them.
    locator: Option<Locator>,
    src: &'s str,
    tokens: Vec<Token<'s>>,
}

/// Mermaid trims whitespace between a closing brace and the end of its line before parsing:
/// `source.replace(/}\s*\n/g, '}\n')`. Returns the ranges of `source` that removes, in order.
fn brace_runs(source: &str) -> Vec<Range<usize>> {
    let mut removed = Vec::new();
    if !source.contains('}') {
        return removed;
    }

    let mut from = 0;
    while let Some(at) = source.get(from..).and_then(|rest| rest.find('}')) {
        let tail_start = from + at + 1;
        let tail = source.get(tail_start..).unwrap_or("");
        let run = tail
            .char_indices()
            .find(|&(_, c)| !is_space(u32::from(c)))
            .map_or(tail.len(), |(end, _)| end);
        match tail.get(..run).and_then(|space| space.rfind('\n')) {
            Some(newline) => {
                if newline > 0 {
                    removed.push(tail_start..tail_start + newline);
                }
                from = tail_start + newline + 1;
            }
            None => from = tail_start,
        }
    }
    removed
}

/// Parses flowchart `source` into `db`. `lines` maps `source` to authored lines.
pub(crate) fn parse_flowchart(
    source: &str,
    lines: Option<&Lines>,
    db: &mut FlowDb,
) -> Result<(), Error> {
    let removed = brace_runs(source);
    let src = if removed.is_empty() {
        Cow::Borrowed(source)
    } else {
        Cow::Owned(lines::without(source, &removed))
    };
    let locator = lines.map(|lines| {
        let mut lines = lines.clone();
        lines.remove(source, &removed);
        lines.locate(&src)
    });

    let mut parser = Parser {
        db,
        i: 0,
        locator,
        src: &src,
        tokens: tokenize(&src),
    };
    parser.run()
}

impl Parser<'_, '_> {
    fn kind(&self) -> T {
        self.kind_at(self.i)
    }

    fn kind_at(&self, i: usize) -> T {
        self.tokens.get(i).map_or(T::End, |token| token.kind)
    }

    fn text_at(&self, i: usize) -> &str {
        self.tokens.get(i).map_or("", |token| &token.text)
    }

    /// Takes the current token's text.
    fn take(&mut self) -> String {
        let text = self.text_at(self.i).to_owned();
        self.i += 1;
        text
    }

    /// Makes the authored line of the current token the line that later model updates record,
    /// and returns it.
    fn mark(&mut self) -> Option<u32> {
        let locator = self.locator.as_ref()?;
        let start = self
            .tokens
            .get(self.i)
            .map_or(self.src.len(), |token| token.start);
        self.db.line = Some(locator.line(start));
        self.db.line
    }

    #[cold]
    fn fail(&self, expected: &str) -> Error {
        let t = self.kind();
        let start = self
            .tokens
            .get(self.i)
            .map_or(self.src.len(), |token| token.start);
        if t == T::Error {
            return Error::syntax("flowchart", self.src, start, "", true);
        }

        let mut detail = String::from("Expecting ");
        detail.push_str(expected);
        detail.push_str(", got '");
        detail.push_str(t.name());
        detail.push('\'');
        Error::syntax("flowchart", self.src, start, &detail, false)
    }

    fn expect(&mut self, kind: T, also: &str) -> Result<String, Error> {
        if self.kind() != kind {
            let mut expected = String::from("'");
            expected.push_str(kind.name());
            expected.push('\'');
            if !also.is_empty() {
                expected.push_str(", ");
                expected.push_str(also);
            }
            return Err(self.fail(&expected));
        }
        Ok(self.take())
    }

    fn separator(&mut self) -> Result<(), Error> {
        if !matches!(self.kind(), T::Newline | T::Semi | T::Eof) {
            return Err(self.fail("'SEMI', 'NEWLINE', 'EOF'"));
        }
        self.i += 1;
        Ok(())
    }

    /// Concatenates a run of tokens of one class, failing with `expected` when there is none.
    fn run_of(&mut self, class: fn(T) -> bool, expected: &str) -> Result<String, Error> {
        if !class(self.kind()) {
            return Err(self.fail(expected));
        }
        let mut s = self.take();
        while class(self.kind()) {
            s.push_str(self.text_at(self.i));
            self.i += 1;
        }
        Ok(s)
    }

    fn id_string(&mut self) -> Result<String, Error> {
        self.run_of(is_id, "'NODE_STRING'")
    }

    fn alpha_num(&mut self) -> Result<String, Error> {
        self.run_of(is_alpha_num, "'NODE_STRING'")
    }

    fn text(&mut self) -> Result<FlowText, Error> {
        let r#type = match self.kind() {
            T::Str => LabelType::String,
            T::MdStr => LabelType::Markdown,
            t if is_text(t) => LabelType::Text,
            _ => {
                let mut expected = String::from(TEXT_TOKENS);
                expected.push_str(", 'TAGEND', 'STR', 'MD_STR'");
                return Err(self.fail(&expected));
            }
        };
        let mut text = self.take();
        while is_text(self.kind()) {
            text.push_str(self.text_at(self.i));
            self.i += 1;
        }
        Ok(FlowText { text, r#type })
    }

    fn closed(&mut self, kind: T) -> Result<(), Error> {
        self.expect(kind, TEXT_TOKENS).map(drop)
    }

    fn edge_text(&mut self) -> Result<FlowText, Error> {
        let r#type = match self.kind() {
            T::Str => LabelType::String,
            T::MdStr => LabelType::Markdown,
            T::EdgeText | T::UnicodeText => LabelType::Text,
            _ => return Err(self.fail("'EDGE_TEXT', 'STR', 'MD_STR'")),
        };
        let mut text = self.take();
        while matches!(self.kind(), T::EdgeText | T::UnicodeText) {
            text.push_str(self.text_at(self.i));
            self.i += 1;
        }
        Ok(FlowText { text, r#type })
    }

    fn text_no_tags(&mut self) -> Result<FlowText, Error> {
        let r#type = match self.kind() {
            T::Str => LabelType::Text,
            T::MdStr => LabelType::Markdown,
            t if is_no_tags(t) => LabelType::Text,
            _ => return Err(self.fail("'NODE_STRING', 'STR', 'MD_STR'")),
        };
        let mut text = self.take();
        while is_no_tags(self.kind()) {
            text.push_str(self.text_at(self.i));
            self.i += 1;
        }
        Ok(FlowText { text, r#type })
    }

    fn shape_data(&mut self) -> String {
        let mut s = self.take();
        while self.kind() == T::ShapeData {
            s.push_str(self.text_at(self.i));
            self.i += 1;
        }
        s
    }

    /// A label between `open`, already consumed, and `close`, added with `shape`.
    fn shaped(&mut self, id: &str, close: T, shape: &str) -> Result<(), Error> {
        let label = self.text()?;
        self.closed(close)?;
        self.db
            .add_vertex(id, Some(label), Some(shape), None, None, None)
    }

    fn vertex(&mut self) -> Result<String, Error> {
        self.mark();
        let id = self.id_string()?;
        let open = self.kind();
        match open {
            T::Sqs => {
                self.i += 1;
                self.shaped(&id, T::Sqe, "square")?;
            }
            T::DoubleCircleStart => {
                self.i += 1;
                self.shaped(&id, T::DoubleCircleEnd, "doublecircle")?;
            }
            T::Ps => {
                self.i += 1;
                if self.kind() == T::Ps {
                    self.i += 1;
                    let label = self.text()?;
                    self.closed(T::Pe)?;
                    self.expect(T::Pe, "")?;
                    self.db
                        .add_vertex(&id, Some(label), Some("circle"), None, None, None)?;
                } else {
                    self.shaped(&id, T::Pe, "round")?;
                }
            }
            T::EllipseStart => {
                self.i += 1;
                self.shaped(&id, T::EllipseEnd, "ellipse")?;
            }
            T::StadiumStart => {
                self.i += 1;
                self.shaped(&id, T::StadiumEnd, "stadium")?;
            }
            T::SubroutineStart => {
                self.i += 1;
                self.shaped(&id, T::SubroutineEnd, "subroutine")?;
            }
            T::VertexWithPropsStart => {
                self.i += 1;
                let field = self.expect(T::NodeString, "")?;
                self.expect(T::Colon, "")?;
                let value = self.expect(T::NodeString, "")?;
                self.expect(T::Pipe, "")?;
                let label = self.text()?;
                self.closed(T::Sqe)?;
                self.db.add_vertex(
                    &id,
                    Some(label),
                    Some("rect"),
                    None,
                    Some((field, value)),
                    None,
                )?;
            }
            T::CylinderStart => {
                self.i += 1;
                self.shaped(&id, T::CylinderEnd, "cylinder")?;
            }
            T::DiamondStart => {
                self.i += 1;
                if self.kind() == T::DiamondStart {
                    self.i += 1;
                    let label = self.text()?;
                    self.closed(T::DiamondStop)?;
                    self.expect(T::DiamondStop, "")?;
                    self.db
                        .add_vertex(&id, Some(label), Some("hexagon"), None, None, None)?;
                } else {
                    self.shaped(&id, T::DiamondStop, "diamond")?;
                }
            }
            T::TagEnd => {
                self.i += 1;
                self.shaped(&id, T::Sqe, "odd")?;
            }
            T::TrapStart => {
                self.i += 1;
                let label = self.text()?;
                let shape = if self.kind() == T::InvTrapEnd {
                    self.i += 1;
                    "lean_right"
                } else {
                    self.closed(T::TrapEnd)?;
                    "trapezoid"
                };
                self.db
                    .add_vertex(&id, Some(label), Some(shape), None, None, None)?;
            }
            T::InvTrapStart => {
                self.i += 1;
                let label = self.text()?;
                let shape = if self.kind() == T::TrapEnd {
                    self.i += 1;
                    "lean_left"
                } else {
                    self.closed(T::InvTrapEnd)?;
                    "inv_trapezoid"
                };
                self.db
                    .add_vertex(&id, Some(label), Some(shape), None, None, None)?;
            }
            _ => self.db.add_vertex(&id, None, None, None, None, None)?,
        }
        Ok(id)
    }

    fn styled_vertex(&mut self) -> Result<String, Error> {
        let id = self.vertex()?;
        if self.kind() == T::StyleSeparator {
            self.i += 1;
            let class = self.id_string()?;
            self.db.set_class(&id, &class);
        }
        Ok(id)
    }

    /// A node group joined by `&`, and the shape data that trails its last node. Pele keeps the
    /// trailing data in `pendingData`.
    fn node(&mut self) -> Result<(Vec<String>, Option<String>), Error> {
        let mut ids = vec![self.styled_vertex()?];
        loop {
            let data = (self.kind() == T::ShapeData).then(|| self.shape_data());
            if self.kind() != T::Space {
                return Ok((ids, data));
            }

            self.skip_spaces();
            if self.kind() != T::Amp {
                if data.is_some() {
                    return Err(self.fail("'AMP'"));
                }
                return Ok((ids, None));
            }

            self.i += 1;
            if self.kind() != T::Space {
                return Err(self.fail("'SPACE'"));
            }
            self.skip_spaces();
            let next = self.styled_vertex()?;
            if let (Some(data), Some(last)) = (&data, ids.last()) {
                self.db
                    .add_vertex(last, None, None, None, None, Some(data))?;
            }
            ids.push(next);
        }
    }

    /// Steps past the current token and any spaces after it.
    fn skip_spaces(&mut self) {
        self.i += 1;
        while self.kind() == T::Space {
            self.i += 1;
        }
    }

    fn link(&mut self) -> Result<LinkInfo, Error> {
        let id = (self.kind() == T::LinkId).then(|| self.take());

        if self.kind() == T::Link {
            let text = self.take();
            let mut info = FlowDb::destruct_link(&text, None);
            info.id = id;
            if self.kind() == T::Pipe {
                self.i += 1;
                info.text = Some(self.text()?);
                self.closed(T::Pipe)?;
                if self.kind() == T::Space {
                    self.i += 1;
                }
            }
            return Ok(info);
        }

        if self.kind() == T::StartLink {
            let start = self.take();
            let label = self.edge_text()?;
            let end = self.expect(T::Link, "'EDGE_TEXT'")?;
            let mut info = FlowDb::destruct_link(&end, Some(&start));
            info.text = Some(label);
            info.id = id;
            return Ok(info);
        }

        Err(self.fail("'LINK', 'START_LINK'"))
    }

    fn vertex_statement(&mut self) -> Result<Vec<String>, Error> {
        let (mut stmt, data) = self.node()?;
        if let (Some(data), Some(last)) = (&data, stmt.last()) {
            self.db
                .add_vertex(last, None, None, None, None, Some(data))?;
        }

        let mut nodes = stmt.clone();
        loop {
            if !matches!(self.kind(), T::Link | T::StartLink | T::LinkId) {
                return Ok(nodes);
            }
            let line = self.mark();
            let info = self.link()?;
            let (next, data) = self.node()?;
            if let (Some(data), Some(last)) = (&data, next.last()) {
                self.db
                    .add_vertex(last, None, None, None, None, Some(data))?;
            }
            // An edge belongs to the line of its arrow, which may differ from its end node's.
            self.db.line = line;
            self.db.add_link(&stmt, &next, &info)?;

            let mut joined = next.clone();
            joined.append(&mut nodes);
            nodes = joined;
            stmt = next;
        }
    }

    fn style(&mut self) -> Result<String, Error> {
        self.run_of(is_style_part, "'NODE_STRING', 'NUM', 'COLON', 'BRKT'")
    }

    fn styles_opt(&mut self) -> Result<Vec<String>, Error> {
        let mut out = vec![self.style()?];
        while self.kind() == T::Comma {
            self.i += 1;
            out.push(self.style()?);
        }
        Ok(out)
    }

    fn link_style_statement(&mut self) -> Result<(), Error> {
        self.i += 1;
        self.expect(T::Space, "")?;
        let mut positions = Vec::new();
        if self.kind() == T::Default {
            positions.push(self.take());
        } else {
            positions.push(self.expect(T::Num, "'DEFAULT'")?);
            while self.kind() == T::Comma {
                self.i += 1;
                positions.push(self.expect(T::Num, "")?);
            }
        }
        self.expect(T::Space, "")?;

        if self.kind() != T::Interpolate {
            let styles = self.styles_opt()?;
            return self.db.update_link(&positions, styles);
        }

        self.i += 1;
        self.expect(T::Space, "")?;
        let interpolate = self.alpha_num()?;
        if self.kind() == T::Space {
            self.i += 1;
            let styles = self.styles_opt()?;
            self.db.update_link_interpolate(&positions, &interpolate)?;
            self.db.update_link(&positions, styles)
        } else {
            self.db.update_link_interpolate(&positions, &interpolate)
        }
    }

    fn link_tail(&mut self, id: &str, url: &str) -> Result<(), Error> {
        if self.kind() != T::Space {
            self.db.set_link(id, url, None);
            return Ok(());
        }

        self.i += 1;
        if self.kind() == T::LinkTarget {
            let target = self.take();
            self.db.set_link(id, url, Some(&target));
            return Ok(());
        }

        let tooltip = self.expect(T::Str, "'LINK_TARGET'")?;
        if self.kind() == T::Space {
            self.i += 1;
            let target = self.expect(T::LinkTarget, "")?;
            self.db.set_link(id, url, Some(&target));
        } else {
            self.db.set_link(id, url, None);
        }
        self.db.set_tooltip(id, &tooltip);
        Ok(())
    }

    fn click_statement(&mut self) -> Result<(), Error> {
        let id = self.take();
        match self.kind() {
            T::CallbackName => {
                self.i += 1;
                if self.kind() == T::CallbackArgs {
                    self.i += 1;
                }
                let mut tooltip = None;
                if self.kind() == T::Space {
                    self.i += 1;
                    tooltip = Some(self.expect(T::Str, "")?);
                }
                self.db.set_click_event(&id);
                if let Some(tooltip) = tooltip {
                    self.db.set_tooltip(&id, &tooltip);
                }
                Ok(())
            }
            T::Href => {
                self.i += 1;
                let url = self.expect(T::Str, "")?;
                self.link_tail(&id, &url)
            }
            T::Str => {
                let url = self.take();
                self.link_tail(&id, &url)
            }
            _ => {
                self.alpha_num()?;
                if self.kind() == T::Space {
                    self.i += 1;
                    let tooltip = self.expect(T::Str, "")?;
                    self.db.set_click_event(&id);
                    self.db.set_tooltip(&id, &tooltip);
                } else {
                    self.db.set_click_event(&id);
                }
                Ok(())
            }
        }
    }

    fn subgraph_header(&mut self) -> Result<Frame, Error> {
        let line = self.mark();
        self.i += 1;
        if matches!(self.kind(), T::Newline | T::Semi | T::Eof) {
            self.i += 1;
            return Ok(Frame {
                collect: false,
                id: None,
                line,
                list: Vec::new(),
                same: false,
                title: None,
            });
        }

        self.expect(T::Space, "'SEMI', 'NEWLINE', 'EOF'")?;
        let id = self.text_no_tags()?;
        let mut title = None;
        if self.kind() == T::Sqs {
            self.i += 1;
            title = Some(self.text()?);
            self.closed(T::Sqe)?;
        }
        if !matches!(self.kind(), T::Newline | T::Semi | T::Eof) {
            return Err(self.fail("'SEMI', 'NEWLINE', 'EOF', 'SQS'"));
        }
        self.i += 1;

        let same = title.is_none();
        let title = title.unwrap_or_else(|| id.clone());
        Ok(Frame {
            collect: false,
            id: Some(id),
            line,
            list: Vec::new(),
            same,
            title: Some(title),
        })
    }

    /// One statement, or `None` for statements that add nothing to a subgraph body.
    fn statement(&mut self) -> Result<Option<DocItem>, Error> {
        self.mark();
        let t = self.kind();
        match t {
            T::Style => {
                self.i += 1;
                self.expect(T::Space, "")?;
                let id = self.id_string()?;
                self.expect(T::Space, "")?;
                let styles = self.styles_opt()?;
                self.db
                    .add_vertex(&id, None, None, Some(styles), None, None)?;
                self.separator()?;
                Ok(None)
            }
            T::LinkStyle => {
                self.link_style_statement()?;
                self.separator()?;
                Ok(None)
            }
            T::ClassDef => {
                self.i += 1;
                self.expect(T::Space, "")?;
                let id = self.id_string()?;
                self.expect(T::Space, "")?;
                let styles = self.styles_opt()?;
                self.db.add_class(&id, &styles);
                self.separator()?;
                Ok(None)
            }
            T::Class => {
                self.i += 1;
                self.expect(T::Space, "")?;
                let ids = self.id_string()?;
                self.expect(T::Space, "")?;
                let class = self.id_string()?;
                self.db.set_class(&ids, &class);
                self.separator()?;
                Ok(None)
            }
            T::Click => {
                self.click_statement()?;
                self.separator()?;
                Ok(None)
            }
            T::DirectionTb | T::DirectionBt | T::DirectionRl | T::DirectionLr | T::DirectionTd => {
                self.i += 1;
                let value = match t {
                    T::DirectionTb => "TB",
                    T::DirectionBt => "BT",
                    T::DirectionRl => "RL",
                    T::DirectionLr => "LR",
                    _ => "TD",
                };
                Ok(Some(DocItem::Dir(value)))
            }
            T::AccTitle => {
                self.i += 1;
                let value = js_trim(&self.expect(T::AccTitleValue, "")?).to_owned();
                self.db.set_acc_title(&value);
                Ok(Some(DocItem::Str(value)))
            }
            T::AccDescr => {
                self.i += 1;
                let value = js_trim(&self.expect(T::AccDescrValue, "")?).to_owned();
                self.db.set_acc_description(&value);
                Ok(Some(DocItem::Str(value)))
            }
            T::AccDescrMultilineValue => {
                let value = js_trim(&self.take()).to_owned();
                self.db.set_acc_description(&value);
                Ok(Some(DocItem::Str(value)))
            }
            _ => {
                if !is_id(t) {
                    return Err(self.fail(STATEMENT_START));
                }
                let nodes = self.vertex_statement()?;
                self.separator()?;
                Ok(Some(DocItem::Ids(nodes)))
            }
        }
    }

    /// Subgraphs nest without recursion: each open one keeps the list and collect flag of its
    /// parent.
    fn document(&mut self) -> Result<(), Error> {
        let mut open: Vec<Frame> = Vec::new();
        let mut list: Vec<DocItem> = Vec::new();
        let mut collect = false;
        loop {
            let t = self.kind();
            if t == T::End {
                break;
            }

            if t == T::SubgraphEnd {
                let Some(frame) = open.pop() else {
                    break;
                };
                self.i += 1;
                // A subgraph belongs to the line of its header, not of its `end`.
                self.db.line = frame.line;
                let id = self.db.add_subgraph(
                    frame.id.as_ref(),
                    &list,
                    frame.title.as_ref(),
                    frame.same,
                );
                list = frame.list;
                collect = frame.collect;
                if collect {
                    list.push(DocItem::Str(id));
                }
            } else if matches!(t, T::Semi | T::Newline | T::Space | T::Eof) {
                if collect {
                    list.push(DocItem::Str(self.text_at(self.i).to_owned()));
                }
                self.i += 1;
            } else if t == T::Subgraph {
                let mut frame = self.subgraph_header()?;
                frame.list = std::mem::take(&mut list);
                frame.collect = collect;
                open.push(frame);
                collect = true;
            } else if let Some(item) = self.statement()?
                && collect
            {
                list.push(item);
            }
        }

        if !open.is_empty() {
            return Err(self.fail("'end'"));
        }
        Ok(())
    }

    fn run(&mut self) -> Result<(), Error> {
        while matches!(self.kind(), T::Space | T::Newline) {
            self.i += 1;
        }
        self.expect(T::Graph, "'SPACE', 'NEWLINE'")?;

        if self.kind() == T::NoDir {
            self.i += 1;
            self.db.set_direction("TB");
        } else {
            let dir = self.expect(T::Dir, "'NODIR'")?;
            let t = self.kind();
            if t == T::Space {
                while self.kind() == T::Space {
                    self.i += 1;
                }
                if self.kind() != T::Newline {
                    return Err(self.fail("'NEWLINE', 'SPACE'"));
                }
            } else if t != T::Semi && t != T::Newline {
                return Err(self.fail("'SEMI', 'NEWLINE', 'SPACE'"));
            }
            self.i += 1;
            self.db.set_direction(&dir);
        }

        self.document()?;
        if self.kind() != T::End {
            return Err(self.fail(STATEMENT_START));
        }
        Ok(())
    }
}
