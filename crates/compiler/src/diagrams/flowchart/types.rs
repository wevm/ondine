//! Records of the flowchart model. Port of Pele's `src/diagrams/flowchart/types.ts`.

use crate::yaml::Value;

/// How a label is written: plain text, a quoted string, or Markdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LabelType {
    /// Pele's `'markdown'`.
    Markdown,
    /// Pele's `'string'`.
    String,
    /// Pele's `'text'`.
    Text,
}

impl LabelType {
    /// The name Pele stores.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Markdown => "markdown",
            Self::String => "string",
            Self::Text => "text",
        }
    }
}

/// A label as the parser reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FlowText {
    /// The label text.
    pub(crate) text: String,
    /// How the label is written.
    pub(crate) r#type: LabelType,
}

/// A node. Fields Pele leaves `undefined` are `None`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FlowNode {
    /// `h` from shape data, as `Number()` reads it.
    pub(crate) asset_height: Option<f64>,
    /// `w` from shape data, as `Number()` reads it.
    pub(crate) asset_width: Option<f64>,
    /// Class names, in application order.
    pub(crate) classes: Vec<String>,
    /// `constraint` from shape data.
    pub(crate) constraint: Option<String>,
    /// A direction. Mermaid's parser never sets it, but the model keeps its slot.
    pub(crate) dir: Option<String>,
    /// `form` from shape data.
    pub(crate) form: Option<String>,
    /// `icon` from shape data.
    pub(crate) icon: Option<String>,
    /// The node id.
    pub(crate) id: String,
    /// `img` from shape data.
    pub(crate) img: Option<String>,
    /// How the label is written.
    pub(crate) label_type: LabelType,
    /// The authored lines of the statements that name the node, ascending. Empty unless the host
    /// asks for source lines. Pele has no such field.
    pub(crate) lines: Vec<u32>,
    /// The sanitized click target.
    pub(crate) link: Option<String>,
    /// The click target window.
    pub(crate) link_target: Option<String>,
    /// `pos` from shape data.
    pub(crate) pos: Option<String>,
    /// `[|field:value|...]` properties, in insertion order. Set by the first `addVertex`.
    pub(crate) props: Option<Vec<(String, String)>>,
    /// Inline styles, in application order.
    pub(crate) styles: Vec<String>,
    /// The label.
    pub(crate) text: Option<String>,
    /// The shape: a bracket syntax name or a shape data name.
    pub(crate) r#type: Option<String>,
}

/// An edge.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FlowEdge {
    /// `animate` from shape data, as YAML read it.
    pub(crate) animate: Option<Value>,
    /// `animation` from shape data, as YAML read it.
    pub(crate) animation: Option<Value>,
    /// Class names, in application order.
    pub(crate) classes: Vec<String>,
    /// The target node id.
    pub(crate) end: String,
    /// The user-defined or generated id.
    pub(crate) id: Option<String>,
    /// The curve: from `linkStyle ... interpolate`, or any YAML value from shape data `curve`.
    pub(crate) interpolate: Option<Value>,
    /// Whether `id` was written in the source.
    pub(crate) is_user_defined_id: bool,
    /// How the label is written.
    pub(crate) label_type: LabelType,
    /// The extra rank span, capped at 10.
    pub(crate) length: Option<i32>,
    /// The authored lines of the statements that name the edge, ascending: its arrow, and any
    /// `linkStyle`, `class`, or `@{ ... }` for it. Empty unless the host asks for source lines.
    /// Pele has no such field.
    pub(crate) lines: Vec<u32>,
    /// The source node id.
    pub(crate) start: String,
    /// `normal`, `thick`, `dotted`, `invisible`, or `INVALID`.
    pub(crate) stroke: Option<String>,
    /// Styles from `linkStyle`.
    pub(crate) style: Option<Vec<String>>,
    /// The label.
    pub(crate) text: String,
    /// The arrowhead kind, such as `arrow_point`.
    pub(crate) r#type: Option<String>,
}

/// A `classDef`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FlowClassDef {
    /// The class name.
    pub(crate) id: String,
    /// Declarations.
    pub(crate) styles: Vec<String>,
    /// Declarations that mention `color`, with `fill` renamed `bgFill`.
    pub(crate) text_styles: Vec<String>,
}

/// A subgraph.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FlowSubgraph {
    /// Class names, in application order.
    pub(crate) classes: Vec<String>,
    /// The layout direction.
    pub(crate) dir: Option<String>,
    /// The subgraph id.
    pub(crate) id: String,
    /// How the title is written.
    pub(crate) label_type: LabelType,
    /// The authored lines of its `subgraph` headers and of the statements that name it,
    /// ascending. Empty unless the host asks for source lines. Pele has no such field.
    pub(crate) lines: Vec<u32>,
    /// Shape data merged across statements, in insertion order.
    pub(crate) metadata: Option<Vec<(String, Value)>>,
    /// Member node and subgraph ids.
    pub(crate) nodes: Vec<String>,
    /// The title.
    pub(crate) title: String,
}

/// An arrow as `destructLink` reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct LinkInfo {
    /// The user-defined id, with its `@`.
    pub(crate) id: Option<String>,
    /// The extra rank span.
    pub(crate) length: Option<i32>,
    /// The stroke.
    pub(crate) stroke: &'static str,
    /// The label.
    pub(crate) text: Option<FlowText>,
    /// The arrowhead kind.
    pub(crate) r#type: &'static str,
}

/// An item a subgraph body collects: separator text, the node ids of a statement, or a
/// `direction` statement.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum DocItem {
    /// A `direction` statement.
    Dir(&'static str),
    /// Node ids, most recent first.
    Ids(Vec<String>),
    /// Separator or accessibility text.
    Str(String),
}
