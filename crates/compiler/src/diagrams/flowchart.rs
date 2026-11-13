//! Flowcharts: Mermaid's `graph` and `flowchart` diagrams, which swimlane diagrams share. Port of
//! Pele's `src/diagrams/flowchart/index.ts`.

pub(crate) mod db;
pub(crate) mod graph;
pub(crate) mod lexer;
pub(crate) mod parser;
pub(crate) mod render;
pub(crate) mod shapes;
pub(crate) mod tokens;
pub(crate) mod types;

use super::{Diagram, Input, Rendered};
use crate::error::Error;
use crate::lines::Lines;
use crate::svg::root::RenderOptions;
use crate::util::url::UrlOptions;
use crate::yaml::Value;

/// The `flowchart` diagram type.
pub(crate) const DIAGRAM: Diagram = Diagram {
    keep_comments: false,
    render: render_input,
    section: None,
};

fn render_input(
    input: &Input<'_>,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
) -> Result<Rendered, Error> {
    let db = parse(
        input.source,
        input.lines,
        input.config,
        input.title,
        Some(input.max_edges),
    )?;
    Ok(render::render(&db, input.config, options, url, None))
}

/// The parsed flowchart model: Pele's `FlowDb`.
pub(crate) use db::FlowDb as Model;

/// Parses preprocessed flowchart `source`, as Pele's `flowchart.parse` does. `lines` maps
/// `source` to authored lines, which the nodes, edges, and subgraphs then record. `config` is the
/// diagram configuration, of which `flowchart.inheritDir` is read. `title` comes from front
/// matter. `max_edges` bounds the edge count.
///
/// # Errors
///
/// Returns a syntax error for input Mermaid rejects, a semantic error for unknown shapes and
/// out-of-range `linkStyle` indexes, and a limit error past `max_edges` edges.
pub(crate) fn parse(
    source: &str,
    lines: Option<&Lines>,
    config: &Value,
    title: Option<&str>,
    max_edges: Option<usize>,
) -> Result<Model, Error> {
    let inherit_dir = matches!(
        property(config, "flowchart").and_then(|flow| property(flow, "inheritDir")),
        Some(Value::Bool(true))
    );
    let mut db = Model::new(db::Options {
        inherit_dir,
        max_edges,
    });
    if let Some(title) = title.filter(|title| !title.is_empty()) {
        db.title = Some(title.to_owned());
    }
    parser::parse_flowchart(source, lines, &mut db)?;
    Ok(db)
}

/// A property of a YAML mapping, as JavaScript's `value.key` reads it.
fn property<'v>(value: &'v Value, key: &str) -> Option<&'v Value> {
    let Value::Object(entries) = value else {
        return None;
    };
    entries
        .iter()
        .find(|(own, _)| own == key)
        .map(|(_, value)| value)
}

/// Serializes `model` as `JSON.stringify` serializes Pele's parsed `FlowDb` after this
/// normalization, which the parity tests apply on Pele's side:
///
/// - The top level holds `type`, `direction`, `nodes`, `edges`, `subgraphs`, `classes`,
///   `tooltips`, `defaultEdgeStyle`, `defaultInterpolate`, `title`, `accTitle`, and `accDescr`, in
///   that order. Private `FlowDb` fields are left out.
/// - The `nodes`, `classes`, and `tooltips` maps become plain objects with
///   `Object.fromEntries`, so their keys follow JavaScript property order: integer-like keys
///   (array indexes) ascending, then the other keys in map insertion order. Object values in the
///   model (`props`, `metadata`, and YAML mappings inside it) follow the same order.
/// - Node, edge, subgraph, and class records list their fields in the declaration order of
///   Pele's `types.ts` interfaces.
/// - `undefined` fields are dropped and non-finite numbers become `null`, as `JSON.stringify`
///   does.
///
/// On Pele's side:
///
/// ```ts
/// const NODE = ['id', 'text', 'labelType', 'type', 'styles', 'classes', 'dir', 'props', 'link',
///   'linkTarget', 'icon', 'form', 'pos', 'img', 'constraint', 'assetWidth', 'assetHeight'];
/// const EDGE = ['start', 'end', 'type', 'text', 'labelType', 'stroke', 'length', 'style',
///   'classes', 'id', 'isUserDefinedId', 'interpolate', 'animate', 'animation'];
/// const SUBGRAPH = ['id', 'nodes', 'title', 'classes', 'dir', 'labelType', 'metadata'];
/// const CLASS = ['id', 'styles', 'textStyles'];
/// const pick = (o, keys) => Object.fromEntries(keys.map((k) => [k, o[k]]));
/// JSON.stringify({
///   type: db.type, direction: db.direction,
///   nodes: Object.fromEntries([...db.nodes].map(([k, v]) => [k, pick(v, NODE)])),
///   edges: db.edges.map((e) => pick(e, EDGE)),
///   subgraphs: db.subgraphs.map((s) => pick(s, SUBGRAPH)),
///   classes: Object.fromEntries([...db.classes].map(([k, v]) => [k, pick(v, CLASS)])),
///   tooltips: Object.fromEntries(db.tooltips),
///   defaultEdgeStyle: db.defaultEdgeStyle, defaultInterpolate: db.defaultInterpolate,
///   title: db.title, accTitle: db.accTitle, accDescr: db.accDescr,
/// });
/// ```
#[cfg(feature = "model")]
pub(crate) fn to_json(model: &Model) -> String {
    let mut json = json::Writer::default();
    json.open('{');
    json.key("type");
    json.string("flowchart");
    json.optional_string("direction", model.direction.as_deref());

    json.key("nodes");
    json.open('{');
    for node in json::property_order(&model.nodes, |node| &node.id) {
        json.key(&node.id);
        json.open('{');
        json.key("id");
        json.string(&node.id);
        json.optional_string("text", node.text.as_deref());
        json.key("labelType");
        json.string(node.label_type.as_str());
        json.optional_string("type", node.r#type.as_deref());
        json.key("styles");
        json.strings(&node.styles);
        json.key("classes");
        json.strings(&node.classes);
        json.optional_string("dir", node.dir.as_deref());
        if let Some(props) = &node.props {
            json.key("props");
            json.open('{');
            for (key, value) in json::property_order(props, |(key, _)| key) {
                json.key(key);
                json.string(value);
            }
            json.close('}');
        }
        json.optional_string("link", node.link.as_deref());
        json.optional_string("linkTarget", node.link_target.as_deref());
        json.optional_string("icon", node.icon.as_deref());
        json.optional_string("form", node.form.as_deref());
        json.optional_string("pos", node.pos.as_deref());
        json.optional_string("img", node.img.as_deref());
        json.optional_string("constraint", node.constraint.as_deref());
        if let Some(width) = node.asset_width {
            json.key("assetWidth");
            json.number(width);
        }
        if let Some(height) = node.asset_height {
            json.key("assetHeight");
            json.number(height);
        }
        json.close('}');
    }
    json.close('}');

    json.key("edges");
    json.open('[');
    for edge in &model.edges {
        json.item();
        json.open('{');
        json.key("start");
        json.string(&edge.start);
        json.key("end");
        json.string(&edge.end);
        json.optional_string("type", edge.r#type.as_deref());
        json.key("text");
        json.string(&edge.text);
        json.key("labelType");
        json.string(edge.label_type.as_str());
        json.optional_string("stroke", edge.stroke.as_deref());
        if let Some(length) = edge.length {
            json.key("length");
            json.number(f64::from(length));
        }
        if let Some(style) = &edge.style {
            json.key("style");
            json.strings(style);
        }
        json.key("classes");
        json.strings(&edge.classes);
        json.optional_string("id", edge.id.as_deref());
        json.key("isUserDefinedId");
        json.raw(if edge.is_user_defined_id {
            "true"
        } else {
            "false"
        });
        json.optional_value("interpolate", edge.interpolate.as_ref());
        json.optional_value("animate", edge.animate.as_ref());
        json.optional_value("animation", edge.animation.as_ref());
        json.close('}');
    }
    json.close(']');

    json.key("subgraphs");
    json.open('[');
    for subgraph in &model.subgraphs {
        json.item();
        json.open('{');
        json.key("id");
        json.string(&subgraph.id);
        json.key("nodes");
        json.strings(&subgraph.nodes);
        json.key("title");
        json.string(&subgraph.title);
        json.key("classes");
        json.strings(&subgraph.classes);
        json.optional_string("dir", subgraph.dir.as_deref());
        json.key("labelType");
        json.string(subgraph.label_type.as_str());
        if let Some(metadata) = &subgraph.metadata {
            json.key("metadata");
            json.object(metadata, 0);
        }
        json.close('}');
    }
    json.close(']');

    json.key("classes");
    json.open('{');
    for class in json::property_order(&model.classes, |class| &class.id) {
        json.key(&class.id);
        json.open('{');
        json.key("id");
        json.string(&class.id);
        json.key("styles");
        json.strings(&class.styles);
        json.key("textStyles");
        json.strings(&class.text_styles);
        json.close('}');
    }
    json.close('}');

    json.key("tooltips");
    json.open('{');
    for (id, tooltip) in json::property_order(&model.tooltips, |(id, _)| id) {
        json.key(id);
        json.string(tooltip);
    }
    json.close('}');

    if let Some(style) = &model.default_edge_style {
        json.key("defaultEdgeStyle");
        json.strings(style);
    }
    json.optional_string("defaultInterpolate", model.default_interpolate.as_deref());
    json.optional_string("title", model.title.as_deref());
    json.optional_string("accTitle", model.acc_title.as_deref());
    json.optional_string("accDescr", model.acc_descr.as_deref());
    json.close('}');
    json.out
}

/// A JSON writer that matches `JSON.stringify` output.
#[cfg(feature = "model")]
mod json {
    use crate::yaml::Value;
    use crate::yaml::push_number;

    /// How deep nested YAML values are written. Deeper values become `null`.
    const MAX_DEPTH: usize = 256;

    /// Output and comma state.
    #[derive(Default)]
    pub(super) struct Writer {
        /// The JSON text.
        pub(super) out: String,
        /// Whether the next member or item is the first of its container.
        first: bool,
    }

    impl Writer {
        pub(super) fn open(&mut self, bracket: char) {
            self.out.push(bracket);
            self.first = true;
        }

        pub(super) fn close(&mut self, bracket: char) {
            self.out.push(bracket);
            self.first = false;
        }

        /// Starts an array item.
        pub(super) fn item(&mut self) {
            if !self.first {
                self.out.push(',');
            }
            self.first = false;
        }

        /// Starts an object member.
        pub(super) fn key(&mut self, key: &str) {
            self.item();
            self.string(key);
            self.out.push(':');
        }

        pub(super) fn raw(&mut self, text: &str) {
            self.out.push_str(text);
        }

        /// Writes a string as `JSON.stringify` quotes it.
        pub(super) fn string(&mut self, text: &str) {
            self.out.push('"');
            for c in text.chars() {
                match c {
                    '"' => self.out.push_str("\\\""),
                    '\\' => self.out.push_str("\\\\"),
                    '\u{8}' => self.out.push_str("\\b"),
                    '\u{c}' => self.out.push_str("\\f"),
                    '\n' => self.out.push_str("\\n"),
                    '\r' => self.out.push_str("\\r"),
                    '\t' => self.out.push_str("\\t"),
                    c if u32::from(c) < 0x20 => {
                        let code = u32::from(c);
                        self.out.push_str("\\u00");
                        for nibble in [code >> 4, code & 15] {
                            self.out.push(char::from_digit(nibble, 16).unwrap_or('0'));
                        }
                    }
                    c => self.out.push(c),
                }
            }
            self.out.push('"');
        }

        pub(super) fn number(&mut self, value: f64) {
            if value.is_finite() {
                push_number(&mut self.out, value);
            } else {
                self.out.push_str("null");
            }
        }

        pub(super) fn strings(&mut self, items: &[String]) {
            self.open('[');
            for item in items {
                self.item();
                self.string(item);
            }
            self.close(']');
        }

        pub(super) fn optional_string(&mut self, key: &str, value: Option<&str>) {
            if let Some(value) = value {
                self.key(key);
                self.string(value);
            }
        }

        pub(super) fn optional_value(&mut self, key: &str, value: Option<&Value>) {
            if let Some(value) = value {
                self.key(key);
                self.value(value, 0);
            }
        }

        pub(super) fn object(&mut self, entries: &[(String, Value)], depth: usize) {
            self.open('{');
            for (key, value) in property_order(entries, |(key, _)| key) {
                self.key(key);
                self.value(value, depth + 1);
            }
            self.close('}');
        }

        pub(super) fn value(&mut self, value: &Value, depth: usize) {
            if depth >= MAX_DEPTH {
                self.out.push_str("null");
                return;
            }
            match value {
                Value::Array(items) => {
                    self.open('[');
                    for item in items {
                        self.item();
                        self.value(item, depth + 1);
                    }
                    self.close(']');
                }
                Value::Bool(value) => self.raw(if *value { "true" } else { "false" }),
                Value::Null => self.raw("null"),
                Value::Number(value) => self.number(*value),
                Value::Object(entries) => self.object(entries, depth),
                Value::String(text) => self.string(text),
            }
        }
    }

    /// The array index a property key names, as JavaScript orders own properties.
    fn array_index(key: &str) -> Option<u32> {
        if key != "0" && (key.is_empty() || key.starts_with('0')) {
            return None;
        }
        if !key.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        key.parse::<u32>().ok().filter(|&index| index != u32::MAX)
    }

    /// Entries in JavaScript property order: array-index keys ascending, then the rest in order.
    pub(super) fn property_order<E>(entries: &[E], key: impl Fn(&E) -> &String) -> Vec<&E> {
        let mut indexed: Vec<(u32, &E)> = entries
            .iter()
            .filter_map(|entry| array_index(key(entry)).map(|index| (index, entry)))
            .collect();
        indexed.sort_by_key(|&(index, _)| index);

        let mut ordered: Vec<&E> = indexed.into_iter().map(|(_, entry)| entry).collect();
        ordered.extend(
            entries
                .iter()
                .filter(|entry| array_index(key(entry)).is_none()),
        );
        ordered
    }
}

#[cfg(all(test, feature = "model"))]
mod tests {
    use std::fmt::Write;

    use super::{parse, to_json};
    use crate::yaml::Value;

    /// The parity fixture: Pele's parser input and its normalized model JSON or error, generated
    /// by running Pele's TypeScript.
    const FIXTURE: &str = include_str!("../../tests/fixtures/flowchart-parse.json");

    #[derive(Debug)]
    enum Json {
        Arr(Vec<Json>),
        Bool(bool),
        Null,
        Num(f64),
        Obj(Vec<(String, Json)>),
        Str(String),
    }

    impl Json {
        fn get(&self, key: &str) -> Option<&Json> {
            let Json::Obj(entries) = self else {
                return None;
            };
            entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        }

        fn str(&self, key: &str) -> Option<&str> {
            match self.get(key) {
                Some(Json::Str(s)) => Some(s),
                _ => None,
            }
        }

        fn num(&self, key: &str) -> Option<f64> {
            match self.get(key) {
                Some(Json::Num(n)) => Some(*n),
                _ => None,
            }
        }
    }

    /// A model's JSON, or an error's code, message, line, and column.
    type Outcome = (Option<String>, Option<(String, String, u32, u32)>);

    struct Reader<'a> {
        bytes: &'a [u8],
        at: usize,
    }

    impl Reader<'_> {
        fn ws(&mut self) {
            while self.bytes.get(self.at).is_some_and(u8::is_ascii_whitespace) {
                self.at += 1;
            }
        }

        fn value(&mut self) -> Json {
            self.ws();
            let c = self.bytes[self.at];
            match c {
                b'{' => {
                    self.at += 1;
                    let mut entries = Vec::new();
                    loop {
                        self.ws();
                        if self.bytes[self.at] == b'}' {
                            self.at += 1;
                            return Json::Obj(entries);
                        }
                        if self.bytes[self.at] == b',' {
                            self.at += 1;
                            continue;
                        }
                        let Json::Str(key) = self.value() else {
                            panic!("key")
                        };
                        self.ws();
                        assert_eq!(self.bytes[self.at], b':');
                        self.at += 1;
                        entries.push((key, self.value()));
                    }
                }
                b'[' => {
                    self.at += 1;
                    let mut items = Vec::new();
                    loop {
                        self.ws();
                        if self.bytes[self.at] == b']' {
                            self.at += 1;
                            return Json::Arr(items);
                        }
                        if self.bytes[self.at] == b',' {
                            self.at += 1;
                            continue;
                        }
                        items.push(self.value());
                    }
                }
                b'"' => {
                    self.at += 1;
                    let mut units: Vec<u16> = Vec::new();
                    loop {
                        let c = self.bytes[self.at];
                        self.at += 1;
                        match c {
                            b'"' => break,
                            b'\\' => {
                                let e = self.bytes[self.at];
                                self.at += 1;
                                let unit = match e {
                                    b'n' => 10,
                                    b'r' => 13,
                                    b't' => 9,
                                    b'b' => 8,
                                    b'f' => 12,
                                    b'u' => {
                                        let hex =
                                            std::str::from_utf8(&self.bytes[self.at..self.at + 4])
                                                .unwrap();
                                        self.at += 4;
                                        u16::from_str_radix(hex, 16).unwrap()
                                    }
                                    other => u16::from(other),
                                };
                                units.push(unit);
                            }
                            _ => {
                                let start = self.at - 1;
                                let len = match c {
                                    0..0x80 => 1,
                                    0xc0..0xe0 => 2,
                                    0xe0..0xf0 => 3,
                                    _ => 4,
                                };
                                let s =
                                    std::str::from_utf8(&self.bytes[start..start + len]).unwrap();
                                units.extend(s.encode_utf16());
                                self.at = start + len;
                            }
                        }
                    }
                    Json::Str(String::from_utf16(&units).unwrap())
                }
                b't' => {
                    self.at += 4;
                    Json::Bool(true)
                }
                b'f' => {
                    self.at += 5;
                    Json::Bool(false)
                }
                b'n' => {
                    self.at += 4;
                    Json::Null
                }
                _ => {
                    let start = self.at;
                    while self
                        .bytes
                        .get(self.at)
                        .is_some_and(|b| b"+-.eE0123456789".contains(b))
                    {
                        self.at += 1;
                    }
                    let text = std::str::from_utf8(&self.bytes[start..self.at]).unwrap();
                    Json::Num(text.parse().unwrap())
                }
            }
        }
    }

    fn actual(case: &Json) -> Outcome {
        let input = case.str("input").unwrap();
        let inherit = matches!(case.get("inheritDir"), Some(Json::Bool(true)));
        let config = Value::Object(vec![(
            "flowchart".to_owned(),
            Value::Object(vec![("inheritDir".to_owned(), Value::Bool(inherit))]),
        )]);
        #[expect(
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "test data"
        )]
        let max_edges = case.num("maxEdges").map(|n| n as usize);
        match parse(input, None, &config, case.str("title"), max_edges) {
            Ok(model) => {
                for subgraph in &model.subgraphs {
                    assert_eq!(model.subgraph(&subgraph.id), Some(subgraph));
                }
                (Some(to_json(&model)), None)
            }
            Err(error) => (
                None,
                Some((
                    error.code.as_str().to_owned(),
                    error.message,
                    error.line,
                    error.column,
                )),
            ),
        }
    }

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "test data"
    )]
    fn expected(case: &Json) -> Outcome {
        if let Some(model) = case.str("model") {
            return (Some(model.to_owned()), None);
        }
        let error = case.get("error").unwrap();
        (
            None,
            Some((
                error.str("code").unwrap().to_owned(),
                error.str("message").unwrap().to_owned(),
                error.num("line").unwrap() as u32,
                error.num("column").unwrap() as u32,
            )),
        )
    }

    #[test]
    fn matches_pele() {
        let Json::Arr(cases) = (Reader {
            bytes: FIXTURE.as_bytes(),
            at: 0,
        })
        .value() else {
            panic!("fixture is not an array");
        };

        let mut differing = Vec::new();
        for case in &cases {
            if actual(case) != expected(case) {
                differing.push(case.str("origin").unwrap_or("?").to_owned());
            }
        }
        let mut report = String::new();
        for origin in differing.iter().take(20) {
            let case = cases
                .iter()
                .find(|case| case.str("origin") == Some(origin))
                .unwrap();
            write!(
                report,
                "\n{origin}\n  input: {:?}\n  pele:  {:?}\n  ondine: {:?}",
                case.str("input").unwrap(),
                expected(case),
                actual(case)
            )
            .unwrap();
        }
        assert!(
            differing.is_empty(),
            "{} of {} cases differ from Pele:{report}",
            differing.len(),
            cases.len()
        );
    }

    #[test]
    fn canonical_shapes() {
        assert_eq!(super::shapes::canonical_shape(None), "rect");
        assert_eq!(super::shapes::canonical_shape(Some("round")), "rounded");
        assert_eq!(super::shapes::canonical_shape(Some("database")), "cyl");
        assert_eq!(super::shapes::canonical_shape(Some("nope")), "rect");
    }
}
