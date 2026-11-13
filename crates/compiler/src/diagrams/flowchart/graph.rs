//! Resolves a parsed flowchart into the nodes and edges that are drawn: group membership,
//! collapsed subgraphs, class styles, and edge defaults. Port of Pele's
//! `src/diagrams/flowchart/graph.ts`.

use std::collections::{BTreeMap, BTreeSet};

use super::db::FlowDb;
use super::types::{FlowNode, LabelType};
use crate::yaml::Value;

/// A node or group to draw. Mirrors Pele's `GraphNode`; fields only variants read are omitted.
#[derive(Clone, Debug)]
pub(crate) struct GraphNode {
    /// Height of an image or icon, when set.
    pub(crate) asset_height: Option<f64>,
    /// Width of an image or icon, when set.
    pub(crate) asset_width: Option<f64>,
    /// Space-separated class names.
    pub(crate) css_classes: String,
    /// Declarations from the node's classes.
    pub(crate) css_compiled_styles: Vec<String>,
    /// Declarations written on the node itself.
    pub(crate) css_styles: Vec<String>,
    /// Whether an image keeps its aspect ratio (`on`).
    pub(crate) constraint: Option<String>,
    /// The direction inside a group.
    pub(crate) dir: Option<String>,
    /// Icon name.
    pub(crate) icon: Option<String>,
    /// Node id.
    pub(crate) id: String,
    /// Image address.
    pub(crate) img: Option<String>,
    /// Whether the node is drawn as a group.
    pub(crate) is_group: bool,
    /// Label text.
    pub(crate) label: Option<String>,
    /// How the label is written.
    pub(crate) label_type: LabelType,
    /// Authored source lines, ascending, for `data-lines`.
    pub(crate) lines: Vec<u32>,
    /// Link address.
    pub(crate) link: Option<String>,
    /// Link target.
    pub(crate) link_target: Option<String>,
    /// The enclosing group's id.
    pub(crate) parent_id: Option<String>,
    /// Label position beside an image or icon (`t` or `b`).
    pub(crate) pos: Option<String>,
    /// Shape name.
    pub(crate) shape: String,
    /// Tooltip text.
    pub(crate) tooltip: Option<String>,
}

/// An edge to draw. Mirrors Pele's `GraphEdge`; fields no renderer reads are omitted.
#[derive(Clone, Debug)]
pub(crate) struct GraphEdge {
    /// Marker at the head.
    pub(crate) arrow_type_end: &'static str,
    /// Marker at the tail.
    pub(crate) arrow_type_start: &'static str,
    /// Space-separated class names.
    pub(crate) classes: &'static str,
    /// Declarations from the edge's classes.
    pub(crate) css_compiled_styles: Vec<String>,
    /// Curve name, from the edge, the diagram default, or the config.
    pub(crate) curve: Option<String>,
    /// Head node id.
    pub(crate) end: String,
    /// Edge id.
    pub(crate) id: String,
    /// Label text.
    pub(crate) label: String,
    /// How the label is written.
    pub(crate) label_type: LabelType,
    /// Authored source lines, ascending, for `data-lines`.
    pub(crate) lines: Vec<u32>,
    /// Least rank span.
    pub(crate) minlen: Option<i32>,
    /// Tail node id.
    pub(crate) start: String,
    /// Declarations from `linkStyle`, after the defaults.
    pub(crate) style: Vec<String>,
    /// Stroke kind: `normal`, `thick`, `dotted`, or `invisible`.
    pub(crate) thickness: Option<String>,
}

/// The drawn nodes and edges.
#[derive(Clone, Debug, Default)]
pub(crate) struct FlowGraph {
    /// Edges in declaration order.
    pub(crate) edges: Vec<GraphEdge>,
    /// Groups (innermost declared last first), then vertices in declaration order.
    pub(crate) nodes: Vec<GraphNode>,
}

fn shape_of(vertex: &FlowNode) -> String {
    if vertex.img.as_deref().is_some_and(|img| !img.is_empty()) {
        return String::from("imageSquare");
    }
    if vertex.icon.as_deref().is_some_and(|icon| !icon.is_empty()) {
        return String::from(match vertex.form.as_deref() {
            Some("circle") => "iconCircle",
            Some("square") => "iconSquare",
            Some("rounded") => "iconRounded",
            _ => "icon",
        });
    }

    match vertex.r#type.as_deref() {
        None | Some("square") => String::from("squareRect"),
        Some("round") => String::from("roundedRect"),
        Some(other) => String::from(other),
    }
}

fn arrows(r#type: Option<&str>) -> (&'static str, &'static str) {
    match r#type {
        Some("arrow_circle") => ("none", "arrow_circle"),
        Some("arrow_cross") => ("none", "arrow_cross"),
        Some("double_arrow_point") => ("arrow_point", "arrow_point"),
        Some("double_arrow_circle") => ("arrow_circle", "arrow_circle"),
        Some("double_arrow_cross") => ("arrow_cross", "arrow_cross"),
        _ => ("none", "arrow_point"),
    }
}

fn is_collapsed(metadata: Option<&Vec<(String, Value)>>) -> bool {
    metadata.is_some_and(|entries| {
        entries
            .iter()
            .any(|(key, value)| key == "view" && *value == Value::String(String::from("collapsed")))
    })
}

/// Builds the drawn graph. `default_curve` comes from the `flowchart.curve` config.
pub(crate) fn build(model: &FlowDb, default_curve: Option<&str>) -> FlowGraph {
    let subgraphs = &model.subgraphs;

    // Pele concatenates and trims after each class, so earlier declarations are trimmed again.
    let compiled = |names: &mut dyn Iterator<Item = &str>| -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for name in names {
            if let Some(def) = model.classes.iter().find(|def| def.id == name) {
                out.extend(def.styles.iter().cloned());
                out.extend(def.text_styles.iter().cloned());
                for entry in &mut out {
                    let trimmed = super::db::js_trim(entry);
                    if trimmed.len() != entry.len() {
                        *entry = String::from(trimmed);
                    }
                }
            }
        }
        out
    };

    // Each subgraph declared inside another, by child id.
    let mut subgraph_parent: BTreeMap<&str, &str> = BTreeMap::new();
    for sg in subgraphs {
        for child in &sg.nodes {
            if model.subgraph(child).is_some() {
                subgraph_parent.insert(child, &sg.id);
            }
        }
    }

    // A subgraph's color index follows declaration order through the nesting. Ondine's
    // monochrome theme does not read it, so only the traversal Pele performs is skipped.

    let any_collapsed = subgraphs
        .iter()
        .any(|sg| is_collapsed(sg.metadata.as_ref()));
    let mut outermost: BTreeMap<String, Option<String>> = BTreeMap::new();
    let mut hidden: BTreeSet<String> = BTreeSet::new();
    let mut collapsed_into: BTreeMap<String, String> = BTreeMap::new();

    if any_collapsed {
        for sg in subgraphs {
            let Some(ancestor) =
                outermost_collapsed(model, &subgraph_parent, &mut outermost, &sg.id)
            else {
                continue;
            };

            if sg.id != ancestor {
                hidden.insert(sg.id.clone());
                collapsed_into.insert(sg.id.clone(), ancestor.clone());
            }
            for child in &sg.nodes {
                if *child == ancestor {
                    continue;
                }
                hidden.insert(child.clone());
                collapsed_into.insert(child.clone(), ancestor.clone());
            }
        }
    }
    let is_hidden = |id: &str| hidden.contains(id);

    let mut parent_of: BTreeMap<&str, &str> = BTreeMap::new();
    let mut groups: BTreeSet<&str> = BTreeSet::new();
    for sg in subgraphs.iter().rev() {
        if is_hidden(&sg.id) {
            continue;
        }
        if !sg.nodes.is_empty() {
            groups.insert(&sg.id);
        }
        for id in &sg.nodes {
            parent_of.insert(id, &sg.id);
        }
    }
    let tooltips: BTreeMap<&str, &str> = model
        .tooltips
        .iter()
        .map(|(id, text)| (id.as_str(), text.as_str()))
        .collect();

    let mut nodes: Vec<GraphNode> = Vec::new();
    let mut by_id: BTreeMap<&str, usize> = BTreeMap::new();

    for sg in subgraphs.iter().rev() {
        if is_hidden(&sg.id) {
            continue;
        }
        let collapsed = is_collapsed(sg.metadata.as_ref());
        let node = GraphNode {
            asset_height: None,
            asset_width: None,
            css_classes: sg.classes.join(" "),
            css_compiled_styles: compiled(&mut sg.classes.iter().map(String::as_str)),
            css_styles: Vec::new(),
            constraint: None,
            dir: sg.dir.clone(),
            icon: None,
            id: sg.id.clone(),
            img: None,
            is_group: !collapsed,
            label: Some(sg.title.clone()),
            label_type: sg.label_type,
            lines: sg.lines.clone(),
            link: None,
            link_target: None,
            parent_id: parent_of.get(sg.id.as_str()).map(|id| String::from(*id)),
            pos: None,
            shape: String::from(if collapsed { "collapsedGroup" } else { "rect" }),
            tooltip: None,
        };
        by_id.insert(&sg.id, nodes.len());
        nodes.push(node);
    }

    for vertex in &model.nodes {
        if is_hidden(&vertex.id) {
            continue;
        }

        let existing = by_id.get(vertex.id.as_str()).copied();
        if let Some(node) = existing.and_then(|at| nodes.get_mut(at)) {
            node.css_styles.clone_from(&vertex.styles);
            node.css_compiled_styles = compiled(&mut vertex.classes.iter().map(String::as_str));
            node.css_classes = vertex.classes.join(" ");
            // Statements that name a subgraph as a node also name the group drawn for it.
            for &line in &vertex.lines {
                crate::lines::mark(&mut node.lines, line);
            }
            continue;
        }

        let is_group = groups.contains(vertex.id.as_str());
        let mut css_classes = String::from("default ");
        css_classes.push_str(&vertex.classes.join(" "));
        let node = GraphNode {
            asset_height: vertex.asset_height,
            asset_width: vertex.asset_width,
            css_classes,
            css_compiled_styles: compiled(
                &mut ["default", "node"]
                    .into_iter()
                    .chain(vertex.classes.iter().map(String::as_str)),
            ),
            css_styles: vertex.styles.clone(),
            constraint: vertex.constraint.clone(),
            dir: vertex.dir.clone(),
            icon: vertex.icon.clone(),
            id: vertex.id.clone(),
            img: vertex.img.clone(),
            is_group,
            label: vertex.text.clone(),
            label_type: vertex.label_type,
            lines: vertex.lines.clone(),
            link: vertex.link.clone(),
            link_target: vertex.link_target.clone(),
            parent_id: parent_of
                .get(vertex.id.as_str())
                .map(|id| String::from(*id)),
            pos: vertex.pos.clone(),
            shape: if is_group {
                String::from("rect")
            } else {
                shape_of(vertex)
            },
            tooltip: tooltips
                .get(vertex.id.as_str())
                .map(|text| String::from(*text)),
        };
        by_id.insert(&vertex.id, nodes.len());
        nodes.push(node);
    }

    let mut edges = Vec::with_capacity(model.edges.len());
    for (index, raw) in model.edges.iter().enumerate() {
        let start_into = collapsed_into.get(&raw.start).map(String::as_str);
        let end_into = collapsed_into.get(&raw.end).map(String::as_str);
        let start = start_into.unwrap_or(&raw.start);
        let end = end_into.unwrap_or(&raw.end);
        if start == end && (start_into.is_some() || end_into.is_some()) {
            continue;
        }

        let (arrow_start, arrow_end) = arrows(raw.r#type.as_deref());
        let mut style = model.default_edge_style.clone().unwrap_or_default();
        if let Some(own) = &raw.style {
            style.extend(own.iter().cloned());
        }
        let invisible = raw.stroke.as_deref() == Some("invisible");
        let plain = invisible || raw.r#type.as_deref() == Some("arrow_open");

        let id = match raw.id.as_deref() {
            Some(id) if !id.is_empty() => String::from(id),
            _ => {
                let mut id = String::from("L_");
                id.push_str(start);
                id.push('_');
                id.push_str(end);
                id.push('_');
                crate::error::push_u32(&mut id, u32::try_from(index).unwrap_or(u32::MAX));
                id
            }
        };

        // `raw.interpolate || model.defaultInterpolate || defaultCurve`, as JavaScript reads it.
        let curve = match raw.interpolate.as_ref().filter(|value| value.truthy()) {
            Some(value) => {
                let mut text = String::new();
                value.push_js_string(&mut text);
                Some(text)
            }
            None => model
                .default_interpolate
                .clone()
                .filter(|text| !text.is_empty())
                .or_else(|| {
                    default_curve
                        .filter(|text| !text.is_empty())
                        .map(String::from)
                }),
        };

        edges.push(GraphEdge {
            arrow_type_end: if plain { "none" } else { arrow_end },
            arrow_type_start: if plain { "none" } else { arrow_start },
            classes: if invisible {
                ""
            } else {
                "edge-thickness-normal edge-pattern-solid flowchart-link"
            },
            css_compiled_styles: compiled(&mut raw.classes.iter().map(String::as_str)),
            curve,
            end: String::from(end),
            id,
            label: raw.text.clone(),
            label_type: raw.label_type,
            lines: raw.lines.clone(),
            minlen: raw.length,
            start: String::from(start),
            style,
            thickness: raw.stroke.clone(),
        });
    }

    FlowGraph { edges, nodes }
}

/// The outermost collapsed subgraph around `id`, resolved once per chain of parents.
fn outermost_collapsed(
    model: &FlowDb,
    parents: &BTreeMap<&str, &str>,
    cache: &mut BTreeMap<String, Option<String>>,
    id: &str,
) -> Option<String> {
    let mut chain: Vec<&str> = Vec::new();
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut current = Some(id);
    while let Some(key) = current {
        if cache.contains_key(key) || !seen.insert(key) {
            break;
        }
        chain.push(key);
        current = parents.get(key).copied();
    }

    let mut result = current.and_then(|key| cache.get(key).cloned()).flatten();
    for key in chain.iter().rev() {
        if result.is_none() && is_collapsed(model.subgraph(key).and_then(|sg| sg.metadata.as_ref()))
        {
            result = Some(String::from(*key));
        }
        cache.insert(String::from(*key), result.clone());
    }
    result
}
