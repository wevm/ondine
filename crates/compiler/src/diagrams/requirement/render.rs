//! Draws a requirement diagram: boxes with a stereotype, a name, and field rows, joined by
//! relationship edges. Port of Pele's `src/diagrams/requirement/render.ts`.

use super::db::RequirementDb;
use super::graph::{GraphEdge, build_requirement_graph};
use crate::diagrams::Rendered;
use crate::diagrams::common::fit_width::{runs_across, turn_to_fit};
use crate::layout::compound::{CEdge, CNode, Dir, cnode, compound_layout, direction};
use crate::layout::layered::LayeredOptions;
use crate::svg::builder::{
    esc, esc_text, label_svg, push_num, shared_style, span_style, text_style,
};
use crate::svg::edges::{EdgePath, edge_label_svg, route_path};
use crate::svg::root::{Accessible, RenderOptions, svg_document};
use crate::svg::theme::{ROUNDED, ResolvedStyle, class_names, resolve_style};
use crate::text::label::{Label, layout_label};
use crate::text::measure::{self, BOLD, ITALIC};
use crate::util::chars::is_space;
use crate::yaml::push_number;

const NODE_SEP: f64 = 40.0;
const EDGE_SEP: f64 = 16.0;
const RANK_SEP: f64 = 56.0;
const PAD_X: f64 = 14.0;
const PAD_Y: f64 = 10.0;
const ROW_GAP: f64 = 2.0;
const COLUMN_GAP: f64 = 12.0;
const MIN_WIDTH: f64 = 120.0;
const NAME_WRAP: f64 = 280.0;
const VALUE_WRAP: f64 = 240.0;
const LOOP: f64 = 26.0;
const CONTAINS_RADIUS: f64 = 6.0;
const ARROW_LENGTH: f64 = 9.0;
const ARROW_HALF: f64 = 4.5;
const END_STEP: f64 = 28.0;
const END_MARGIN: f64 = 14.0;

/// A field row: its key and value labels.
struct Row {
    height: f64,
    key: Label,
    value: Label,
}

/// A node's measured parts.
struct NodeView {
    h: f64,
    head_height: f64,
    key_width: f64,
    kind: Label,
    name: Label,
    rows: Vec<Row>,
    style: ResolvedStyle,
    w: f64,
}

/// Adds bold or italic to a whole label after its markdown is read, since emphasis markers only
/// toggle styles the label does not start with.
fn restyle(mut label: Label, bits: u8) -> Label {
    if bits == 0 || label.lines.is_empty() {
        return label;
    }
    let mut width: f64 = 0.0;
    let mut widths = Vec::with_capacity(label.lines.len());
    for line in &mut label.lines {
        let mut total = 0.0;
        for span in line {
            span.style |= bits;
            if span.icon.is_none() {
                span.width = measure::width(&span.text, label.size, span.style);
            }
            total += span.width;
        }
        widths.push(total);
        if total > width {
            width = total;
        }
    }
    label.widths = widths;
    label.width = width;
    label
}

/// Writes a measured label with its lines starting at `x`, the first line's box starting at `top`.
fn start_text(label: &Label, x: f64, top: f64, attrs: &str) -> String {
    let mut y = top + label.line_height / 2.0 + label.size * 0.35;
    let shared = shared_style(label);
    let mut body = String::new();
    for line in &label.lines {
        let mut first = true;
        for span in line {
            if span.icon.is_some() || span.text.is_empty() {
                continue;
            }
            body.push_str("<tspan");
            if first {
                body.push_str(" x=\"");
                push_num(&mut body, x);
                body.push_str("\" y=\"");
                push_num(&mut body, y);
                body.push('"');
            }
            body.push_str(span_style(span.style & !shared));
            body.push('>');
            body.push_str(&esc(&span.text));
            body.push_str("</tspan>");
            first = false;
        }
        y += label.line_height;
    }
    if body.is_empty() {
        return body;
    }
    let mut out = String::from("<text");
    out.push_str(&text_style(attrs, shared));
    out.push_str(" text-anchor=\"start\" xml:space=\"preserve\">");
    out.push_str(&body);
    out.push_str("</text>");
    out
}

/// Appends `x,y`.
fn push_point(out: &mut String, x: f64, y: f64) {
    push_num(out, x);
    out.push(',');
    push_num(out, y);
}

/// The containment mark: a circled plus whose rim touches (x, y), where (dx, dy) points at the
/// node.
fn contains_marker(x: f64, y: f64, dx: f64, dy: f64) -> String {
    let r = CONTAINS_RADIUS;
    let cx = x - dx * r;
    let cy = y - dy * r;
    let mut out = String::from("<circle class=\"ondine-marker\" cx=\"");
    push_num(&mut out, cx);
    out.push_str("\" cy=\"");
    push_num(&mut out, cy);
    out.push_str("\" r=\"");
    push_number(&mut out, r);
    out.push_str("\"/><path class=\"ondine-marker\" d=\"M");
    push_point(&mut out, cx - dx * r, cy - dy * r);
    out.push('L');
    push_point(&mut out, cx + dx * r, cy + dy * r);
    out.push('M');
    push_point(&mut out, cx + dy * r, cy - dx * r);
    out.push('L');
    push_point(&mut out, cx - dy * r, cy + dx * r);
    out.push_str("\"/>");
    out
}

/// An open arrowhead with its tip at (x, y), pointing along (dx, dy).
fn arrow_marker(x: f64, y: f64, dx: f64, dy: f64) -> String {
    let bx = x - dx * ARROW_LENGTH;
    let by = y - dy * ARROW_LENGTH;
    let mut out = String::from("<path class=\"ondine-marker\" d=\"M");
    push_point(&mut out, bx - dy * ARROW_HALF, by + dx * ARROW_HALF);
    out.push('L');
    push_point(&mut out, x, y);
    out.push('L');
    push_point(&mut out, bx + dy * ARROW_HALF, by - dx * ARROW_HALF);
    out.push_str("\" stroke-linejoin=\"round\"/>");
    out
}

/// One end of an edge on a side of a node: the edge, the index of its point in the route, and
/// where the edge heads from there.
struct End {
    at: usize,
    edge: usize,
    toward: f64,
}

/// The layout attaches every edge to the middle of a node's side. Ends that share a side are
/// spread along it, in the order of where they head, so their markers do not sit on each other.
#[expect(clippy::cast_precision_loss, reason = "counts are small")]
fn spread_ends(cnodes: &[CNode], cedges: &mut [CEdge], views: &[NodeView]) {
    // Sides in order of first use, as a `Map` iterates them.
    let mut sides: Vec<(usize, Vec<End>)> = Vec::new();
    for (e, edge) in cedges.iter().enumerate() {
        let route = &edge.route;
        if edge.src == edge.dst || route.len() < 6 {
            continue;
        }
        for at in [0, route.len() - 3] {
            let node = if at == 0 { edge.src } else { edge.dst };
            let Some(c) = cnodes.get(node) else {
                continue;
            };
            let point = |k: usize| route.get(k).copied().unwrap_or(0.0);
            let dx = point(at) - c.x;
            let dy = point(at + 1) - c.y;
            let next = if at == 0 { 3 } else { at - 3 };
            let side = if ((dy.abs() - c.h / 2.0).abs()) < 0.5 {
                if dy < 0.0 { 0 } else { 2 }
            } else if ((dx.abs() - c.w / 2.0).abs()) < 0.5 {
                if dx < 0.0 { 3 } else { 1 }
            } else {
                continue;
            };
            let key = node * 4 + side;
            let end = End {
                at,
                edge: e,
                toward: point(if side % 2 == 0 { next } else { next + 1 }),
            };
            match sides.iter_mut().find(|(k, _)| *k == key) {
                Some((_, list)) => list.push(end),
                None => sides.push((key, vec![end])),
            }
        }
    }

    for (key, mut list) in sides {
        if list.len() < 2 {
            continue;
        }
        let node = key >> 2;
        let vertical = key % 2 == 1;
        let Some(view) = views.get(node) else {
            continue;
        };
        let span = (if vertical { view.h } else { view.w }) - 2.0 * END_MARGIN;
        let step = (span / (list.len() - 1) as f64).clamp(0.0, END_STEP);
        list.sort_by(|a, b| {
            (a.toward - b.toward)
                .partial_cmp(&0.0)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        let count = list.len();
        let Some(c) = cnodes.get(node) else {
            continue;
        };
        for (j, end) in list.iter().enumerate() {
            let offset = (j as f64 - (count - 1) as f64 / 2.0) * step;
            let Some(route) = cedges.get_mut(end.edge).map(|edge| &mut edge.route) else {
                continue;
            };
            if vertical {
                if let Some(y) = route.get_mut(end.at + 1) {
                    *y = c.y + offset;
                }
            } else if let Some(x) = route.get_mut(end.at) {
                *x = c.x + offset;
            }
        }
    }
}

/// Draws `model`, turned to run down when it runs across and does not fit the host's width.
pub(crate) fn render(model: &RequirementDb, options: &RenderOptions) -> Rendered {
    if !runs_across(Some(model.direction)) {
        return draw(model, options, false);
    }
    turn_to_fit(options, |down| draw(model, options, down))
}

/// `class` without a leading `default` and one whitespace character after it, as
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

/// Wraps text in guillemets, as a stereotype.
fn stereotype(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 4);
    out.push('«');
    out.push_str(text);
    out.push('»');
    out
}

/// `turned` draws a diagram that runs across as one that runs down.
#[expect(clippy::too_many_lines, reason = "mirrors Pele's draw")]
fn draw(model: &RequirementDb, options: &RenderOptions, turned: bool) -> Rendered {
    let graph = build_requirement_graph(model);
    let size = options.font_size.unwrap_or(16.0);
    let small = crate::svg::builder::js_round(size * 0.875);
    let pad = options.padding.unwrap_or(8.0);

    let mut index: Vec<(&str, usize)> = Vec::new();
    let mut views: Vec<NodeView> = Vec::new();
    let mut cnodes: Vec<CNode> = Vec::new();
    for node in &graph.nodes {
        let style = resolve_style(node.css_styles);
        let base = (if style.bold { BOLD } else { 0 }) | (if style.italic { ITALIC } else { 0 });
        let name_size = style.font_size.unwrap_or(size);
        let body_size = style.font_size.unwrap_or(small);
        let kind_text = stereotype(node.requirement.map_or("Element", |r| r.r#type.as_str()));
        let kind = restyle(
            layout_label(Some(&kind_text), false, body_size, 4000.0, 0, None),
            base,
        );
        let name = restyle(
            layout_label(Some(node.id), true, name_size, NAME_WRAP, 0, None),
            base | BOLD,
        );
        let fields: Vec<(&str, &str)> = match (node.requirement, node.element) {
            (Some(r), _) => vec![
                ("ID", r.requirement_id.as_str()),
                ("Text", r.text.as_str()),
                ("Risk", r.risk.as_str()),
                ("Verification", r.verify_method.as_str()),
            ],
            (None, Some(e)) => vec![("Type", e.r#type.as_str()), ("Doc Ref", e.doc_ref.as_str())],
            (None, None) => Vec::new(),
        };
        let mut rows = Vec::new();
        let mut key_width: f64 = 0.0;
        let mut value_width: f64 = 0.0;
        let mut body_height = 0.0;
        for (label, text) in fields {
            if text.is_empty() {
                continue;
            }
            let key = restyle(
                layout_label(Some(label), false, body_size, 4000.0, 0, None),
                base,
            );
            let value = restyle(
                layout_label(Some(text), true, body_size, VALUE_WRAP, 0, None),
                base,
            );
            let height = f64::max(key.height, value.height);
            key_width = key_width.max(key.width);
            value_width = value_width.max(value.width);
            body_height += height + ROW_GAP;
            rows.push(Row { height, key, value });
        }
        if !rows.is_empty() {
            body_height += 2.0 * PAD_Y - ROW_GAP;
        }
        let head_height = 2.0 * PAD_Y + kind.height + name.height;
        let w = MIN_WIDTH
            .max(kind.width.max(name.width) + 2.0 * PAD_X)
            .max(if rows.is_empty() {
                0.0
            } else {
                key_width + COLUMN_GAP + value_width + 2.0 * PAD_X
            });
        // A requirement and an element may share a name; relationships then go to the requirement.
        if !index.iter().any(|(id, _)| *id == node.id) {
            index.push((node.id, views.len()));
        }
        let mut c = cnode(w, head_height + body_height, -1);
        #[expect(clippy::cast_precision_loss, reason = "counts are small")]
        {
            c.seq = views.len() as f64;
        }
        views.push(NodeView {
            h: head_height + body_height,
            head_height,
            key_width,
            kind,
            name,
            rows,
            style,
            w,
        });
        cnodes.push(c);
    }
    let find = |id: Option<&str>| {
        id.and_then(|id| index.iter().find(|(own, _)| *own == id).map(|&(_, at)| at))
    };

    let root_dir = if turned {
        Dir::Tb
    } else {
        direction(Some(graph.direction))
    };
    let sideways = matches!(root_dir, Dir::Lr | Dir::Rl);
    let mut cedges: Vec<CEdge> = Vec::new();
    let mut edge_labels: Vec<Label> = Vec::new();
    let mut drawn: Vec<&GraphEdge<'_>> = Vec::new();
    for edge in &graph.edges {
        let (Some(src), Some(dst)) = (find(edge.start), find(edge.end)) else {
            continue;
        };
        let label = layout_label(
            Some(&stereotype(edge.r#type)),
            false,
            small,
            4000.0,
            0,
            None,
        );
        if src == dst
            && let Some(c) = cnodes.get_mut(src)
        {
            // A self-loop sits beside the node, across the flow, and the node's box grows to hold
            // it.
            if sideways {
                c.h += 2.0 * (LOOP + label.height + 8.0);
            } else {
                c.w += 2.0 * (LOOP + label.width + 12.0);
            }
        }
        cedges.push(CEdge {
            dst,
            label_h: label.height + 4.0,
            label_w: label.width + 12.0,
            label_x: 0.0,
            label_y: 0.0,
            minlen: 1,
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
            node_sep: NODE_SEP,
            port_sep: 20.0,
            rank_sep: RANK_SEP,
            straight: false,
            tracks: false,
        },
    );

    spread_ends(&cnodes, &mut cedges, &views);

    let title = layout_label(model.title.as_deref(), false, size, 4000.0, BOLD, None);
    let title_height = if title.height > 0.0 {
        title.height + 12.0
    } else {
        0.0
    };
    let inner = layout.width.max(title.width);
    let ox = pad + (inner - layout.width) / 2.0;
    let oy = pad + title_height;
    let width = (inner + 2.0 * pad).ceil();
    let height = (layout.height + title_height + 2.0 * pad).ceil();

    let mut nodes_out = String::new();
    for (i, view) in views.iter().enumerate() {
        let (Some(node), Some(c)) = (graph.nodes.get(i), cnodes.get(i)) else {
            continue;
        };
        let w = view.w;
        let h = view.h;
        let style = &view.style;
        let left = -w / 2.0;
        let top = -h / 2.0;

        let mut body = String::from("<rect x=\"");
        push_num(&mut body, left);
        body.push_str("\" y=\"");
        push_num(&mut body, top);
        body.push_str("\" width=\"");
        push_num(&mut body, w);
        body.push_str("\" height=\"");
        push_num(&mut body, h);
        body.push('"');
        body.push_str(ROUNDED);
        body.push_str(" fill=\"var(--_s)\" stroke=\"var(--_b)\"");
        body.push_str(&style.shape);
        body.push_str("/>");

        let mut attrs = String::from(" class=\"ondine-stereotype\" font-size=\"");
        push_number(&mut attrs, view.kind.size);
        attrs.push_str("\" fill=\"var(--_m)\"");
        attrs.push_str(&style.text);
        body.push_str(&label_svg(
            &view.kind,
            0.0,
            top + PAD_Y + view.kind.height / 2.0,
            &attrs,
            None,
        ));
        let mut attrs = String::from(" class=\"ondine-label\" font-size=\"");
        push_number(&mut attrs, view.name.size);
        attrs.push_str("\" font-weight=\"var(--_hw)\"");
        attrs.push_str(&style.text);
        body.push_str(&label_svg(
            &view.name,
            0.0,
            top + PAD_Y + view.kind.height + view.name.height / 2.0,
            &attrs,
            None,
        ));

        if let Some(first) = view.rows.first() {
            let divider = top + view.head_height;
            body.push_str("<path class=\"ondine-divider\" d=\"M");
            push_point(&mut body, left, divider);
            body.push('H');
            push_num(&mut body, -left);
            body.push_str("\" stroke=\"var(--_b)\"");
            body.push_str(&style.line);
            body.push_str("/>");

            let mut key_attrs = String::from(" class=\"ondine-field\" fill=\"var(--_m)\"");
            key_attrs.push_str(&style.text);
            let mut value_attrs = String::from(" class=\"ondine-value\"");
            value_attrs.push_str(&style.text);
            let mut y = divider + PAD_Y;
            let mut rows = String::new();
            for row in &view.rows {
                rows.push_str(&start_text(&row.key, left + PAD_X, y, &key_attrs));
                rows.push_str(&start_text(
                    &row.value,
                    left + PAD_X + view.key_width + COLUMN_GAP,
                    y,
                    &value_attrs,
                ));
                y += row.height + ROW_GAP;
            }
            body.push_str("<g class=\"ondine-fields\" font-size=\"");
            push_number(&mut body, first.key.size);
            body.push_str("\">");
            body.push_str(&rows);
            body.push_str("</g>");
        }

        nodes_out.push_str("<g class=\"ondine-node ");
        nodes_out.push_str(if node.requirement.is_some() {
            "ondine-requirement"
        } else {
            "ondine-element"
        });
        nodes_out.push_str(&class_names(without_default(&node.css_classes)));
        nodes_out.push_str("\" data-id=\"");
        nodes_out.push_str(&esc_text(node.id));
        nodes_out.push_str("\" transform=\"translate(");
        push_point(&mut nodes_out, c.x + ox, c.y + oy);
        nodes_out.push_str(")\">");
        nodes_out.push_str(&body);
        nodes_out.push_str("</g>");
    }

    let mut edges_out = String::new();
    let mut labels_out = String::new();
    let mut loop_seen: Vec<(usize, u32)> = Vec::new();
    for ((ce, edge), label) in cedges.iter().zip(&drawn).zip(&edge_labels) {
        let start_trim = if edge.contains {
            2.0 * CONTAINS_RADIUS
        } else {
            0.0
        };
        let mut label_x = ce.label_x + ox;
        let mut label_y = ce.label_y + oy;
        let path = if ce.src == ce.dst {
            let (Some(view), Some(c)) = (views.get(ce.src), cnodes.get(ce.src)) else {
                continue;
            };
            let cx = c.x + ox;
            let cy = c.y + oy;
            let k = if let Some((_, seen)) = loop_seen.iter_mut().find(|(node, _)| *node == ce.src)
            {
                *seen += 1;
                *seen - 1
            } else {
                loop_seen.push((ce.src, 1));
                0
            };
            let k = f64::from(k);
            let half = (if sideways { view.w } else { view.h }) / 2.0;
            let spread = f64::min(half - 4.0, 12.0 + k * 6.0);
            let reach = LOOP + k * 8.0;
            // Local frame: `out` points away from the node, `along` runs along its side.
            let at = |d: &mut String, out: f64, along: f64| {
                if sideways {
                    push_point(d, cx + along, cy + view.h / 2.0 + out);
                } else {
                    push_point(d, cx + view.w / 2.0 + out, cy + along);
                }
            };
            // A cubic whose control points are this far out peaks at `reach`.
            let pull = reach / 0.75;
            let mut d = String::from("M");
            at(&mut d, start_trim, -spread);
            d.push('C');
            at(&mut d, pull, -spread);
            d.push(' ');
            at(&mut d, pull, spread);
            d.push(' ');
            at(&mut d, 0.0, spread);
            if sideways {
                label_x = cx;
                label_y = cy + view.h / 2.0 + reach + 4.0 + ce.label_h / 2.0;
            } else {
                label_x = cx + view.w / 2.0 + reach + 4.0 + ce.label_w / 2.0;
                label_y = cy;
            }
            EdgePath {
                d,
                edx: if sideways { 0.0 } else { -1.0 },
                edy: if sideways { -1.0 } else { 0.0 },
                ex: if sideways {
                    cx + spread
                } else {
                    cx + view.w / 2.0
                },
                ey: if sideways {
                    cy + view.h / 2.0
                } else {
                    cy + spread
                },
                sdx: if sideways { 0.0 } else { -1.0 },
                sdy: if sideways { -1.0 } else { 0.0 },
                sx: if sideways {
                    cx - spread
                } else {
                    cx + view.w / 2.0
                },
                sy: if sideways {
                    cy + view.h / 2.0
                } else {
                    cy - spread
                },
            }
        } else {
            let mut route = ce.route.clone();
            for point in route.chunks_mut(3) {
                if let [x, y, ..] = point {
                    *x += ox;
                    *y += oy;
                }
            }
            route_path(&route, None, start_trim, 0.0, None, None)
        };

        edges_out.push_str("<g class=\"ondine-edge ondine-relationship-");
        edges_out.push_str(if edge.contains { "contains" } else { "arrow" });
        edges_out.push_str("\" data-id=\"");
        edges_out.push_str(&esc_text(&edge.id));
        edges_out.push_str("\"><path d=\"");
        edges_out.push_str(&path.d);
        edges_out.push('"');
        if !edge.contains {
            edges_out.push_str(" stroke-dasharray=\"5 4\"");
        }
        edges_out.push_str("/>");
        if edge.contains {
            edges_out.push_str(&contains_marker(path.sx, path.sy, path.sdx, path.sdy));
        } else {
            edges_out.push_str(&arrow_marker(path.ex, path.ey, path.edx, path.edy));
        }
        edges_out.push_str("</g>");
        labels_out.push_str(&edge_label_svg(
            &edge.id,
            &[],
            label,
            label_x,
            label_y,
            "",
            None,
        ));
    }

    let mut content = label_svg(
        &title,
        width / 2.0,
        pad + title.height / 2.0,
        " class=\"ondine-title\" font-weight=\"var(--_tw)\"",
        None,
    );
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
        content.push_str("\" fill=\"var(--_m)\">");
        content.push_str(&labels_out);
        content.push_str("</g>");
    }
    content.push_str("<g class=\"ondine-nodes\">");
    content.push_str(&nodes_out);
    content.push_str("</g>");

    let accessible = Accessible {
        acc_descr: model.acc_descr.as_deref(),
        acc_title: model.acc_title.as_deref(),
    };
    let svg = svg_document(
        "requirement",
        width,
        height,
        size,
        options,
        accessible,
        &content,
    );

    Rendered {
        height,
        links: Vec::new(),
        svg,
        width,
    }
}
