//! Agentflows: flowcharts of agent steps, tools, decisions, and references, grouped in flows,
//! with source positions for each statement and diagnostics about shapes and nesting. Port of
//! Pele's `src/diagrams/agentflow/index.ts`.
//!
//! Pele's `diagnostics.ts` only names the diagnostic vocabulary for tests, so it is not ported.

pub(crate) mod color_slots;
pub(crate) mod db;
pub(crate) mod graph;
pub(crate) mod lexer;
pub(crate) mod parser;
pub(crate) mod render;
pub(crate) mod shapes;
pub(crate) mod tokens;
pub(crate) mod types;

/// The JSON reader of the parity fixtures, loaded once per build.
#[cfg(all(test, not(feature = "sankey")))]
#[path = "../../tests/fixtures/json.rs"]
mod fixture;
#[cfg(all(test, feature = "sankey"))]
use super::sankey::fixture;

use super::{Diagram, Input, Rendered};
use crate::error::Error;
use crate::svg::root::RenderOptions;
use crate::util::url::UrlOptions;
use crate::yaml::Value;

/// The `agentflow` diagram type.
pub(crate) const DIAGRAM: Diagram = Diagram {
    keep_comments: true,
    render: render_input,
    section: None,
};

/// The parsed agentflow: Pele's `AgentflowDb`, with the graph Pele's `agentGraph` builds once per
/// model while parsing.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Model {
    /// The model.
    pub(crate) db: db::AgentflowDb,
    /// The nodes and edges to draw.
    pub(crate) graph: graph::AgentGraph,
}

fn render_input(
    input: &Input<'_>,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
) -> Result<Rendered, Error> {
    let model = parse(
        input.source,
        input.config,
        input.title,
        input.line_offset,
        Some(input.max_edges),
    )?;
    Ok(render::render(
        &model.db,
        &model.graph,
        input.config,
        options,
        url,
    ))
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

/// Parses preprocessed agentflow `source`, which keeps its `%%` comment lines, as Pele's
/// `agentflow.parse` does, and builds the graph to draw. `config` is the diagram configuration,
/// of which `direction`, `flowchart.inheritDir`, and `flowchart.curve` are read. `title` comes
/// from front matter, and `line_offset` counts the front matter lines. `max_edges` bounds the
/// edge count.
///
/// # Errors
///
/// Returns a syntax error for input Mermaid rejects and for metadata that is not YAML, a
/// semantic error for unknown shapes and out-of-range `linkStyle` indexes, and a limit error past
/// `max_edges` edges.
pub(crate) fn parse(
    source: &str,
    config: &Value,
    title: Option<&str>,
    line_offset: u32,
    max_edges: Option<usize>,
) -> Result<Model, Error> {
    // Mermaid reads these settings from the flowchart section for agentflow too.
    let flow = property(config, "flowchart");
    let inherit_dir = matches!(
        flow.and_then(|flow| property(flow, "inheritDir")),
        Some(Value::Bool(true))
    );
    let direction = match property(config, "direction") {
        Some(Value::String(direction)) => Some(direction.clone()),
        _ => None,
    };
    let mut db = db::AgentflowDb::new(db::Options {
        direction,
        inherit_dir,
        line_offset,
        max_edges,
    });
    if let Some(title) = title.filter(|title| !title.is_empty()) {
        db.title = Some(title.to_owned());
    }
    parser::parse_agentflow(source, &mut db)?;

    let curve = match flow.and_then(|flow| property(flow, "curve")) {
        Some(Value::String(curve)) => Some(curve.as_str()),
        _ => None,
    };
    let graph = graph::build_agent_graph(&mut db, render::PALETTE, curve);
    Ok(Model { db, graph })
}

/// Serializes `model` as `JSON.stringify` serializes this normalization of Pele's parsed
/// `AgentflowDb`, which the parity tests apply on Pele's side. Maps become arrays of entries in
/// insertion order, records list their fields in the order below, and `undefined` fields are
/// dropped. The graph is the one `agentGraph` built while parsing.
///
/// ```ts
/// const NODE = ['id', 'text', 'labelType', 'type', 'styles', 'classes', 'dir', 'props', 'link',
///   'linkTarget', 'metadata', 'isConnector'];
/// const EDGE = ['start', 'end', 'type', 'text', 'labelType', 'stroke', 'edgeSemantic', 'length',
///   'style', 'classes', 'id', 'isUserDefinedId', 'interpolate', 'animate', 'animation', 'metadata'];
/// const SUBGRAPH = ['id', 'nodes', 'title', 'classes', 'dir', 'labelType', 'type', 'metadata'];
/// const CLASS = ['id', 'styles', 'textStyles'];
/// const GNODE = ['id', 'label', 'labelType', 'shape', 'isGroup', 'parentId', 'cssStyles',
///   'cssCompiledStyles', 'cssClasses', 'dir', 'link', 'linkTarget', 'tooltip', 'colorIndex',
///   'kind', 'metadata'];
/// const GEDGE = ['id', 'isUserDefinedId', 'start', 'end', 'type', 'label', 'labelType',
///   'thickness', 'pattern', 'minlen', 'arrowTypeStart', 'arrowTypeEnd', 'cssCompiledStyles',
///   'style', 'classes', 'animate', 'animation', 'curve', 'metadata'];
/// const at = (p) => p && [p.startLine, p.startColumn, p.endLine, p.endColumn];
/// const { diagnostics: _, ...semantic } = db.semanticModel();
/// JSON.stringify({
///   type: db.type, direction: db.direction,
///   nodes: [...db.nodes].map(([k, v]) => [k, pick(v, NODE)]),
///   edges: db.edges.map((e) => pick(e, EDGE)),
///   subgraphs: db.subgraphs.map((s) => pick(s, SUBGRAPH)),
///   connectors: [...db.connectors.keys()],
///   classes: [...db.classes.values()].map((c) => pick(c, CLASS)),
///   tooltips: [...db.tooltips],
///   defaultEdgeStyle: db.defaultEdgeStyle, defaultInterpolate: db.defaultInterpolate,
///   mappings: db.mappings.map((m) => [m.id, m.type, ...at(m.position)]),
///   diagnostics: db.diagnostics.map((d) => ({ id: d.id, severity: d.severity,
///     message: d.message, nodeId: d.nodeId, edgeId: d.edgeId, position: at(d.position) })),
///   title: db.title, accTitle: db.accTitle, accDescr: db.accDescr,
///   graph: { nodes: graph.nodes.map((n) => pick(n, GNODE)),
///     edges: graph.edges.map((e) => pick(e, GEDGE)) },
///   semantic,
/// });
/// ```
#[cfg(feature = "model")]
pub(crate) fn to_json(model: &Model) -> String {
    let db = &model.db;
    let mut json = json::Writer::default();
    json.open('{');
    json.key("type");
    json.string("agentflow");
    json.optional_string("direction", db.direction.as_deref());

    json.key("nodes");
    json.open('[');
    for node in &db.nodes {
        json.item();
        json.open('[');
        json.item();
        json.string(&node.id);
        json.item();
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
            for (key, value) in props {
                json.key(key);
                json.string(value);
            }
            json.close('}');
        }
        json.optional_string("link", node.link.as_deref());
        json.optional_string("linkTarget", node.link_target.as_deref());
        json.optional_object("metadata", node.metadata.as_deref());
        if node.is_connector {
            json.key("isConnector");
            json.raw("true");
        }
        json.close('}');
        json.close(']');
    }
    json.close(']');

    json.key("edges");
    json.open('[');
    for edge in &db.edges {
        json.item();
        json.open('{');
        json.key("start");
        json.string(&edge.start);
        json.key("end");
        json.string(&edge.end);
        json.key("type");
        json.string(edge.r#type);
        json.key("text");
        json.string(&edge.text);
        json.key("labelType");
        json.string(edge.label_type.as_str());
        json.key("stroke");
        json.string(edge.stroke);
        json.optional_string("edgeSemantic", edge.edge_semantic);
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
        json.key("id");
        json.string(&edge.id);
        json.key("isUserDefinedId");
        json.boolean(edge.is_user_defined_id);
        json.optional_value("interpolate", edge.interpolate.as_ref());
        json.optional_value("animate", edge.animate.as_ref());
        json.optional_value("animation", edge.animation.as_ref());
        json.optional_object("metadata", edge.metadata.as_deref());
        json.close('}');
    }
    json.close(']');

    json.key("subgraphs");
    json.open('[');
    for subgraph in &db.subgraphs {
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
        json.key("type");
        json.string("flow");
        json.optional_object("metadata", subgraph.metadata.as_deref());
        json.close('}');
    }
    json.close(']');

    json.key("connectors");
    json.strings(&db.connectors);

    json.key("classes");
    json.open('[');
    for class in &db.classes {
        json.item();
        json.open('{');
        json.key("id");
        json.string(&class.id);
        json.key("styles");
        json.strings(&class.styles);
        json.key("textStyles");
        json.strings(&class.text_styles);
        json.close('}');
    }
    json.close(']');

    json.key("tooltips");
    json.open('[');
    for (id, tooltip) in &db.tooltips {
        json.item();
        json.open('[');
        json.item();
        json.string(id);
        json.item();
        json.string(tooltip);
        json.close(']');
    }
    json.close(']');

    if let Some(style) = &db.default_edge_style {
        json.key("defaultEdgeStyle");
        json.strings(style);
    }
    json.optional_string("defaultInterpolate", db.default_interpolate.as_deref());

    json.key("mappings");
    json.open('[');
    for mapping in &db.mappings {
        json.item();
        json.open('[');
        json.item();
        json.string(&mapping.id);
        json.item();
        json.string(mapping.r#type);
        json.position_items(mapping.position);
        json.close(']');
    }
    json.close(']');

    json.key("diagnostics");
    json.open('[');
    for diagnostic in &db.diagnostics {
        json.item();
        json.open('{');
        json.key("id");
        json.string(diagnostic.id);
        json.key("severity");
        json.string(diagnostic.severity);
        json.key("message");
        json.string(&diagnostic.message);
        json.optional_string("nodeId", diagnostic.node_id.as_deref());
        if let Some(position) = diagnostic.position {
            json.key("position");
            json.open('[');
            json.position_items(position);
            json.close(']');
        }
        json.close('}');
    }
    json.close(']');

    json.optional_string("title", db.title.as_deref());
    json.optional_string("accTitle", db.acc_title.as_deref());
    json.optional_string("accDescr", db.acc_descr.as_deref());

    json.key("graph");
    json.open('{');
    json.key("nodes");
    json.open('[');
    for node in &model.graph.nodes {
        json.item();
        json.open('{');
        json.key("id");
        json.string(&node.id);
        json.optional_string("label", node.label.as_deref());
        json.key("labelType");
        json.string(node.label_type.as_str());
        json.key("shape");
        json.string(&node.shape);
        json.key("isGroup");
        json.boolean(node.is_group);
        json.optional_string("parentId", node.parent_id.as_deref());
        json.key("cssStyles");
        json.strings(&node.css_styles);
        json.key("cssCompiledStyles");
        json.strings(&node.css_compiled_styles);
        json.key("cssClasses");
        json.string(&node.css_classes);
        json.optional_string("dir", node.dir.as_deref());
        json.optional_string("link", node.link.as_deref());
        json.optional_string("linkTarget", node.link_target.as_deref());
        json.optional_string("tooltip", node.tooltip.as_deref());
        if let Some(index) = node.color_index {
            json.key("colorIndex");
            json.number(f64::from(index));
        }
        json.optional_string("kind", node.kind);
        json.optional_object("metadata", node.metadata.as_deref());
        json.close('}');
    }
    json.close(']');
    json.key("edges");
    json.open('[');
    for edge in &model.graph.edges {
        json.item();
        json.open('{');
        json.key("id");
        json.string(&edge.id);
        json.key("isUserDefinedId");
        json.boolean(edge.is_user_defined_id);
        json.key("start");
        json.string(&edge.start);
        json.key("end");
        json.string(&edge.end);
        json.key("type");
        json.string(edge.r#type);
        json.key("label");
        json.string(&edge.label);
        json.key("labelType");
        json.string(edge.label_type.as_str());
        json.key("thickness");
        json.string(edge.thickness);
        json.key("pattern");
        json.string(edge.thickness);
        if let Some(minlen) = edge.minlen {
            json.key("minlen");
            json.number(f64::from(minlen));
        }
        json.key("arrowTypeStart");
        json.string(graph::GraphEdge::ARROW_TYPE_START);
        json.key("arrowTypeEnd");
        json.string(edge.arrow_type_end);
        json.key("cssCompiledStyles");
        json.strings(&edge.css_compiled_styles);
        json.key("style");
        json.strings(&edge.style);
        json.key("classes");
        json.string(graph::GraphEdge::CLASSES);
        json.optional_value("animate", edge.animate.as_ref());
        json.optional_value("animation", edge.animation.as_ref());
        json.optional_value("curve", edge.curve.as_ref());
        json.optional_object("metadata", edge.metadata.as_deref());
        json.close('}');
    }
    json.close(']');
    json.close('}');

    json.key("semantic");
    semantic::write(&mut json, db);
    json.close('}');
    json.out
}

/// Pele's `semanticModel`: the diagram with presentation-only controls left out, for tools that
/// read what it means. Ondine exposes no model API, so only the parity tests read it.
#[cfg(feature = "model")]
mod semantic {
    use super::db::{AgentflowDb, PRESENTATION_KEYS};
    use super::json::Writer;
    use super::shapes::resolve_shape_alias;
    use super::types::Metadata;

    /// Metadata without presentation keys, or `None` when nothing is left.
    fn metadata(metadata: Option<&Metadata>) -> Option<Metadata> {
        let kept: Metadata = metadata?
            .iter()
            .filter(|(key, _)| !PRESENTATION_KEYS.contains(&key.as_str()))
            .cloned()
            .collect();
        (!kept.is_empty()).then_some(kept)
    }

    /// Writes the semantic model without its diagnostics, which the model already lists.
    pub(super) fn write(json: &mut Writer, db: &AgentflowDb) {
        json.open('{');
        json.key("vertices");
        json.open('[');
        for v in db
            .nodes
            .iter()
            .filter(|v| !v.is_connector && db.subgraph(&v.id).is_none())
        {
            json.item();
            json.open('{');
            json.key("id");
            json.string(&v.id);
            json.optional_string("label", v.text.as_deref());
            json.optional_string("shape", v.r#type.as_deref().map(resolve_shape_alias));
            json.key("vertexKind");
            json.string(AgentflowDb::vertex_kind(v));
            json.optional_object("metadata", metadata(v.metadata.as_ref()).as_deref());
            json.close('}');
        }
        json.close(']');

        json.key("edges");
        json.open('[');
        for e in &db.edges {
            json.item();
            json.open('{');
            json.key("start");
            json.string(&e.start);
            json.key("end");
            json.string(&e.end);
            json.key("id");
            json.string(&e.id);
            if !e.text.is_empty() {
                json.key("label");
                json.string(&e.text);
            }
            json.key("type");
            json.string(e.r#type);
            json.key("stroke");
            json.string(e.stroke);
            json.optional_string("edgeSemantic", e.edge_semantic);
            if let Some(length) = e.length {
                json.key("length");
                json.number(f64::from(length));
            }
            json.optional_object(
                "metadata",
                e.metadata
                    .as_deref()
                    .filter(|metadata| !metadata.is_empty()),
            );
            json.close('}');
        }
        json.close(']');

        json.key("subGraphs");
        json.open('[');
        for sg in &db.subgraphs {
            json.item();
            json.open('{');
            json.key("id");
            json.string(&sg.id);
            json.key("nodes");
            json.strings(&sg.nodes);
            json.key("type");
            json.string("flow");
            json.key("title");
            json.string(&sg.title);
            json.optional_string("direction", sg.dir.as_deref());
            json.optional_object("metadata", metadata(sg.metadata.as_ref()).as_deref());
            json.close('}');
        }
        json.close(']');

        json.key("connectors");
        json.open('[');
        for v in db
            .nodes
            .iter()
            .filter(|v| v.is_connector && db.subgraph(&v.id).is_none())
        {
            json.item();
            json.open('{');
            json.key("id");
            json.string(&v.id);
            if let Some(text) = v.text.as_deref().filter(|text| *text != v.id) {
                json.key("title");
                json.string(text);
            }
            json.optional_object("metadata", metadata(v.metadata.as_ref()).as_deref());
            json.close('}');
        }
        json.close(']');
        json.optional_string("direction", db.direction.as_deref());
        json.close('}');
    }
}

/// A JSON writer that matches `JSON.stringify` output.
#[cfg(feature = "model")]
mod json {
    use super::db::{push_json, push_json_string};
    use super::types::ElementPosition;
    use crate::yaml::Value;

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
            push_json_string(&mut self.out, key);
            self.out.push(':');
        }

        pub(super) fn raw(&mut self, text: &str) {
            self.out.push_str(text);
        }

        pub(super) fn boolean(&mut self, value: bool) {
            self.raw(if value { "true" } else { "false" });
        }

        pub(super) fn string(&mut self, text: &str) {
            push_json_string(&mut self.out, text);
        }

        pub(super) fn number(&mut self, value: f64) {
            push_json(&mut self.out, &Value::Number(value));
        }

        pub(super) fn strings(&mut self, items: &[String]) {
            self.open('[');
            for item in items {
                self.item();
                self.string(item);
            }
            self.close(']');
        }

        /// Writes a position's start and end as array items.
        pub(super) fn position_items(&mut self, position: ElementPosition) {
            for value in [
                position.start_line,
                position.start_column,
                position.end_line,
                position.end_column,
            ] {
                self.item();
                self.number(f64::from(value));
            }
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
                push_json(&mut self.out, value);
            }
        }

        pub(super) fn optional_object(&mut self, key: &str, entries: Option<&[(String, Value)]>) {
            if let Some(entries) = entries {
                self.key(key);
                self.open('{');
                for (key, value) in entries {
                    self.key(key);
                    push_json(&mut self.out, value);
                }
                self.close('}');
            }
        }
    }
}

#[cfg(test)]
mod tests {
    //! Parity with Pele's `agentflow.parse` over every string literal of Pele's agentflow compat
    //! specs and upstream fixtures, the documents the specs build at runtime, Mermaid's reference
    //! inputs, hand-written cases, and fuzzed variants, as recorded by running Pele's TypeScript.

    use super::fixture::{Json, cases};
    use super::parse;
    use crate::yaml::Value;

    /// The parity fixture: Pele's parser input and options, and its model JSON or error.
    const FIXTURE: &str = include_str!("../../tests/fixtures/agentflow-parse.json");

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "test data"
    )]
    fn run(case: &Json) -> Result<super::Model, crate::error::Error> {
        let mut flow = vec![(
            "inheritDir".to_owned(),
            Value::Bool(matches!(case.get("inheritDir"), Some(Json::Bool(true)))),
        )];
        if let Some(curve) = case.str("curve") {
            flow.push(("curve".to_owned(), Value::String(curve.to_owned())));
        }
        let mut config = vec![("flowchart".to_owned(), Value::Object(flow))];
        if let Some(direction) = case.str("direction") {
            config.push(("direction".to_owned(), Value::String(direction.to_owned())));
        }
        parse(
            case.str("input").unwrap_or_default(),
            &Value::Object(config),
            case.str("title"),
            case.num("lineOffset").map_or(0, |n| n as u32),
            case.num("maxEdges").map(|n| n as usize),
        )
    }

    #[test]
    fn matches_pele() {
        let mut differing = Vec::new();
        let cases = cases(FIXTURE);
        for case in &cases {
            let actual = run(case);
            let same = match (&actual, case.get("error")) {
                (Ok(_), None) => {
                    #[cfg(feature = "model")]
                    {
                        actual.as_ref().map(super::to_json).ok().as_deref() == case.str("model")
                    }
                    #[cfg(not(feature = "model"))]
                    {
                        true
                    }
                }
                (Err(error), Some(expected)) => {
                    Some(error.code.as_str()) == expected.str("code")
                        && Some(error.message.as_str()) == expected.str("message")
                        && Some(f64::from(error.line)) == expected.num("line")
                        && Some(f64::from(error.column)) == expected.num("column")
                }
                _ => false,
            };
            if !same {
                #[cfg(feature = "model")]
                let actual = actual.as_ref().map(super::to_json);
                differing.push((
                    case.str("origin").unwrap_or("?"),
                    case.str("input").unwrap_or_default(),
                    case.str("model")
                        .map_or_else(|| format!("{:?}", case.get("error")), str::to_owned),
                    format!("{actual:?}"),
                ));
            }
        }
        assert!(
            differing.is_empty(),
            "{} of {} cases differ from Pele: {:#?}",
            differing.len(),
            cases.len(),
            differing.iter().take(5).collect::<Vec<_>>()
        );
    }
    /// Mermaid's own models for 239 inputs, captured from its browser build by Pele's
    /// `tests/support/agentflow-reference.ts`.
    #[cfg(feature = "model")]
    const REFERENCE: &str = include_str!("../../../../test/vectors/agentflow-reference.json");

    /// The member `key` of an object.
    #[cfg(feature = "model")]
    fn member<'j>(value: &'j Json, key: &str) -> Option<&'j Json> {
        value.get(key)
    }

    /// The listed members of `value`, as Pele's `pick` keeps the defined ones.
    #[cfg(feature = "model")]
    fn pick(value: &Json, keys: &[&str]) -> Json {
        Json::Obj(
            keys.iter()
                .filter_map(|&key| member(value, key).map(|v| (key.to_owned(), clone(v))))
                .collect(),
        )
    }

    #[cfg(feature = "model")]
    fn clone(value: &Json) -> Json {
        match value {
            Json::Arr(items) => Json::Arr(items.iter().map(clone).collect()),
            Json::Bool(b) => Json::Bool(*b),
            Json::Null => Json::Null,
            Json::Num(n) => Json::Num(*n),
            Json::Obj(entries) => {
                Json::Obj(entries.iter().map(|(k, v)| (k.clone(), clone(v))).collect())
            }
            Json::Str(s) => Json::Str(s.clone()),
        }
    }

    #[cfg(feature = "model")]
    fn items(value: Option<&Json>) -> &[Json] {
        match value {
            Some(Json::Arr(items)) => items,
            _ => &[],
        }
    }

    /// Pele's `comparable`: markup becomes a placeholder, keys are sorted, and Mermaid's DOM
    /// bookkeeping is dropped.
    #[cfg(feature = "model")]
    fn comparable(value: &Json) -> Json {
        match value {
            Json::Str(s) if s.contains(['<', '>', '&']) => Json::Str("(markup)".to_owned()),
            Json::Arr(items) => Json::Arr(items.iter().map(comparable).collect()),
            Json::Obj(entries) => {
                let mut out: Vec<(String, Json)> = entries
                    .iter()
                    .filter(|(k, _)| k != "domId" && k != "haveCallback")
                    .map(|(k, v)| (k.clone(), comparable(v)))
                    .collect();
                out.sort_by(|a, b| a.0.encode_utf16().cmp(b.0.encode_utf16()));
                Json::Obj(out)
            }
            other => clone(other),
        }
    }

    #[cfg(feature = "model")]
    #[expect(clippy::float_cmp, reason = "JSON numbers compare exactly")]
    fn same(a: &Json, b: &Json) -> bool {
        match (a, b) {
            (Json::Arr(x), Json::Arr(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(a, b)| same(a, b))
            }
            (Json::Bool(x), Json::Bool(y)) => x == y,
            (Json::Null, Json::Null) => true,
            (Json::Num(x), Json::Num(y)) => x == y,
            (Json::Obj(x), Json::Obj(y)) => {
                x.len() == y.len() && x.iter().zip(y).all(|(a, b)| a.0 == b.0 && same(&a.1, &b.1))
            }
            (Json::Str(x), Json::Str(y)) => x == y,
            _ => false,
        }
    }

    /// Ondine's model through the accessors Pele's reference projection reads.
    #[cfg(feature = "model")]
    fn project(model: &Json) -> Json {
        let text = |key: &str| member(model, key).map_or_else(|| Json::Str(String::new()), clone);
        let diagnostics: Vec<Json> = items(member(model, "diagnostics"))
            .iter()
            .map(|d| {
                let mut out = pick(d, &["id", "severity", "message", "nodeId"]);
                if let (Json::Obj(entries), [a, b, c, e]) = (&mut out, items(member(d, "position")))
                {
                    entries.push((
                        "position".to_owned(),
                        Json::Obj(vec![
                            ("startLine".to_owned(), clone(a)),
                            ("startColumn".to_owned(), clone(b)),
                            ("endLine".to_owned(), clone(c)),
                            ("endColumn".to_owned(), clone(e)),
                            ("startIndex".to_owned(), Json::Num(0.0)),
                            ("endIndex".to_owned(), Json::Num(0.0)),
                        ]),
                    ));
                }
                out
            })
            .collect();
        let graph = member(model, "graph");
        let nodes = items(graph.and_then(|g| member(g, "nodes")))
            .iter()
            .map(|n| {
                let mut out = pick(
                    n,
                    &[
                        "id",
                        "label",
                        "labelType",
                        "shape",
                        "isGroup",
                        "parentId",
                        "cssCompiledStyles",
                        "cssClasses",
                        "dir",
                        "link",
                        "linkTarget",
                        "tooltip",
                        "colorIndex",
                        "metadata",
                    ],
                );
                // Mermaid marks a node's kind with a class; Pele keeps the kind on the node.
                let mut classes = n.str("cssClasses").unwrap_or_default().to_owned();
                if let Some(kind) = n.str("kind") {
                    classes.push_str(" af-kind-");
                    classes.push_str(kind);
                }
                let classes = classes.split_whitespace().collect::<Vec<_>>().join(" ");
                if let Json::Obj(entries) = &mut out
                    && let Some(entry) = entries.iter_mut().find(|(k, _)| k == "cssClasses")
                {
                    entry.1 = Json::Str(classes);
                }
                out
            })
            .collect();
        let layout_edges = items(graph.and_then(|g| member(g, "edges")))
            .iter()
            .map(|e| {
                pick(
                    e,
                    &[
                        "id",
                        "start",
                        "end",
                        "type",
                        "label",
                        "labelType",
                        "thickness",
                        "minlen",
                        "arrowTypeStart",
                        "arrowTypeEnd",
                        "cssCompiledStyles",
                        "style",
                        "curve",
                        "metadata",
                        "isUserDefinedId",
                    ],
                )
            })
            .collect();
        let mappings = items(member(model, "mappings"))
            .iter()
            .map(|m| {
                let mut row: Vec<Json> = items(Some(m)).iter().map(clone).collect();
                row.push(Json::Num(0.0));
                row.push(Json::Num(0.0));
                Json::Arr(row)
            })
            .collect();
        let mut semantic = member(model, "semantic").map_or(Json::Null, clone);
        if let Json::Obj(entries) = &mut semantic {
            entries.push((
                "diagnostics".to_owned(),
                Json::Arr(diagnostics.iter().map(clone).collect()),
            ));
        }

        let mut out = vec![];
        if let Some(direction) = member(model, "direction") {
            out.push(("direction".to_owned(), clone(direction)));
        }
        out.push(("accTitle".to_owned(), text("accTitle")));
        out.push(("accDescr".to_owned(), text("accDescr")));
        out.push((
            "vertices".to_owned(),
            Json::Arr(
                items(member(model, "nodes"))
                    .iter()
                    .filter_map(|entry| items(Some(entry)).get(1))
                    .map(|v| {
                        pick(
                            v,
                            &[
                                "id",
                                "text",
                                "labelType",
                                "type",
                                "styles",
                                "classes",
                                "link",
                                "linkTarget",
                                "metadata",
                                "isConnector",
                                "props",
                            ],
                        )
                    })
                    .collect(),
            ),
        ));
        for (to, from) in [
            ("edges", "edges"),
            ("defaultStyle", "defaultEdgeStyle"),
            ("defaultInterpolate", "defaultInterpolate"),
            ("subGraphs", "subgraphs"),
            ("classes", "classes"),
        ] {
            if let Some(value) = member(model, from) {
                out.push((to.to_owned(), clone(value)));
            }
        }
        out.push(("nodes".to_owned(), Json::Arr(nodes)));
        out.push(("layoutEdges".to_owned(), Json::Arr(layout_edges)));
        out.push(("mappings".to_owned(), Json::Arr(mappings)));
        out.push(("diagnostics".to_owned(), Json::Arr(diagnostics)));
        out.push(("semantic".to_owned(), semantic));
        Json::Obj(out)
    }

    /// Ondine's projected model for `src` through the whole pipeline, or the first line of its
    /// error, as Pele's `peleReference` builds it with Mermaid's palette of 12 and `basis` curve.
    #[cfg(feature = "model")]
    fn reference_of(src: &str) -> Result<Json, String> {
        let first_line = |error: crate::error::Error| {
            error
                .message
                .split('\n')
                .next()
                .unwrap_or_default()
                .to_owned()
        };
        let pre = crate::preprocess::preprocess(src, false).map_err(first_line)?;
        let mut source = crate::preprocess::encode_entities(&pre.with_comments).into_owned();
        source.push('\n');
        let config = Value::Object(pre.config);
        let mut model = parse(
            &source,
            &config,
            pre.title.as_deref(),
            pre.line_offset,
            Some(5000),
        )
        .map_err(first_line)?;
        model.graph = super::graph::build_agent_graph(&mut model.db, 12, Some("basis"));
        let json = super::to_json(&model);
        let mut wrapped = String::from("[");
        wrapped.push_str(&json);
        wrapped.push(']');
        let parsed = cases(&wrapped).into_iter().next().unwrap_or(Json::Null);
        Ok(project(&parsed))
    }

    #[cfg(feature = "model")]
    #[test]
    fn matches_mermaid_reference() {
        let reference = cases(REFERENCE);
        let mut differing = Vec::new();
        let mut matched = 0;
        for entry in &reference {
            let src = entry.str("src").unwrap_or_default();
            let mine = reference_of(src);
            let ok = match (&mine, entry.get("model"), entry.str("error")) {
                // Pele's YAML reader has no aliases and reads a lone colon as text, so it
                // accepts these where Mermaid's js-yaml does not.
                (Ok(_), None, Some(error))
                    if error.contains("alias") || error.contains("explicit mapping pair") =>
                {
                    true
                }
                (Ok(mine), Some(theirs), _) => same(&comparable(mine), &comparable(theirs)),
                (Err(_), None, Some(_)) => true,
                _ => false,
            };
            if ok {
                matched += 1;
            } else {
                differing.push((src, format!("{:?}", mine.map(|m| comparable(&m)))));
            }
        }
        assert!(
            differing.is_empty(),
            "{matched} of {} match Mermaid; differing: {:#?}",
            reference.len(),
            differing.iter().take(3).collect::<Vec<_>>()
        );
        assert_eq!(matched, 239);
    }
}
