//! Draws a use case diagram: actors as figures, use cases as ovals or boxes, notes, JSON tables,
//! and system boundaries, joined by relationship edges. Port of Pele's
//! `src/diagrams/usecase/render.ts`.

use super::graph::{GraphEdge, GraphNode, NodeShape, build_usecase_graph};
use super::types::{ActorType, UsecaseModel};
use crate::diagrams::Rendered;
use crate::diagrams::common::fit_width::{runs_across, turn_to_fit};
use crate::layout::compound::{CEdge, CNode, Dir, cnode, compound_layout, direction, shift_layout};
use crate::layout::layered::LayeredOptions;
use crate::svg::builder::{esc, esc_text, js_round, label_svg, push_num};
use crate::svg::cluster::{Crossings, mark_crossings};
use crate::svg::edges::{EdgePath, edge_label_svg, marker, marker_trim, route_path};
use crate::svg::root::{Accessible, RenderOptions, svg_document};
use crate::svg::theme::{ROUNDED, ResolvedStyle, class_names, resolve_style};
use crate::text::label::{Label, layout_label};
use crate::text::measure::{BOLD, ITALIC};
use crate::util::chars::is_space;
use crate::yaml::{Value, push_number};

/// Room between nodes in a rank.
const NODE_SEP: f64 = 32.0;
/// Room between edges.
const EDGE_SEP: f64 = 16.0;
/// Room between ranks.
const RANK_SEP: f64 = 72.0;
/// Padding inside a boundary.
const GROUP_PAD: f64 = 20.0;
/// How far a self-relationship reaches out of its node.
const LOOP: f64 = 26.0;
/// Width of an actor's figure.
const FIGURE_W: f64 = 40.0;
/// Height of an actor's figure.
const FIGURE_H: f64 = 60.0;
/// Room between an actor's figure and its text.
const FIGURE_GAP: f64 = 6.0;
/// Padding around a package's title.
const TAB_PAD: f64 = 6.0;
/// Padding beside the text of a JSON table cell.
const CELL_PAD: f64 = 10.0;
/// Room added to each JSON table row.
const ROW_PAD: f64 = 8.0;
/// Size of a note's folded corner.
const FOLD: f64 = 9.0;
/// Length of the triangle of a generalization.
const EXTENSION: f64 = 12.0;
/// The most room between edge ends on one side of a node.
const PORT_GAP: f64 = 14.0;
/// Fill and stroke of a filled outline.
const SHAPE_ATTRS: &str = " fill=\"var(--_s)\" stroke=\"var(--_b)\"";

/// How a node is drawn.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// An actor.
    Actor,
    /// A system boundary.
    Boundary,
    /// A rectangle or a note.
    Box,
    /// An oval.
    Ellipse,
    /// A JSON table.
    Json,
}

/// A node's measured parts.
struct View {
    /// Height of a JSON table's header.
    head_h: f64,
    /// Height of what is drawn; the layout box is larger when the node carries a
    /// self-relationship.
    h: f64,
    /// Width of a JSON table's key column.
    key_w: f64,
    /// How it is drawn.
    kind: Kind,
    /// Its label.
    label: Label,
    /// A JSON table's key and value labels.
    rows: Vec<(Label, Label)>,
    /// Its stereotype.
    stereotype: Label,
    /// Its author styles.
    style: ResolvedStyle,
    /// Width of what is drawn.
    w: f64,
}

/// The member `key` of an object.
fn property<'v>(value: &'v Value, key: &str) -> Option<&'v Value> {
    match value {
        Value::Object(entries) => entries.iter().find(|(own, _)| own == key).map(|(_, v)| v),
        _ => None,
    }
}

/// A positive number option of the `usecase` section, at most 2000, or `fallback`.
fn number_option(config: &Value, key: &str, fallback: f64) -> f64 {
    match property(config, "usecase").and_then(|section| property(section, key)) {
        Some(Value::Number(value)) if *value > 0.0 => value.min(2000.0),
        _ => fallback,
    }
}

/// A plain label is shown as written: no tags, no markdown, no `\n` breaks. Its markup
/// characters are handed to the label parser as character references, which it turns back into
/// characters.
fn literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&#38;"),
            '<' => out.push_str("&#60;"),
            '>' => out.push_str("&#62;"),
            '\\' => out.push_str("&#92;"),
            c => out.push(c),
        }
    }
    out
}

/// Lays out a label, plain text shown as written.
fn text(raw: Option<&str>, markdown: bool, size: f64, wrap: f64, base: u8) -> Label {
    match raw {
        Some(raw) if !raw.is_empty() && !markdown => {
            layout_label(Some(&literal(raw)), false, size, wrap, base, None)
        }
        _ => layout_label(raw, markdown, size, wrap, base, None),
    }
}

/// Wraps text in guillemets.
fn guillemets(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    out.push('«');
    out.push_str(text);
    out.push('»');
    out
}

/// The point of a node's outline on one of its four sides (0 top, 1 right, 2 bottom, 3 left),
/// `along` away from the middle of that side. Lines meet an actor at its figure, not at its
/// label.
fn anchor(view: &View, c: &CNode, side: usize, along: f64) -> (f64, f64) {
    let sign = if side == 1 || side == 2 { 1.0 } else { -1.0 };
    let upright = side == 1 || side == 3;
    let mut reach = (if upright { view.w } else { view.h }) / 2.0;
    let mut middle = 0.0;
    if view.kind == Kind::Actor {
        if upright {
            reach = FIGURE_W / 2.0 + 3.0;
            middle = (FIGURE_H - view.h) / 2.0;
        } else {
            reach += 2.0;
        }
    } else if view.kind == Kind::Ellipse {
        let span = (if upright { view.h } else { view.w }) / 2.0;
        reach *= f64::max(1.0 - (along / span).powi(2), 0.0).sqrt();
    }
    if upright {
        (c.x + sign * reach, c.y + middle + along)
    } else {
        (c.x + along, c.y + sign * reach)
    }
}

/// How much of a side edges may spread over.
fn side_length(view: &View, side: usize) -> f64 {
    let upright = side == 1 || side == 3;
    if view.kind == Kind::Actor {
        return if upright {
            FIGURE_H * 0.5
        } else {
            FIGURE_W * 0.6
        };
    }
    let length = if upright { view.h } else { view.w };
    if view.kind == Kind::Ellipse {
        length * 0.6
    } else {
        length - 16.0
    }
}

/// One end of an edge on a side of a node.
struct End {
    /// The index of its point in the route.
    at: usize,
    /// The edge.
    edge: usize,
    /// Where the edge heads from there, along the side.
    toward: f64,
}

/// The layout brings every edge that ends on one side of a node to the middle of that side. This
/// spaces them along the side in the order they fan out, and puts each on the node's outline.
#[expect(clippy::cast_precision_loss, reason = "counts are small")]
fn attach_ends(cedges: &mut [CEdge], cnodes: &[CNode], views: &[View]) {
    // Sides in order of first use, as a `Map` iterates them.
    let mut sides: Vec<(usize, Vec<End>)> = Vec::new();
    for (e, edge) in cedges.iter().enumerate() {
        let route = &edge.route;
        if edge.src == edge.dst || route.len() < 6 {
            continue;
        }
        let last = route.len() - 3;
        for (index, at, next) in [(edge.src, 0, 3), (edge.dst, last, last - 3)] {
            let Some(c) = cnodes.get(index) else {
                continue;
            };
            if c.is_group {
                continue;
            }
            let point = |k: usize| route.get(k).copied().unwrap_or(f64::NAN);
            let dx = point(at) - c.x;
            let dy = point(at + 1) - c.y;
            let side = if (dy.abs() - c.h / 2.0).abs() < 0.5 {
                if dy < 0.0 { 0 } else { 2 }
            } else if (dx.abs() - c.w / 2.0).abs() < 0.5 {
                if dx < 0.0 { 3 } else { 1 }
            } else {
                continue;
            };
            let key = index * 4 + side;
            let end = End {
                at,
                edge: e,
                toward: point(next + (side & 1)),
            };
            match sides.iter_mut().find(|(own, _)| *own == key) {
                Some((_, list)) => list.push(end),
                None => sides.push((key, vec![end])),
            }
        }
    }
    for (key, mut ends) in sides {
        let index = key >> 2;
        let side = key & 3;
        let (Some(view), Some(c)) = (views.get(index), cnodes.get(index)) else {
            continue;
        };
        let count = ends.len() as f64;
        let step = if ends.len() > 1 {
            PORT_GAP.min(side_length(view, side) / (count - 1.0))
        } else {
            0.0
        };
        ends.sort_by(|a, b| {
            (a.toward - b.toward)
                .partial_cmp(&0.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for (i, end) in ends.iter().enumerate() {
            let (x, y) = anchor(view, c, side, (i as f64 - (count - 1.0) / 2.0) * step);
            if let Some(route) = cedges.get_mut(end.edge).map(|edge| &mut edge.route) {
                if let Some(slot) = route.get_mut(end.at) {
                    *slot = x;
                }
                if let Some(slot) = route.get_mut(end.at + 1) {
                    *slot = y;
                }
            }
        }
    }
}

/// Appends `x,y`.
fn push_point(out: &mut String, x: f64, y: f64) {
    push_num(out, x);
    out.push(',');
    push_num(out, y);
}

/// The hollow triangle of a generalization, with its tip at (x, y).
fn extension(x: f64, y: f64, dx: f64, dy: f64, color: &str) -> String {
    let bx = x - dx * EXTENSION;
    let by = y - dy * EXTENSION;
    let half = EXTENSION * 0.5;
    let mut out = String::from("<path class=\"ondine-marker\" d=\"M");
    push_point(&mut out, x, y);
    out.push('L');
    push_point(&mut out, bx - dy * half, by + dx * half);
    out.push('L');
    push_point(&mut out, bx + dy * half, by - dx * half);
    out.push_str("Z\" fill=\"var(--_bg)\" stroke=\"");
    out.push_str(color);
    out.push_str("\" stroke-linejoin=\"round\"/>");
    out
}

/// How far a marker of `type` trims the end of its edge.
fn trim(r#type: &str) -> f64 {
    if r#type == "extension" {
        EXTENSION
    } else {
        marker_trim(r#type)
    }
}

/// Draws a marker of `type` with its tip at (x, y).
fn mark(r#type: &str, x: f64, y: f64, dx: f64, dy: f64, color: &str) -> String {
    if r#type == "extension" {
        extension(x, y, dx, dy, color)
    } else {
        marker(r#type, x, y, dx, dy, color)
    }
}

/// An actor's figure, centered on the origin.
fn figure(view: &View, node: &GraphNode<'_>) -> String {
    let shape = &view.style.shape;
    let mut line = String::from(" fill=\"none\" stroke=\"var(--_b)\"");
    line.push_str(&view.style.line);
    let mut head = -21.0;
    let mut radius = 9.0;
    let mut out = String::new();
    match node.shape {
        NodeShape::Actor(ActorType::Hollow) => {
            radius = 8.0;
            out.push_str("<circle cy=\"-21\" r=\"8\" fill=\"var(--_bg)\" stroke=\"var(--_b)\"");
            out.push_str(shape);
            out.push_str("/><path d=\"M-20,-9H20V0H5.5L20,15.5L12,25.5L0,12L-12,25.5L-20,15.5L-5.5,0H-20Z\" fill=\"var(--_bg)\" stroke=\"var(--_b)\" stroke-linejoin=\"round\"");
            out.push_str(shape);
            out.push_str("/>");
        }
        NodeShape::Actor(ActorType::Awesome) => {
            out.push_str("<path d=\"M0,-30A11,11 0 1 1 0,-8A11,11 0 1 1 0,-30ZM-20,25C-20,9 -12,-3 0,-3C12,-3 20,9 20,25C20,28 18,30 15,30H-15C-18,30 -20,28 -20,25Z\" fill=\"var(--_b)\"");
            out.push_str(shape);
            out.push_str("/>");
        }
        NodeShape::Actor(ActorType::Icon) => {
            head = -14.0;
            radius = 12.0;
            out.push_str("<rect x=\"-22\" y=\"-26\" width=\"44\" height=\"44\"");
            out.push_str(ROUNDED);
            out.push_str(SHAPE_ATTRS);
            out.push_str(shape);
            out.push_str("/><svg class=\"ondine-icon\" data-icon=\"");
            out.push_str(&esc_text(node.icon.unwrap_or("")));
            out.push_str("\" x=\"-14\" y=\"-18\" width=\"28\" height=\"28\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"></svg>");
        }
        _ => {
            out.push_str("<circle cy=\"-21\" r=\"9\"");
            out.push_str(SHAPE_ATTRS);
            out.push_str(shape);
            out.push_str("/><path d=\"M0,-12V10M-14,-3H14M0,10L-12,30M0,10L12,30\"");
            out.push_str(&line);
            out.push_str("/>");
        }
    }
    if node.business {
        // The slash of a business actor: a chord across the lower right of the head.
        out.push_str("<path class=\"ondine-business\" d=\"M");
        push_point(&mut out, 0.12 * radius, head + 0.99 * radius);
        out.push('L');
        push_point(&mut out, 0.92 * radius, head - 0.39 * radius);
        out.push('"');
        out.push_str(&line);
        out.push_str("/>");
    }
    out
}

/// Draws `model` with the `usecase` section of `config`, turned to run down when it runs across
/// and does not fit the host's width.
pub(crate) fn render(model: &UsecaseModel, config: &Value, options: &RenderOptions) -> Rendered {
    if !runs_across(Some(model.direction)) {
        return draw(model, config, options, false);
    }
    turn_to_fit(options, |down| draw(model, config, options, down))
}

/// `classes` without a leading `default` and one whitespace character after it, as
/// `/^default\s?/` removes them.
fn without_default(classes: &str) -> &str {
    let Some(rest) = classes.strip_prefix("default") else {
        return classes;
    };
    match rest.chars().next() {
        Some(c) if is_space(u32::from(c)) => rest.get(c.len_utf8()..).unwrap_or(""),
        _ => rest,
    }
}

/// An edge's author classes: its classes without
/// `/^default relationship relationship-\S+\s?/`.
fn edge_classes(classes: &str) -> &str {
    let Some(rest) = classes.strip_prefix("default relationship relationship-") else {
        return classes;
    };
    let kind = rest
        .char_indices()
        .find(|(_, c)| is_space(u32::from(*c)))
        .map_or(rest.len(), |(at, _)| at);
    if kind == 0 {
        return classes;
    }
    let rest = rest.get(kind..).unwrap_or("");
    match rest.chars().next() {
        Some(c) if is_space(u32::from(c)) => rest.get(c.len_utf8()..).unwrap_or(""),
        _ => rest,
    }
}

/// Measures a node.
fn measure(node: &GraphNode<'_>, size: f64, small: f64, tiny: f64) -> View {
    let style = if node.css_styles.is_empty() {
        resolve_style(&node.css_compiled_styles)
    } else {
        let mut all = node.css_compiled_styles.clone();
        all.extend(node.css_styles.iter().cloned());
        resolve_style(&all)
    };
    let base = (if style.bold { BOLD } else { 0 }) | (if style.italic { ITALIC } else { 0 });
    let font_size = style.font_size.unwrap_or(size);
    let markdown = node.markdown;
    let stereotype = text(
        node.stereotype.map(guillemets).as_deref(),
        false,
        tiny,
        200.0,
        base,
    );
    let mut view = View {
        head_h: 0.0,
        h: 0.0,
        key_w: 0.0,
        kind: Kind::Box,
        label: stereotype.clone(),
        rows: Vec::new(),
        stereotype,
        style,
        w: 0.0,
    };
    if node.shape == NodeShape::Boundary {
        view.kind = Kind::Boundary;
        view.label = text(Some(node.label), markdown, font_size, 4000.0, base);
        return view;
    }
    if let Some(rows) = &node.json_rows {
        view.kind = Kind::Json;
        view.label = text(Some(node.label), false, font_size, 4000.0, base | BOLD);
        let mut value_w: f64 = 0.0;
        for row in rows {
            let key = text(Some(&row.key), false, small, 4000.0, base);
            let value = text(Some(&row.value), false, small, 260.0, base);
            view.key_w = view.key_w.max(key.width);
            value_w = value_w.max(value.width);
            view.h += value.height.max(js_round(small * 1.5)) + ROW_PAD;
            view.rows.push((key, value));
        }
        // The root of an empty object has a value and no key, and then there is no key column.
        if view.key_w > 0.0 {
            view.key_w += 2.0 * CELL_PAD;
        }
        view.head_h = view.label.height.max(js_round(font_size * 1.5)) + ROW_PAD;
        view.w = (view.label.width + 2.0 * CELL_PAD)
            .max(if view.rows.is_empty() {
                0.0
            } else {
                view.key_w + value_w + 2.0 * CELL_PAD
            })
            .max(72.0);
        view.h += view.head_h;
        return view;
    }
    if let NodeShape::Actor(_) = node.shape {
        view.kind = Kind::Actor;
        view.label = text(Some(node.label), markdown, font_size, 140.0, base);
        view.w = FIGURE_W.max(view.label.width).max(view.stereotype.width);
        view.h = FIGURE_H + FIGURE_GAP + view.stereotype.height + view.label.height;
        return view;
    }
    let note = node.shape == NodeShape::Note;
    view.label = text(
        Some(node.label),
        markdown,
        if note { small } else { font_size },
        180.0,
        base,
    );
    let tw = view.label.width.max(view.stereotype.width);
    let th = view.label.height + view.stereotype.height;
    match node.shape {
        NodeShape::Ellipse | NodeShape::Business => {
            view.kind = Kind::Ellipse;
            // An oval needs more room around its text than a box, and a business oval room for
            // its slash.
            view.w = (tw + 2.0 * node.padding + if node.business { 16.0 } else { 4.0 }).max(88.0);
            view.h = (th + 28.0).max(48.0);
        }
        NodeShape::Note => {
            view.w = tw + 2.0 * node.padding + 6.0;
            view.h = th + 18.0;
        }
        _ => {
            view.w = (tw + 2.0 * node.padding + 12.0).max(80.0);
            view.h = (th + 24.0).max(44.0);
        }
    }
    view
}

/// `turned` draws a diagram that runs across as one that runs down.
#[expect(clippy::too_many_lines, reason = "mirrors Pele's draw")]
fn draw(model: &UsecaseModel, config: &Value, options: &RenderOptions, turned: bool) -> Rendered {
    let graph = build_usecase_graph(model);
    let size = options.font_size.unwrap_or(16.0);
    let small = js_round(size * 0.875);
    let tiny = js_round(size * 0.8125);
    let pad = options.padding.unwrap_or(8.0);
    let root_dir = if turned {
        Dir::Tb
    } else {
        direction(Some(model.direction))
    };
    let sideways = matches!(root_dir, Dir::Lr | Dir::Rl);

    // Ids by node, the last node of an id winning, as a `Map` set in order keeps it.
    let mut index: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
    for (i, node) in graph.nodes.iter().enumerate() {
        index.insert(node.id, i);
    }

    let mut views: Vec<View> = Vec::with_capacity(graph.nodes.len());
    let mut cnodes: Vec<CNode> = Vec::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let view = measure(node, size, small, tiny);
        let mut c = if view.kind == Kind::Boundary {
            let title_h = view.label.height;
            let mut c = cnode(0.0, 0.0, -1);
            c.is_group = true;
            c.pad_x = GROUP_PAD;
            c.pad_bottom = GROUP_PAD;
            c.pad_top = if node.package {
                title_h + 2.0 * TAB_PAD + 16.0
            } else if title_h > 0.0 {
                title_h + 16.0
            } else {
                GROUP_PAD
            };
            c.min_w = view.label.width + 2.0 * GROUP_PAD;
            c
        } else {
            cnode(view.w, view.h, -1)
        };
        #[expect(clippy::cast_precision_loss, reason = "counts are small")]
        {
            c.seq = views.len() as f64;
        }
        views.push(view);
        cnodes.push(c);
    }
    for (node, c) in graph.nodes.iter().zip(cnodes.iter_mut()) {
        c.parent = node
            .parent_id
            .and_then(|parent| index.get(parent))
            .and_then(|&at| i32::try_from(at).ok())
            .unwrap_or(-1);
    }

    let mut cedges: Vec<CEdge> = Vec::new();
    let mut edge_labels: Vec<Label> = Vec::new();
    let mut drawn: Vec<&GraphEdge<'_>> = Vec::new();
    for edge in &graph.edges {
        let (Some(&src), Some(&dst)) = (index.get(edge.start), index.get(edge.end)) else {
            continue;
        };
        let semantic = matches!(edge.relationship_type, "include" | "extend");
        let label = match edge.label {
            Some((raw, markdown)) if semantic => {
                text(Some(&guillemets(raw)), markdown, small, 200.0, 0)
            }
            Some((raw, markdown)) => text(Some(raw), markdown, small, 200.0, 0),
            None => text(None, false, small, 200.0, 0),
        };
        if src == dst {
            // A self-relationship loops out across the flow, and the node's box grows to hold
            // it.
            if let Some(c) = cnodes.get_mut(src) {
                if sideways {
                    c.h += 2.0
                        * (LOOP
                            + if label.height > 0.0 {
                                label.height + 8.0
                            } else {
                                0.0
                            });
                } else {
                    c.w += 2.0
                        * (LOOP
                            + if label.width > 0.0 {
                                label.width + 12.0
                            } else {
                                0.0
                            });
                }
            }
        }
        cedges.push(CEdge {
            dst,
            label_h: if label.height > 0.0 {
                label.height + 4.0
            } else {
                0.0
            },
            label_w: if label.width > 0.0 {
                label.width + 12.0
            } else {
                0.0
            },
            label_x: 0.0,
            label_y: 0.0,
            minlen: edge.minlen,
            route: Vec::new(),
            src,
        });
        drawn.push(edge);
        edge_labels.push(label);
    }

    let layout = compound_layout(
        &mut cnodes,
        &mut cedges,
        root_dir,
        &LayeredOptions {
            apart: false,
            edge_sep: EDGE_SEP,
            head_room: 0.0,
            node_sep: number_option(config, "nodeSpacing", NODE_SEP),
            port_sep: 20.0,
            rank_sep: number_option(config, "rankSpacing", RANK_SEP),
            straight: false,
            tracks: false,
        },
    );

    let title = layout_label(model.title.as_deref(), false, size, 4000.0, BOLD, None);
    let title_height = if title.height > 0.0 {
        title.height + 12.0
    } else {
        0.0
    };
    let inner = layout.width.max(title.width);
    let ox = pad + (inner - layout.width) / 2.0;
    let oy = pad + title_height;
    shift_layout(&mut cnodes, &mut cedges, ox, oy);
    let width = (inner + 2.0 * pad).ceil();
    let height = (layout.height + title_height + 2.0 * pad).ceil();

    // Where edges cross each boundary's top border, so that a title can keep out of their way.
    let mut crossings: Crossings = Crossings::new();
    for e in &cedges {
        mark_crossings(&mut crossings, &e.route);
    }
    attach_ends(&mut cedges, &cnodes, &views);

    let mut clusters = String::new();
    let mut nodes_out = String::new();
    for ((view, c), node) in views.iter().zip(&cnodes).zip(&graph.nodes) {
        let classes = class_names(without_default(&node.css_classes));
        let id = esc_text(node.id);
        let text_attrs = &view.style.text;
        let shape = &view.style.shape;
        let w = view.w;
        let h = view.h;

        if view.kind == Kind::Boundary {
            let x = c.x - c.w / 2.0;
            let y = c.y - c.h / 2.0;
            let mut boxed =
                String::from(" fill=\"var(--_a)\" fill-opacity=\"0.5\" stroke=\"var(--_b)\"");
            boxed.push_str(shape);
            let mut title_attrs =
                String::from(" class=\"ondine-cluster-label\" fill=\"var(--_m)\"");
            title_attrs.push_str(text_attrs);
            let mut body = String::new();
            if node.package {
                let tab_w = (view.label.width + 2.0 * TAB_PAD + 8.0).min(c.w);
                let tab_h = view.label.height + 2.0 * TAB_PAD;
                body.push_str("<rect class=\"ondine-boundary-tab\" x=\"");
                push_num(&mut body, x);
                body.push_str("\" y=\"");
                push_num(&mut body, y);
                body.push_str("\" width=\"");
                push_num(&mut body, tab_w);
                body.push_str("\" height=\"");
                push_num(&mut body, tab_h);
                body.push('"');
                body.push_str(&boxed);
                body.push_str("/><rect x=\"");
                push_num(&mut body, x);
                body.push_str("\" y=\"");
                push_num(&mut body, y + tab_h);
                body.push_str("\" width=\"");
                push_num(&mut body, c.w);
                body.push_str("\" height=\"");
                push_num(&mut body, c.h - tab_h);
                body.push('"');
                body.push_str(&boxed);
                body.push_str("/>");
                body.push_str(&label_svg(
                    &view.label,
                    x + tab_w / 2.0,
                    y + tab_h / 2.0,
                    &title_attrs,
                    None,
                ));
            } else {
                body.push_str("<rect x=\"");
                push_num(&mut body, x);
                body.push_str("\" y=\"");
                push_num(&mut body, y);
                body.push_str("\" width=\"");
                push_num(&mut body, c.w);
                body.push_str("\" height=\"");
                push_num(&mut body, c.h);
                body.push('"');
                body.push_str(ROUNDED);
                body.push_str(&boxed);
                body.push_str("/>");
                body.push_str(&label_svg(
                    &view.label,
                    title_x(c, view.label.width, &crossings),
                    y + 8.0 + view.label.height / 2.0,
                    &title_attrs,
                    None,
                ));
            }
            clusters.push_str("<g class=\"ondine-cluster ondine-boundary");
            clusters.push_str(&classes);
            clusters.push_str("\" data-id=\"");
            clusters.push_str(&id);
            clusters.push_str("\">");
            clusters.push_str(&body);
            clusters.push_str("</g>");
            continue;
        }

        let mut stereotype_attrs = String::from(" class=\"ondine-stereotype\" font-size=\"");
        push_number(&mut stereotype_attrs, tiny);
        stereotype_attrs.push_str("\" fill=\"var(--_m)\"");
        let mut body = String::new();
        let kind = match view.kind {
            Kind::Actor => {
                let top = -h / 2.0;
                let stereotype_y = top + FIGURE_H + FIGURE_GAP + view.stereotype.height / 2.0;
                body.push_str("<g class=\"ondine-actor-figure\" transform=\"translate(0,");
                push_num(&mut body, top + FIGURE_H / 2.0);
                body.push_str(")\">");
                body.push_str(&figure(view, node));
                body.push_str("</g>");
                body.push_str(&label_svg(
                    &view.stereotype,
                    0.0,
                    stereotype_y,
                    &stereotype_attrs,
                    None,
                ));
                let mut attrs = String::from(" class=\"ondine-label\"");
                attrs.push_str(text_attrs);
                body.push_str(&label_svg(
                    &view.label,
                    0.0,
                    h / 2.0 - view.label.height / 2.0,
                    &attrs,
                    None,
                ));
                "ondine-actor"
            }
            Kind::Json => {
                let x0 = -w / 2.0;
                let mut y = -h / 2.0 + view.head_h;
                let mut cells = String::new();
                let mut lines = String::from("M");
                push_point(&mut lines, x0, y);
                lines.push('H');
                push_num(&mut lines, x0 + w);
                if view.key_w > 0.0 {
                    lines.push('M');
                    push_point(&mut lines, x0 + view.key_w, y);
                    lines.push('V');
                    push_num(&mut lines, h / 2.0);
                }
                let mut value_attrs = String::from(" class=\"ondine-json-value\"");
                value_attrs.push_str(text_attrs);
                for (key, value) in &view.rows {
                    let row_h = value.height.max(js_round(small * 1.5)) + ROW_PAD;
                    cells.push_str(&label_svg(
                        key,
                        x0 + CELL_PAD + key.width / 2.0,
                        y + row_h / 2.0,
                        " class=\"ondine-json-key\" fill=\"var(--_m)\"",
                        None,
                    ));
                    cells.push_str(&left_label(
                        value,
                        x0 + view.key_w + CELL_PAD,
                        y + row_h / 2.0,
                        &value_attrs,
                    ));
                    y += row_h;
                }
                body.push_str("<rect x=\"");
                push_num(&mut body, x0);
                body.push_str("\" y=\"");
                push_num(&mut body, -h / 2.0);
                body.push_str("\" width=\"");
                push_num(&mut body, w);
                body.push_str("\" height=\"");
                push_num(&mut body, h);
                body.push('"');
                body.push_str(ROUNDED);
                body.push_str(SHAPE_ATTRS);
                body.push_str(shape);
                body.push_str("/><path class=\"ondine-json-lines\" d=\"");
                body.push_str(&lines);
                body.push_str("\" fill=\"none\" stroke=\"var(--_b)\"");
                body.push_str(&view.style.line);
                body.push_str("/>");
                let mut attrs = String::from(" class=\"ondine-label\" font-weight=\"var(--_hw)\"");
                attrs.push_str(text_attrs);
                body.push_str(&label_svg(
                    &view.label,
                    0.0,
                    -h / 2.0 + view.head_h / 2.0,
                    &attrs,
                    None,
                ));
                body.push_str("<g font-size=\"");
                push_number(&mut body, small);
                body.push_str("\">");
                body.push_str(&cells);
                body.push_str("</g>");
                "ondine-json"
            }
            _ => {
                let label_y = view.stereotype.height / 2.0;
                let note = node.shape == NodeShape::Note;
                let kind = if view.kind == Kind::Ellipse {
                    body.push_str("<ellipse rx=\"");
                    push_num(&mut body, w / 2.0);
                    body.push_str("\" ry=\"");
                    push_num(&mut body, h / 2.0);
                    body.push('"');
                    body.push_str(SHAPE_ATTRS);
                    body.push_str(shape);
                    body.push_str("/>");
                    if node.business {
                        // The slash of a business use case, cutting off the right end of the
                        // oval beside the text.
                        let from = w / 2.0 - 15.0;
                        let to = w / 2.0 - 5.0;
                        let arc = |at: f64| (h / 2.0) * (1.0 - (at / (w / 2.0)).powi(2)).sqrt();
                        body.push_str("<path class=\"ondine-business\" d=\"M");
                        push_point(&mut body, from, arc(from));
                        body.push('L');
                        push_point(&mut body, to, -arc(to));
                        body.push_str("\" fill=\"none\" stroke=\"var(--_b)\"");
                        body.push_str(&view.style.line);
                        body.push_str("/>");
                    }
                    "ondine-usecase"
                } else if note {
                    let r = w / 2.0;
                    let b = h / 2.0;
                    body.push_str("<path d=\"M");
                    push_point(&mut body, -r, -b);
                    body.push('H');
                    push_num(&mut body, r - FOLD);
                    body.push('L');
                    push_point(&mut body, r, FOLD - b);
                    body.push('V');
                    push_num(&mut body, b);
                    body.push('H');
                    push_num(&mut body, -r);
                    body.push_str(
                        "Z\" fill=\"var(--_a)\" stroke=\"var(--_b)\" stroke-linejoin=\"round\"",
                    );
                    body.push_str(shape);
                    body.push_str("/><path d=\"M");
                    push_point(&mut body, r - FOLD, -b);
                    body.push('V');
                    push_num(&mut body, FOLD - b);
                    body.push('H');
                    push_num(&mut body, r);
                    body.push_str(
                        "\" fill=\"none\" stroke=\"var(--_b)\" stroke-linejoin=\"round\"",
                    );
                    body.push_str(&view.style.line);
                    body.push_str("/>");
                    "ondine-note"
                } else {
                    body.push_str("<rect x=\"");
                    push_num(&mut body, -w / 2.0);
                    body.push_str("\" y=\"");
                    push_num(&mut body, -h / 2.0);
                    body.push_str("\" width=\"");
                    push_num(&mut body, w);
                    body.push_str("\" height=\"");
                    push_num(&mut body, h);
                    body.push('"');
                    body.push_str(ROUNDED);
                    body.push_str(SHAPE_ATTRS);
                    body.push_str(shape);
                    body.push_str("/>");
                    "ondine-usecase"
                };
                body.push_str(&label_svg(
                    &view.stereotype,
                    0.0,
                    -view.label.height / 2.0,
                    &stereotype_attrs,
                    None,
                ));
                let mut attrs = String::from(" class=\"ondine-label\"");
                if note {
                    attrs.push_str(" font-size=\"");
                    push_number(&mut attrs, small);
                    attrs.push('"');
                }
                attrs.push_str(text_attrs);
                body.push_str(&label_svg(&view.label, 0.0, label_y, &attrs, None));
                kind
            }
        };
        nodes_out.push_str("<g class=\"ondine-node ");
        nodes_out.push_str(kind);
        nodes_out.push_str(&classes);
        nodes_out.push_str("\" data-id=\"");
        nodes_out.push_str(&id);
        nodes_out.push_str("\" transform=\"translate(");
        push_point(&mut nodes_out, c.x, c.y);
        nodes_out.push_str(")\">");
        nodes_out.push_str(&body);
        nodes_out.push_str("</g>");
    }

    let mut edges_out = String::new();
    let mut labels_out = String::new();
    let mut loop_seen: Vec<(usize, u32)> = Vec::new();
    for ((ce, edge), label) in cedges.iter_mut().zip(&drawn).zip(&edge_labels) {
        let style = if edge.style.is_empty() && edge.css_compiled_styles.is_empty() {
            None
        } else {
            let mut all = edge.css_compiled_styles.clone();
            all.extend(edge.style.iter().cloned());
            Some(resolve_style(&all))
        };
        let start_type = edge.arrow_start;
        let end_type = edge.arrow_end;
        let path = if ce.src == ce.dst {
            let (Some(c), Some(view)) = (cnodes.get(ce.src), views.get(ce.src)) else {
                continue;
            };
            let k = if let Some((_, seen)) = loop_seen.iter_mut().find(|(node, _)| *node == ce.src)
            {
                *seen += 1;
                *seen - 1
            } else {
                loop_seen.push((ce.src, 1));
                0
            };
            let k = f64::from(k);
            let side = if sideways { 2 } else { 1 };
            let spread = (side_length(view, side) / 2.0).min(8.0 + k * 6.0);
            let reach = LOOP + k * 8.0;
            let (ax, ay) = anchor(view, c, side, -spread);
            let (bx, by) = anchor(view, c, side, spread);
            let hypot = reach.hypot(spread);
            let len = if hypot == 0.0 || hypot.is_nan() {
                1.0
            } else {
                hypot
            };
            let ux = if sideways { spread / len } else { reach / len };
            let uy = if sideways { reach / len } else { spread / len };
            let st = trim(start_type);
            let et = trim(end_type);
            // Both ends leave the node leaning away from each other, and the curve joins them
            // outside it.
            let out = |d: &mut String, x: f64, y: f64, sign: f64, far: f64| {
                let lean = sign * spread * if far == 0.0 { 0.0 } else { 1.0 };
                if sideways {
                    push_point(d, x + lean, y + far);
                } else {
                    push_point(d, x + far, y + lean);
                }
            };
            let sx = if sideways { ax - ux * st } else { ax + ux * st };
            let sy = if sideways { ay + uy * st } else { ay - uy * st };
            let ex = bx + ux * et;
            let ey = by + uy * et;
            let mut d = String::from("M");
            push_point(&mut d, sx, sy);
            d.push('C');
            out(&mut d, ax, ay, -1.0, reach);
            d.push(' ');
            out(&mut d, bx, by, 1.0, reach);
            d.push(' ');
            push_point(&mut d, ex, ey);
            if sideways {
                ce.label_x = c.x;
                ce.label_y = ay.max(by) + reach * 0.75 + 6.0 + label.height / 2.0;
            } else {
                ce.label_x = ax.max(bx) + reach * 0.75 + 8.0 + label.width / 2.0;
                ce.label_y = c.y;
            }
            EdgePath {
                d,
                edx: -ux,
                edy: -uy,
                ex: bx,
                ey: by,
                sdx: if sideways { ux } else { -ux },
                sdy: if sideways { -uy } else { uy },
                sx: ax,
                sy: ay,
            }
        } else {
            if ce.route.len() < 6 {
                continue;
            }
            route_path(
                &ce.route,
                None,
                trim(start_type),
                trim(end_type),
                None,
                None,
            )
        };
        let color = style
            .as_ref()
            .and_then(|style| style.stroke.as_deref())
            .filter(|stroke| !stroke.is_empty())
            .map_or(std::borrow::Cow::Borrowed("var(--_l)"), esc);
        // Mermaid animates these dashes. A still picture keeps the dashes.
        let dashes = if edge.internal {
            "2 3"
        } else if edge.dotted {
            "5 4"
        } else if edge.animate {
            "9 5"
        } else {
            ""
        };
        edges_out.push_str("<g class=\"ondine-edge ondine-");
        edges_out.push_str(edge.relationship_type);
        edges_out.push_str(&class_names(edge_classes(&edge.classes)));
        edges_out.push_str("\" data-id=\"");
        edges_out.push_str(&esc_text(&edge.id));
        edges_out.push('"');
        if let Some(style) = &style {
            edges_out.push_str(&style.line);
        }
        edges_out.push_str("><path d=\"");
        edges_out.push_str(&path.d);
        edges_out.push('"');
        if !dashes.is_empty() {
            edges_out.push_str(" stroke-dasharray=\"");
            edges_out.push_str(dashes);
            edges_out.push('"');
        }
        edges_out.push_str("/>");
        edges_out.push_str(&mark(
            start_type, path.sx, path.sy, path.sdx, path.sdy, &color,
        ));
        edges_out.push_str(&mark(
            end_type, path.ex, path.ey, path.edx, path.edy, &color,
        ));
        edges_out.push_str("</g>");
        if label.width > 0.0 {
            let semantic = matches!(edge.relationship_type, "include" | "extend");
            let mut attrs = String::new();
            if semantic {
                attrs.push_str(" fill=\"var(--_m)\"");
            }
            if let Some(style) = &style {
                attrs.push_str(&style.text);
            }
            labels_out.push_str(&edge_label_svg(
                &edge.id,
                &[],
                label,
                ce.label_x,
                ce.label_y,
                &attrs,
                None,
            ));
        }
    }

    let mut content = label_svg(
        &title,
        width / 2.0,
        pad + title.height / 2.0,
        " class=\"ondine-title\" font-weight=\"var(--_tw)\"",
        None,
    );
    if !clusters.is_empty() {
        content.push_str("<g class=\"ondine-clusters\">");
        content.push_str(&clusters);
        content.push_str("</g>");
    }
    if !edges_out.is_empty() {
        content.push_str(
            "<g class=\"ondine-edges\" fill=\"none\" stroke=\"var(--_l)\" stroke-linecap=\"round\">",
        );
        content.push_str(&edges_out);
        content.push_str("</g>");
    }
    if !labels_out.is_empty() {
        content.push_str("<g class=\"ondine-edge-labels\" font-size=\"");
        push_number(&mut content, small);
        content.push_str("\">");
        content.push_str(&labels_out);
        content.push_str("</g>");
    }
    content.push_str("<g class=\"ondine-nodes\">");
    content.push_str(&nodes_out);
    content.push_str("</g>");

    let accessible = Accessible {
        acc_descr: Some(&model.acc_descr),
        acc_title: Some(&model.acc_title),
    };
    let svg = svg_document(
        "usecase", width, height, size, options, accessible, &content,
    );
    Rendered {
        height,
        links: Vec::new(),
        svg,
        width,
    }
}

/// A boundary's title sits in the middle of its top edge. Where edges enter through that edge it
/// moves into the stretch between them that is nearest the middle, or the widest if none has
/// room.
fn title_x(c: &CNode, width: f64, crossings: &Crossings) -> f64 {
    #[expect(clippy::cast_possible_truncation, reason = "a rounded coordinate")]
    let key = js_round(c.y - c.h / 2.0) as i64;
    let Some(xs) = crossings.get(&key).filter(|_| width != 0.0) else {
        return c.x;
    };
    let left = c.x - c.w / 2.0 + 6.0;
    let right = c.x + c.w / 2.0 - 6.0;
    let mut cuts: Vec<f64> = xs
        .iter()
        .copied()
        .filter(|&x| x > left && x < right)
        .collect();
    cuts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    cuts.push(right);
    let need = width / 2.0 + 6.0;
    let mut best = c.x;
    let mut best_off = f64::INFINITY;
    let mut widest = 0.0;
    let mut from = left;
    for cut in cuts {
        if cut - from >= 2.0 * need {
            let x = c.x.max(from + need).min(cut - need);
            if (x - c.x).abs() < best_off {
                best_off = (x - c.x).abs();
                best = x;
            }
        } else if best_off == f64::INFINITY && cut - from > widest {
            widest = cut - from;
            best = f64::midpoint(from, cut);
        }
        from = cut;
    }
    best
}

/// Writes a label with each line starting at `x`, centered on `cy`.
fn left_label(label: &Label, x: f64, cy: f64, attrs: &str) -> String {
    if label.lines.len() < 2 {
        return label_svg(label, x + label.width / 2.0, cy, attrs, None);
    }
    let mut out = String::new();
    for (i, (line, &width)) in label.lines.iter().zip(&label.widths).enumerate() {
        let one = Label {
            height: label.line_height,
            line_height: label.line_height,
            lines: vec![line.clone()],
            size: label.size,
            width: label.width,
            widths: vec![width],
        };
        #[expect(clippy::cast_precision_loss, reason = "counts are small")]
        let row = i as f64;
        out.push_str(&label_svg(
            &one,
            x + width / 2.0,
            cy - label.height / 2.0 + (row + 0.5) * label.line_height,
            attrs,
            None,
        ));
    }
    out
}
