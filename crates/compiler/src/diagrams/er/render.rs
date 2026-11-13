//! Lays out and draws an ER diagram. Port of Pele's `src/diagrams/er/render.ts`.

use std::cmp::Ordering;
use std::collections::BTreeMap;

use super::db::ErDb;
use super::graph::{self, ErGraphNode};
use super::markers::{MARKER_LENGTH, er_marker};
use crate::diagrams::Rendered;
use crate::diagrams::common::fit_width::{runs_across, turn_to_fit};
use crate::diagrams::common::generics::parse_generic_types;
use crate::layout::compound::{CEdge, CNode, Dir, cnode, compound_layout, direction, shift_layout};
use crate::layout::layered::LayeredOptions;
use crate::svg::builder::{
    esc, esc_text, js_round, label_svg, push_num, shared_style, span_style, text_style,
};
use crate::svg::cluster::{Crossings, cluster_title_x, mark_crossings};
use crate::svg::edges::{edge_label_svg, route_path};
use crate::svg::root::{Accessible, RenderOptions, svg_document};
use crate::svg::theme::{RADIUS, ResolvedStyle, class_names, resolve_style};
use crate::text::entities;
use crate::text::label::{Label, layout_label};
use crate::text::measure::{BOLD, ITALIC};
use crate::util::chars::is_space;
use crate::util::url::{self, LinkInfo, UrlOptions};
use crate::yaml::Value;

/// Room between nodes in a rank.
const NODE_SEP: f64 = 48.0;
/// Room between edges.
const EDGE_SEP: f64 = 20.0;
/// Room between ranks.
const RANK_SEP: f64 = 64.0;
/// Padding inside a subgraph.
const GROUP_PAD: f64 = 20.0;
/// How far a self-relationship reaches out past its markers.
const LOOP: f64 = 28.0;
/// The least stretch of side a self-relationship takes.
const LOOP_BAND: f64 = 38.0;
/// Least width of an entity.
const MIN_WIDTH: f64 = 96.0;
/// Padding beside an entity's name.
const BOX_PAD_X: f64 = 18.0;
/// Padding above and below the name of an entity without attributes.
const BOX_PAD_Y: f64 = 12.0;
/// Padding above and below the name of an entity with attributes.
const HEAD_PAD_Y: f64 = 9.0;
/// Padding beside the attribute table.
const CELL_PAD_X: f64 = 12.0;
/// Room between attribute columns.
const CELL_GAP: f64 = 16.0;
/// Padding of an attribute row.
const ROW_PAD: f64 = 9.0;
/// The width comments wrap at.
const COMMENT_WRAP: f64 = 260.0;
/// A width no cell wraps at.
const NO_WRAP: f64 = 1e6;
/// Room between edge ends on one side.
const PORT_GAP: f64 = 18.0;
/// Room between the outermost edge end and a corner.
const PORT_MARGIN: f64 = 14.0;
/// The fill and stroke of an entity box.
const BOX_ATTRS: &str = " fill=\"var(--_s)\" stroke=\"var(--_b)\"";

/// The value under `key` of a YAML mapping.
fn get<'v>(value: &'v Value, key: &str) -> Option<&'v Value> {
    match value {
        Value::Object(entries) => entries.iter().find(|(own, _)| own == key).map(|(_, v)| v),
        _ => None,
    }
}

/// A positive number option of the `er` section, capped at 2000, or `fallback`.
fn number_option(config: &Value, key: &str, fallback: f64) -> f64 {
    match get(config, "er").and_then(|section| get(section, key)) {
        Some(Value::Number(value)) if *value > 0.0 => value.min(2000.0),
        _ => fallback,
    }
}

/// An attribute row: its type, name, keys, and comment cells, and its height.
struct Row {
    cells: [Label; 4],
    h: f64,
}

/// What a node is drawn as, beside its layout box.
struct NodeView<'g> {
    /// Whether self-relationships loop out of the bottom side rather than the right.
    below: bool,
    /// Left edge of each column, measured from the left edge of the box.
    cols: Vec<f64>,
    h: f64,
    head_h: f64,
    key_size: f64,
    label: Label,
    node: &'g ErGraphNode<'g>,
    row_size: f64,
    rows: Vec<Row>,
    style: ResolvedStyle,
    w: f64,
}

/// A relationship from an entity to itself, with the stretch of the side it loops out of.
struct Loop {
    at: f64,
    band: f64,
    label: Label,
}

/// A count as a coordinate.
#[expect(clippy::cast_precision_loss, reason = "counts stay far below 2^52")]
fn to_f64(value: usize) -> f64 {
    value as f64
}

/// Writes a measured label starting at `x`, with the baseline of its first line at `y`.
fn text_left(label: &Label, x: f64, mut y: f64, attrs: &str) -> String {
    let lines = &label.lines;
    if lines.is_empty() {
        return String::new();
    }
    if let [line] = lines.as_slice()
        && let [span] = line.as_slice()
        && span.icon.is_none()
        && span.style == 0
    {
        let mut out = String::from("<text");
        out.push_str(attrs);
        out.push_str(" x=\"");
        push_num(&mut out, x);
        out.push_str("\" y=\"");
        push_num(&mut out, y);
        out.push_str("\">");
        out.push_str(&esc(&span.text));
        out.push_str("</text>");
        return out;
    }
    let shared = shared_style(label);
    let mut body = String::new();
    let mut extra = String::new();
    for line in lines {
        let has_icon = line.iter().any(|span| span.icon.is_some());
        let mut sx = x;
        let mut placed = false;
        for span in line {
            if let Some(icon) = &span.icon {
                let side = label.size * 1.1;
                extra.push_str("<svg class=\"ondine-icon\" data-icon=\"");
                extra.push_str(&esc_text(icon));
                extra.push_str("\" x=\"");
                push_num(&mut extra, sx);
                extra.push_str("\" y=\"");
                push_num(&mut extra, y - label.size * 0.35 - side / 2.0);
                extra.push_str("\" width=\"");
                push_num(&mut extra, side);
                extra.push_str("\" height=\"");
                push_num(&mut extra, side);
                extra.push_str(
                    "\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" \
                     stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"></svg>",
                );
            } else if !span.text.is_empty() {
                body.push_str("<tspan");
                if has_icon || !placed {
                    body.push_str(" x=\"");
                    push_num(&mut body, sx);
                    body.push_str("\" y=\"");
                    push_num(&mut body, y);
                    body.push('"');
                }
                body.push_str(span_style(span.style & !shared));
                body.push('>');
                body.push_str(&esc(&span.text));
                body.push_str("</tspan>");
                placed = true;
            }
            sx += span.width;
        }
        y += label.line_height;
    }
    let mut out = String::from("<text");
    out.push_str(&text_style(attrs, shared));
    out.push_str(" xml:space=\"preserve\">");
    out.push_str(&body);
    out.push_str("</text>");
    out.push_str(&extra);
    out
}

/// Mermaid reads `~` pairs as angle brackets. They are written as entities so the label parser
/// does not take `<int>` for a tag.
fn cell_text(raw: &str) -> String {
    let converted = parse_generic_types(raw);
    if converted == raw {
        return converted;
    }
    let mut out = String::with_capacity(converted.len() + 8);
    for c in converted.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            c => out.push(c),
        }
    }
    out
}

/// Removes a leading `default` class and one following whitespace character, as
/// `/^default\s?/` does.
fn strip_default(classes: &str) -> &str {
    let Some(rest) = classes.strip_prefix("default") else {
        return classes;
    };
    match rest.chars().next() {
        Some(c) if is_space(u32::from(c)) => rest.get(c.len_utf8()..).unwrap_or(""),
        _ => rest,
    }
}

/// Draws an ER diagram, turning one that runs across to run down when it does not fit the host's
/// width.
pub(crate) fn render(
    db: &ErDb,
    config: &Value,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
) -> Rendered {
    if !runs_across(Some(db.direction)) {
        return draw(db, config, options, url, false);
    }
    turn_to_fit(options, |down| draw(db, config, options, url, down))
}

/// `turned` draws a diagram that runs across as one that runs down.
#[expect(
    clippy::many_single_char_names,
    clippy::too_many_lines,
    reason = "mirrors Pele's draw, one pass per drawing stage"
)]
fn draw(
    db: &ErDb,
    config: &Value,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
    turned: bool,
) -> Rendered {
    let graph = graph::build(db);
    let size = options.font_size.unwrap_or(16.0);
    let small_size = js_round(size * 0.875);
    let pad = options.padding.unwrap_or(8.0);
    let n = graph.nodes.len();

    let mut index: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, node) in graph.nodes.iter().enumerate() {
        index.insert(node.id, i);
    }

    let mut cedges: Vec<CEdge> = Vec::new();
    let mut edge_labels: Vec<Label> = Vec::new();
    let mut drawn = Vec::new();
    let mut loops: Vec<Loop> = Vec::new();
    // The loops of each node, in the order nodes first got one.
    let mut loops_of: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut loop_of: Vec<Option<usize>> = Vec::new();
    for edge in &graph.edges {
        let (Some(&src), Some(&dst)) = (index.get(edge.start), index.get(edge.end)) else {
            continue;
        };
        let label = layout_label(Some(edge.label), true, small_size, 200.0, 0, None);
        let mut own = None;
        if src == dst {
            own = Some(loops.len());
            match loops_of.iter_mut().find(|(node, _)| *node == src) {
                Some((_, list)) => list.push(loops.len()),
                None => loops_of.push((src, vec![loops.len()])),
            }
            loops.push(Loop {
                at: 0.0,
                band: 0.0,
                label: label.clone(),
            });
        }
        loop_of.push(own);
        drawn.push(edge);
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
            minlen: 1,
            route: Vec::new(),
            src,
        });
        edge_labels.push(label);
    }

    // The direction of the level a node is laid out in: its nearest subgraph that sets one, or
    // the diagram's.
    let horizontal = |i: usize| -> bool {
        let mut at = i;
        // Subgraphs can name each other as members, so the walk up is bounded.
        for _ in 0..n {
            let parent = graph.nodes.get(at).and_then(|node| node.parent_id);
            let Some(&up) = parent.and_then(|parent| index.get(parent)) else {
                break;
            };
            if let Some(dir) = graph.nodes.get(up).and_then(|node| node.dir) {
                return dir == "LR" || dir == "RL";
            }
            at = up;
        }
        db.direction == "LR" || db.direction == "RL"
    };

    let mut views: Vec<NodeView<'_>> = Vec::with_capacity(n);
    let mut cnodes: Vec<CNode> = Vec::with_capacity(n);
    for (i, node) in graph.nodes.iter().enumerate() {
        let style = if node.css_styles.is_empty() {
            resolve_style(&node.css_compiled_styles)
        } else {
            let mut all = node.css_compiled_styles.clone();
            all.extend(node.css_styles.iter().cloned());
            resolve_style(&all)
        };
        let base = (if style.bold { BOLD } else { 0 }) | (if style.italic { ITALIC } else { 0 });
        let name_size = style.font_size.unwrap_or(size);
        let row_size = style.font_size.unwrap_or(small_size);
        let key_size = style.font_size.unwrap_or_else(|| js_round(size * 0.75));
        let label = if node.is_group {
            layout_label(Some(node.label), true, name_size, 4000.0, base, None)
        } else {
            let name = if node.alias.is_empty() {
                node.label
            } else {
                node.alias
            };
            layout_label(Some(name), true, name_size, 240.0, base, None)
        };
        let own: &[usize] = loops_of
            .iter()
            .find(|(at, _)| *at == i)
            .map_or(&[], |(_, list)| list.as_slice());
        // Loops leave by a side the layout keeps free of other edges: the right when ranks run
        // down, the bottom when they run across.
        let below = !own.is_empty() && !node.is_group && horizontal(i);
        let mut reach: f64 = 0.0;
        let mut bands = 0.0;
        for &k in own {
            if let Some(item) = loops.get_mut(k) {
                item.band = LOOP_BAND.max(if below {
                    item.label.width + 12.0
                } else {
                    item.label.height + 6.0
                });
                bands += item.band;
                reach = reach.max(if below {
                    item.label.height
                } else {
                    item.label.width
                });
            }
        }
        let min_w = MIN_WIDTH.max(if below { bands + 8.0 } else { 0.0 });
        let mut view = NodeView {
            below,
            cols: Vec::new(),
            h: 0.0,
            head_h: 0.0,
            key_size,
            label,
            node,
            row_size,
            rows: Vec::new(),
            style,
            w: 0.0,
        };
        let mut c = if node.is_group {
            let mut c = cnode(0.0, 0.0, -1);
            c.is_group = true;
            c.dir = node.dir.map(|dir| direction(Some(dir)));
            c.pad_x = GROUP_PAD;
            c.pad_top = if view.label.height > 0.0 {
                view.label.height + 16.0
            } else {
                GROUP_PAD
            };
            c.pad_bottom = GROUP_PAD;
            c.min_w = (view.label.width + 2.0 * GROUP_PAD - 12.0).max(MIN_WIDTH);
            c
        } else {
            let name_h = view.label.height.max(js_round(name_size * 1.5));
            if node.attributes.is_empty() {
                view.w = (view.label.width + 2.0 * BOX_PAD_X).max(min_w);
                view.h = (name_h + 2.0 * BOX_PAD_Y).max(if below { 0.0 } else { bands + 8.0 });
            } else {
                let row_line = js_round(row_size * 1.5);
                let mut widths = [0.0f64; 4];
                for attribute in node.attributes {
                    let keys = attribute.keys.join(", ");
                    let cells = [
                        layout_label(
                            Some(&cell_text(&attribute.r#type)),
                            true,
                            row_size,
                            NO_WRAP,
                            base,
                            None,
                        ),
                        layout_label(
                            Some(&cell_text(&attribute.name)),
                            true,
                            row_size,
                            NO_WRAP,
                            base,
                            None,
                        ),
                        layout_label(Some(&keys), false, key_size, NO_WRAP, base, None),
                        layout_label(
                            Some(&cell_text(&attribute.comment)),
                            true,
                            row_size,
                            COMMENT_WRAP,
                            base,
                            None,
                        ),
                    ];
                    let mut h = row_line;
                    for (cell, width) in cells.iter().zip(widths.iter_mut()) {
                        if cell.width > *width {
                            *width = cell.width;
                        }
                        if cell.height > h {
                            h = cell.height;
                        }
                    }
                    view.rows.push(Row {
                        cells,
                        h: h + ROW_PAD,
                    });
                    view.h += h + ROW_PAD;
                }
                let mut columns = 0usize;
                let mut inner = 0.0;
                for &w in &widths {
                    if w == 0.0 {
                        continue;
                    }
                    inner += w + if columns > 0 { CELL_GAP } else { 0.0 };
                    columns += 1;
                }
                view.w = (inner + 2.0 * CELL_PAD_X)
                    .max(view.label.width + 2.0 * BOX_PAD_X)
                    .max(min_w);
                // A box wider than its table shares the spare width between the columns.
                let spare = if columns > 0 {
                    (view.w - inner - 2.0 * CELL_PAD_X) / to_f64(columns)
                } else {
                    0.0
                };
                let mut x = CELL_PAD_X;
                for &w in &widths {
                    view.cols.push(x);
                    if w > 0.0 {
                        x += w + spare + CELL_GAP;
                    }
                }
                view.head_h = name_h + 2.0 * HEAD_PAD_Y;
                view.h += view.head_h;
            }
            // The layout keeps a node centered in its box, so room for loops is added on both
            // sides.
            let room = if own.is_empty() {
                0.0
            } else {
                2.0 * (MARKER_LENGTH + LOOP * 0.75 + 10.0 + reach)
            };
            if below {
                cnode(view.w, view.h + room, -1)
            } else {
                cnode(view.w + room, view.h, -1)
            }
        };
        c.seq = to_f64(views.len());
        views.push(view);
        cnodes.push(c);
    }
    for (i, node) in graph.nodes.iter().enumerate() {
        let parent = node
            .parent_id
            .and_then(|id| index.get(id))
            .map_or(-1, |&p| i32::try_from(p).unwrap_or(-1));
        if let Some(c) = cnodes.get_mut(i) {
            c.parent = parent;
        }
    }

    let layout = compound_layout(
        &mut cnodes,
        &mut cedges,
        if turned {
            Dir::Tb
        } else {
            direction(Some(db.direction))
        },
        &LayeredOptions {
            apart: false,
            edge_sep: EDGE_SEP,
            head_room: 0.0,
            node_sep: number_option(config, "nodeSpacing", NODE_SEP),
            port_sep: 24.0,
            rank_sep: number_option(config, "rankSpacing", RANK_SEP),
            straight: false,
            tracks: false,
        },
    );

    spread_ends(&mut cedges, &cnodes, &views);

    // A subgraph gets its size from the layout, so its loops hang outside it and may widen the
    // diagram.
    let mut layout_width = layout.width;
    for (i, own) in &loops_of {
        let Some(c) = cnodes.get(*i).filter(|c| c.is_group) else {
            continue;
        };
        let mut reach: f64 = 0.0;
        for &k in own {
            if let Some(item) = loops.get(k) {
                reach = reach.max(item.label.width);
            }
        }
        layout_width =
            layout_width.max(c.x + c.w / 2.0 + MARKER_LENGTH + LOOP * 0.75 + 10.0 + reach);
    }

    let title = layout_label(db.title.as_deref(), false, size, 4000.0, BOLD, None);
    let title_height = if title.height > 0.0 {
        title.height + 12.0
    } else {
        0.0
    };
    let inner = layout_width.max(title.width);
    let ox = pad + (inner - layout_width) / 2.0;
    let oy = pad + title_height;
    shift_layout(&mut cnodes, &mut cedges, ox, oy);
    let width = (inner + 2.0 * pad).ceil();
    let height = (layout.height + title_height + 2.0 * pad).ceil();
    let mut links: Vec<LinkInfo> = Vec::new();

    let mut crossings = Crossings::new();
    if cnodes.iter().any(|c| c.is_group) {
        for e in &cedges {
            mark_crossings(&mut crossings, &e.route);
        }
    }

    let mut clusters = String::new();
    let mut nodes_out = String::new();
    for (i, view) in views.iter_mut().enumerate() {
        let Some(c) = cnodes.get(i) else { continue };
        let node = view.node;
        let classes = class_names(strip_default(&node.css_classes));
        if c.is_group {
            view.w = c.w;
            view.h = c.h;
            let x = c.x - c.w / 2.0;
            let y = c.y - c.h / 2.0;
            let title_x = cluster_title_x(c, view.label.width, &crossings, GROUP_PAD);
            clusters.push_str("<g class=\"ondine-cluster");
            clusters.push_str(&classes);
            clusters.push_str("\" data-id=\"");
            clusters.push_str(&esc_text(node.id));
            clusters.push_str("\"><rect x=\"");
            push_num(&mut clusters, x);
            clusters.push_str("\" y=\"");
            push_num(&mut clusters, y);
            clusters.push_str("\" width=\"");
            push_num(&mut clusters, c.w);
            clusters.push_str("\" height=\"");
            push_num(&mut clusters, c.h);
            clusters.push_str("\" rx=\"");
            clusters.push_str(RADIUS);
            clusters.push_str("\" fill=\"var(--_a)\" fill-opacity=\"0.5\" stroke=\"var(--_b)\"");
            clusters.push_str(&view.style.shape);
            clusters.push_str("/>");
            let mut attrs = String::from(" class=\"ondine-cluster-label\" fill=\"var(--_m)\"");
            attrs.push_str(&view.style.text);
            clusters.push_str(&label_svg(
                &view.label,
                title_x,
                y + 8.0 + view.label.height / 2.0,
                &attrs,
                None,
            ));
            clusters.push_str("</g>");
            continue;
        }

        let w = view.w;
        let h = view.h;
        let x0 = -w / 2.0;
        let y0 = -h / 2.0;
        let text = view.style.text.as_str();
        let mut shape = String::from("x=\"");
        push_num(&mut shape, x0);
        shape.push_str("\" y=\"");
        push_num(&mut shape, y0);
        shape.push_str("\" width=\"");
        push_num(&mut shape, w);
        shape.push_str("\" height=\"");
        push_num(&mut shape, h);
        shape.push_str("\" rx=\"");
        shape.push_str(RADIUS);
        shape.push('"');
        let mut name_attrs = String::from(" class=\"ondine-label\"");
        name_attrs.push_str(text);
        let mut name = label_svg(
            &view.label,
            0.0,
            if view.rows.is_empty() {
                0.0
            } else {
                y0 + view.head_h / 2.0
            },
            &name_attrs,
            None,
        );
        let display = entities::decode(if node.alias.is_empty() {
            node.label
        } else {
            node.alias
        })
        .into_owned();
        let mut spaced = classes.clone();
        spaced.push(' ');
        if spaced.contains(" internal-link ") && !name.is_empty() && url.links != Some(false) {
            // The name ends up in an href, so it gets the same check as a URL.
            let safe = url::link_url(&url::sanitize_url(&display), url).is_some();
            let target = esc(&display).into_owned();
            let mut anchor = String::from("<a class=\"internal-link\"");
            if safe {
                anchor.push_str(" href=\"");
                anchor.push_str(&target);
                anchor.push('"');
                anchor.push_str(&url::rel_attr(url));
            }
            anchor.push_str(" data-href=\"");
            anchor.push_str(&target);
            anchor.push_str("\">");
            anchor.push_str(&name);
            anchor.push_str("</a>");
            name = anchor;
            links.push(LinkInfo {
                href: display,
                id: node.label.to_owned(),
                internal: true,
            });
        }

        let mut body = String::from("<rect ");
        body.push_str(&shape);
        body.push_str(BOX_ATTRS);
        body.push_str(&view.style.shape);
        body.push_str("/>");
        if view.rows.is_empty() {
            body.push_str(&name);
        } else {
            let row_line = js_round(view.row_size * 1.5);
            let line_color = match view.style.stroke.as_deref() {
                Some(stroke) if !stroke.is_empty() => esc(stroke).into_owned(),
                _ => String::from("var(--_b)"),
            };
            let mut stripes = String::new();
            let mut cells = String::new();
            let mut y = y0 + view.head_h;
            let count = view.rows.len();
            for (r, row) in view.rows.iter().enumerate() {
                if r % 2 == 0 {
                    let mut span = String::from("x=\"");
                    push_num(&mut span, x0);
                    span.push_str("\" width=\"");
                    push_num(&mut span, w);
                    span.push('"');
                    stripes.push_str("<rect ");
                    stripes.push_str(&span);
                    stripes.push_str(" y=\"");
                    push_num(&mut stripes, y);
                    stripes.push_str("\" height=\"");
                    push_num(&mut stripes, row.h);
                    if r == count - 1 {
                        // The last row follows the box's rounded bottom corners: a rounded rect,
                        // squared off on top.
                        stripes.push_str("\" rx=\"");
                        stripes.push_str(RADIUS);
                        stripes.push_str("\"/><rect ");
                        stripes.push_str(&span);
                        stripes.push_str(" y=\"");
                        push_num(&mut stripes, y);
                        stripes.push_str("\" height=\"");
                        push_num(&mut stripes, row.h / 2.0);
                    }
                    stripes.push_str("\"/>");
                }
                let baseline = y + ROW_PAD / 2.0 + row_line / 2.0 + view.row_size * 0.35;
                let [r#type, attribute, keys, comment] = &row.cells;
                let col = |k: usize| x0 + view.cols.get(k).copied().unwrap_or(0.0);
                let mut type_attrs = String::from(" class=\"ondine-er-type\" fill=\"var(--_m)\"");
                type_attrs.push_str(text);
                cells.push_str(&text_left(r#type, col(0), baseline, &type_attrs));
                let mut name_attrs = String::from(" class=\"ondine-er-name\"");
                name_attrs.push_str(text);
                cells.push_str(&text_left(attribute, col(1), baseline, &name_attrs));
                let mut key_attrs = String::from(" class=\"ondine-er-keys\" font-size=\"");
                push_num(&mut key_attrs, view.key_size);
                key_attrs.push_str("\" fill=\"var(--_m)\"");
                key_attrs.push_str(text);
                cells.push_str(&text_left(keys, col(2), baseline, &key_attrs));
                let mut comment_attrs =
                    String::from(" class=\"ondine-er-comment\" fill=\"var(--_m)\"");
                comment_attrs.push_str(text);
                cells.push_str(&text_left(comment, col(3), baseline, &comment_attrs));
                y += row.h;
            }
            // Over a fill the diagram chose, the shading is a tint of that fill. The opacity is
            // on the group because the two parts of the last row overlap.
            body.push_str("<g class=\"ondine-er-stripes\" fill=\"var(--_bg)\"");
            if view.style.shape.contains("fill:") {
                body.push_str(" opacity=\"0.6\"");
            }
            body.push('>');
            body.push_str(&stripes);
            body.push_str("</g><path class=\"ondine-er-divider\" d=\"M");
            push_num(&mut body, x0);
            body.push(',');
            push_num(&mut body, y0 + view.head_h);
            body.push('H');
            push_num(&mut body, x0 + w);
            body.push_str("\" stroke=\"");
            body.push_str(&line_color);
            body.push_str("\"/><rect ");
            body.push_str(&shape);
            body.push_str(" fill=\"none\" stroke=\"var(--_b)\"");
            body.push_str(&view.style.line);
            body.push_str("/>");
            body.push_str(&name);
            body.push_str("<g class=\"ondine-er-attributes\" font-size=\"");
            push_num(&mut body, view.row_size);
            body.push_str("\">");
            body.push_str(&cells);
            body.push_str("</g>");
        }
        nodes_out.push_str("<g class=\"ondine-node ondine-entity");
        nodes_out.push_str(&classes);
        nodes_out.push_str("\" data-id=\"");
        nodes_out.push_str(&esc_text(node.label));
        nodes_out.push_str("\" transform=\"translate(");
        push_num(&mut nodes_out, c.x);
        nodes_out.push(',');
        push_num(&mut nodes_out, c.y);
        nodes_out.push_str(")\">");
        nodes_out.push_str(&body);
        nodes_out.push_str("</g>");
    }

    // Self-relationships share the side they loop out of, squeezed together if it is too short
    // for them.
    for (i, own) in &loops_of {
        let Some(view) = views.get(*i) else { continue };
        let total: f64 = own
            .iter()
            .filter_map(|&k| loops.get(k))
            .map(|item| item.band)
            .sum();
        let side = if view.below { view.w } else { view.h };
        let scale = ((side - 8.0).max(0.0) / total).min(1.0);
        let mut at = (-total * scale) / 2.0;
        for &k in own {
            if let Some(item) = loops.get_mut(k) {
                item.band *= scale;
                item.at = at + item.band / 2.0;
                at += item.band;
            }
        }
    }

    let mut edges_out = String::new();
    let mut labels_out = String::new();
    for (i, ce) in cedges.iter_mut().enumerate() {
        let (Some(edge), Some(label)) = (drawn.get(i), edge_labels.get(i)) else {
            continue;
        };
        let (d, start, end) =
            if let Some(item) = loop_of.get(i).copied().flatten().and_then(|k| loops.get(k)) {
                let (Some(c), Some(view)) = (cnodes.get(ce.src), views.get(ce.src)) else {
                    continue;
                };
                // `Math.max(Math.min(...))`, which a band too narrow for 1 still leaves at 1.
                #[expect(
                    clippy::manual_clamp,
                    reason = "the bounds may cross, which clamp rejects"
                )]
                let spread = (item.band / 2.0 - 6.0).min(13.0).max(1.0);
                let far = MARKER_LENGTH + LOOP * 0.75 + 6.0;
                let mut d = String::from("M");
                if view.below {
                    let x = c.x + item.at;
                    let y = c.y + view.h / 2.0;
                    let turn = y + MARKER_LENGTH;
                    let out = y + MARKER_LENGTH + LOOP;
                    push_num(&mut d, x - spread);
                    d.push(',');
                    push_num(&mut d, y);
                    d.push('V');
                    push_num(&mut d, turn);
                    d.push('C');
                    push_num(&mut d, x - spread);
                    d.push(',');
                    push_num(&mut d, out);
                    d.push(' ');
                    push_num(&mut d, x + spread);
                    d.push(',');
                    push_num(&mut d, out);
                    d.push(' ');
                    push_num(&mut d, x + spread);
                    d.push(',');
                    push_num(&mut d, turn);
                    d.push('V');
                    push_num(&mut d, y);
                    ce.label_x = x;
                    ce.label_y = y + far + label.height / 2.0;
                    (
                        d,
                        er_marker(edge.arrow_type_start, x - spread, y, 0.0, -1.0),
                        er_marker(edge.arrow_type_end, x + spread, y, 0.0, -1.0),
                    )
                } else {
                    let x = c.x + view.w / 2.0;
                    let y = c.y + item.at;
                    let turn = x + MARKER_LENGTH;
                    let out = x + MARKER_LENGTH + LOOP;
                    push_num(&mut d, x);
                    d.push(',');
                    push_num(&mut d, y - spread);
                    d.push('H');
                    push_num(&mut d, turn);
                    d.push('C');
                    push_num(&mut d, out);
                    d.push(',');
                    push_num(&mut d, y - spread);
                    d.push(' ');
                    push_num(&mut d, out);
                    d.push(',');
                    push_num(&mut d, y + spread);
                    d.push(' ');
                    push_num(&mut d, turn);
                    d.push(',');
                    push_num(&mut d, y + spread);
                    d.push('H');
                    push_num(&mut d, x);
                    ce.label_x = x + far + label.width / 2.0;
                    ce.label_y = y;
                    (
                        d,
                        er_marker(edge.arrow_type_start, x, y - spread, -1.0, 0.0),
                        er_marker(edge.arrow_type_end, x, y + spread, -1.0, 0.0),
                    )
                }
            } else {
                let mut route = straighten(&ce.route);
                if route.len() < 6 {
                    continue;
                }
                let last = route.len() - 3;
                if let (Some(view), Some(c)) = (views.get(ce.src), cnodes.get(ce.src)) {
                    inset(&mut route, 0, view, c);
                }
                if let (Some(view), Some(c)) = (views.get(ce.dst), cnodes.get(ce.dst)) {
                    inset(&mut route, last, view, c);
                }
                // A curve stops short of each box so that the marker sits on a straight run of line.
                let value = |k: usize| route.get(k).copied().unwrap_or(f64::NAN);
                let dx = (value(last) - value(0)).abs();
                let dy = (value(last + 1) - value(1)).abs();
                let direct = route.len() == 6
                    && (dx < 0.01 || dy < 0.01 || dx.hypot(dy) <= 3.0 * MARKER_LENGTH);
                let stub = if direct { 0.0 } else { MARKER_LENGTH };
                let path = route_path(&route, None, stub, stub, None, None);
                let d = if stub > 0.0 {
                    let mut d = String::from("M");
                    push_num(&mut d, path.sx);
                    d.push(',');
                    push_num(&mut d, path.sy);
                    d.push('L');
                    d.push_str(path.d.get(1..).unwrap_or(""));
                    d.push('L');
                    push_num(&mut d, path.ex);
                    d.push(',');
                    push_num(&mut d, path.ey);
                    d
                } else {
                    path.d.clone()
                };
                (
                    d,
                    er_marker(edge.arrow_type_start, path.sx, path.sy, path.sdx, path.sdy),
                    er_marker(edge.arrow_type_end, path.ex, path.ey, path.edx, path.edy),
                )
            };
        edges_out.push_str("<g class=\"ondine-edge ondine-er-");
        edges_out.push_str(if edge.dashed {
            "non-identifying"
        } else {
            "identifying"
        });
        edges_out.push_str("\" data-id=\"");
        edges_out.push_str(&esc_text(&edge.id));
        edges_out.push_str("\"><path d=\"");
        edges_out.push_str(&d);
        edges_out.push('"');
        if edge.dashed {
            edges_out.push_str(" stroke-dasharray=\"4 3\"");
        }
        edges_out.push_str("/>");
        edges_out.push_str(&start);
        edges_out.push_str(&end);
        edges_out.push_str("</g>");
        if label.width > 0.0 {
            labels_out.push_str(&edge_label_svg(
                &edge.id,
                &[],
                label,
                ce.label_x,
                ce.label_y,
                "",
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
        push_num(&mut content, small_size);
        content.push_str("\">");
        content.push_str(&labels_out);
        content.push_str("</g>");
    }
    content.push_str("<g class=\"ondine-nodes\">");
    content.push_str(&nodes_out);
    content.push_str("</g>");

    let accessible = Accessible {
        acc_descr: db.acc_descr.as_deref(),
        acc_title: db.acc_title.as_deref(),
    };
    let svg = svg_document("er", width, height, size, options, accessible, &content);
    Rendered {
        height,
        links,
        svg,
        width,
    }
}

/// Drops the middle points of straight runs.
fn straighten(route: &[f64]) -> Vec<f64> {
    let mut out: Vec<f64> = route.iter().take(3).copied().collect();
    let len = route.len();
    let mut k = 3;
    while k < len {
        let prev = out.len().saturating_sub(3);
        let value = |at: usize| route.get(at).copied().unwrap_or(f64::NAN);
        let flat = |axis: usize| {
            (out.get(prev + axis).copied().unwrap_or(f64::NAN) - value(k + axis)).abs() < 0.01
                && (value(k + axis) - value(k + 3 + axis)).abs() < 0.01
        };
        if !(k + 3 < len && (flat(0) || flat(1))) {
            out.extend(route.iter().skip(k).take(3));
        }
        k += 3;
    }
    out
}

/// JavaScript's order for `a - b` where the difference is a number: equal values, including
/// both zeros, compare equal.
#[expect(
    clippy::float_cmp,
    reason = "equal values compare equal, as their difference is 0"
)]
fn compare(a: f64, b: f64) -> Ordering {
    if a == b {
        Ordering::Equal
    } else {
        a.total_cmp(&b)
    }
}

/// The layout brings every edge that ends on one side of a box to the middle of that side, where
/// their markers would cover each other. This spaces them along the side, in the order they fan
/// out.
fn spread_ends(cedges: &mut [CEdge], cnodes: &[CNode], views: &[NodeView<'_>]) {
    /// An end of an edge on a side of a node.
    struct End {
        at: usize,
        edge: usize,
        toward: f64,
    }

    let mut keys: BTreeMap<usize, usize> = BTreeMap::new();
    let mut sides: Vec<(usize, Vec<End>)> = Vec::new();
    let mut add = |node: usize, edge: usize, route: &[f64], at: usize, next: usize| {
        let Some(c) = cnodes.get(node) else { return };
        let dx = route.get(at).copied().unwrap_or(f64::NAN) - c.x;
        let dy = route.get(at + 1).copied().unwrap_or(f64::NAN) - c.y;
        let side = if (dy.abs() - c.h / 2.0).abs() < 0.5 {
            if dy < 0.0 { 0 } else { 2 }
        } else if (dx.abs() - c.w / 2.0).abs() < 0.5 {
            if dx < 0.0 { 3 } else { 1 }
        } else {
            return;
        };
        let key = node * 4 + side;
        let end = End {
            at,
            edge,
            toward: route
                .get(next + usize::from(side & 1 == 1))
                .copied()
                .unwrap_or(f64::NAN),
        };
        if let Some(&k) = keys.get(&key) {
            if let Some((_, list)) = sides.get_mut(k) {
                list.push(end);
            }
        } else {
            keys.insert(key, sides.len());
            sides.push((key, vec![end]));
        }
    };
    for (i, e) in cedges.iter().enumerate() {
        let Some(last) = e.route.len().checked_sub(3) else {
            continue;
        };
        if e.src == e.dst || last < 3 {
            continue;
        }
        add(e.src, i, &e.route, 0, 3);
        add(e.dst, i, &e.route, last, last - 3);
    }
    for (key, ends) in &mut sides {
        if ends.len() < 2 {
            continue;
        }
        let node = *key >> 2;
        let vertical = *key & 1 == 1;
        let (Some(c), Some(view)) = (cnodes.get(node), views.get(node)) else {
            continue;
        };
        let (bw, bh) = if c.is_group {
            (c.w, c.h)
        } else {
            (view.w, view.h)
        };
        let length = if vertical { bh } else { bw };
        let step = PORT_GAP
            .min((length - 2.0 * PORT_MARGIN) / to_f64(ends.len() - 1))
            .max(0.0);
        let center = if vertical { c.y } else { c.x };
        ends.sort_by(|a, b| compare(a.toward, b.toward));
        let count = to_f64(ends.len());
        for (i, end) in ends.iter().enumerate() {
            if let Some(v) = cedges
                .get_mut(end.edge)
                .and_then(|e| e.route.get_mut(end.at + usize::from(vertical)))
            {
                *v = center + (to_f64(i) - (count - 1.0) / 2.0) * step;
            }
        }
    }
}

/// An entity with a self-relationship is laid out larger than it is drawn. This moves a route
/// end from the side of that larger box onto the entity's own side.
#[expect(clippy::float_cmp, reason = "Pele compares the sizes exactly")]
fn inset(route: &mut [f64], at: usize, view: &NodeView<'_>, c: &CNode) {
    if c.is_group {
        return;
    }
    let dx = route.get(at).copied().unwrap_or(f64::NAN) - c.x;
    let dy = route.get(at + 1).copied().unwrap_or(f64::NAN) - c.y;
    if c.w != view.w
        && (dx.abs() - c.w / 2.0).abs() < 0.5
        && let Some(x) = route.get_mut(at)
    {
        *x -= js_sign(dx) * (c.w - view.w) / 2.0;
    }
    if c.h != view.h
        && (dy.abs() - c.h / 2.0).abs() < 0.5
        && let Some(y) = route.get_mut(at + 1)
    {
        *y -= js_sign(dy) * (c.h - view.h) / 2.0;
    }
}

/// JavaScript's `Math.sign`: zero for zero.
fn js_sign(value: f64) -> f64 {
    if value == 0.0 || value.is_nan() {
        value
    } else {
        value.signum()
    }
}
