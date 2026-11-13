//! Lays out and draws an agentflow. Port of Pele's `src/diagrams/agentflow/render.ts`.

use std::collections::BTreeMap;

use super::color_slots::{KIND_COUNT, kind_slot};
use super::db::AgentflowDb;
use super::graph::{AgentGraph, GraphEdge, GraphNode};
use super::types::LabelType;
use crate::diagrams::Rendered;
use crate::diagrams::common::fit_width::{runs_across, turn_to_fit};
use crate::diagrams::flowchart::shapes::canonical_shape;
use crate::layout::compound::{CEdge, CNode, Dir, cnode, compound_layout, direction, shift_layout};
use crate::layout::layered::LayeredOptions;
use crate::svg::builder::{esc, esc_text, label_svg, push_num};
use crate::svg::cluster::{Crossings, cluster_title_x, mark_crossings};
use crate::svg::edges::{EdgePath, edge_label_svg, loop_path, marker, marker_trim, route_path};
use crate::svg::root::{Accessible, RenderOptions, svg_document};
use crate::svg::shapes::{PAD_X, PAD_Y, draw_shape, inset_route, shape_size};
use crate::svg::theme::{RADIUS, ResolvedStyle, class_names, resolve_style, series_color};
use crate::text::entities;
use crate::text::label::{Label, layout_label};
use crate::text::measure::{BOLD, ITALIC};
use crate::util::url::{self, LinkInfo, UrlOptions};
use crate::yaml::Value;

/// The number of series colors, which is the palette Mermaid's slot rules are applied to.
pub(crate) const PALETTE: u32 = 8;

const NODE_SEP: f64 = 40.0;
const EDGE_SEP: f64 = 16.0;
const RANK_SEP: f64 = 48.0;
const GROUP_PAD: f64 = 20.0;
const LOOP: f64 = 26.0;
const DOTS: f64 = 14.0;
const COLLAPSED: &str = "collapsed";

/// The side a flow runs toward, with sides numbered clockwise from the top.
fn flow_end(dir: Dir) -> u8 {
    match dir {
        Dir::Tb => 2,
        Dir::Rl => 3,
        Dir::Bt => 0,
        Dir::Lr => 1,
    }
}

/// The value under `key` of a YAML mapping.
fn get<'v>(value: &'v Value, key: &str) -> Option<&'v Value> {
    match value {
        Value::Object(entries) => entries.iter().find(|(own, _)| own == key).map(|(_, v)| v),
        _ => None,
    }
}

fn number_option(config: &Value, key: &str, fallback: f64) -> f64 {
    match get(config, "agentflow").and_then(|section| get(section, key)) {
        Some(Value::Number(value)) if *value > 0.0 => value.min(2000.0),
        _ => fallback,
    }
}

/// A palette color as an outline with a light wash of the same color inside. A fill the author
/// set replaces the wash and stays opaque.
fn tint(slot: u32, wash: &str, style: &ResolvedStyle) -> String {
    let color = series_color(i64::from(slot));
    let mut out = String::from(" fill=\"");
    out.push_str(&color);
    out.push('"');
    // `/[";]fill:/`.
    if !(style.shape.contains("\"fill:") || style.shape.contains(";fill:")) {
        out.push_str(" fill-opacity=\"");
        out.push_str(wash);
        out.push('"');
    }
    out.push_str(" stroke=\"");
    out.push_str(&color);
    out.push('"');
    out
}

/// What a node is drawn as, beside its layout box.
struct NodeView<'g> {
    dy: f64,
    h: f64,
    label: Label,
    node: &'g GraphNode,
    shape: &'static str,
    style: ResolvedStyle,
    w: f64,
}

/// An edge between a node and a flow around it.
struct Held<'g> {
    dst: usize,
    edge: &'g GraphEdge,
    label: Label,
    src: usize,
}

/// Draws an agentflow, turning one that runs across to run down when it does not fit the host's
/// width.
pub(crate) fn render(
    db: &AgentflowDb,
    graph: &AgentGraph,
    config: &Value,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
) -> Rendered {
    if !runs_across(db.direction.as_deref()) {
        return draw(db, graph, config, options, url, false);
    }
    turn_to_fit(options, |down| draw(db, graph, config, options, url, down))
}

#[expect(clippy::cast_precision_loss, reason = "counts stay far below 2^52")]
fn to_f64(value: usize) -> f64 {
    value as f64
}

/// Removes a leading `default` class and one following whitespace character, as
/// `/^default\s?/` does.
fn strip_default(classes: &str) -> &str {
    let Some(rest) = classes.strip_prefix("default") else {
        return classes;
    };
    match rest.chars().next() {
        Some(c) if crate::util::chars::is_space(u32::from(c)) => &rest[c.len_utf8()..],
        _ => rest,
    }
}

/// Removes Mermaid's default edge classes, as Pele's global replace does.
fn strip_edge_classes(classes: &str) -> String {
    let mut out = String::from(classes);
    for name in [
        "edge-thickness-normal",
        "edge-pattern-solid",
        "flowchart-link",
    ] {
        while let Some(at) = out.find(name) {
            out.replace_range(at..at + name.len(), "");
        }
    }
    out
}

/// `turned` draws a diagram that runs across as one that runs down.
#[expect(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    reason = "mirrors Pele's draw, one pass per drawing stage, with its names"
)]
fn draw(
    db: &AgentflowDb,
    graph: &AgentGraph,
    config: &Value,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
    turned: bool,
) -> Rendered {
    let size = options.font_size.unwrap_or(16.0);
    let edge_size = crate::svg::builder::js_round(size * 0.875);
    let wrap_width = number_option(config, "wrappingWidth", 120.0);
    let min_width = number_option(config, "minNodeWidth", 120.0);
    let pad = options.padding.unwrap_or(8.0);

    let mut index: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, node) in graph.nodes.iter().enumerate() {
        index.insert(&node.id, i);
    }

    let mut views: Vec<NodeView<'_>> = Vec::with_capacity(graph.nodes.len());
    let mut cnodes: Vec<CNode> = Vec::with_capacity(graph.nodes.len());
    for node in &graph.nodes {
        let style = if node.css_styles.is_empty() {
            resolve_style(&node.css_compiled_styles)
        } else {
            let mut all = node.css_compiled_styles.clone();
            all.extend(node.css_styles.iter().cloned());
            resolve_style(&all)
        };
        let text_style =
            (if style.bold { BOLD } else { 0 }) | (if style.italic { ITALIC } else { 0 });
        let markdown = node.label_type == LabelType::Markdown;
        let font_size = style.font_size.unwrap_or(size);
        let label_of = |width: f64| {
            layout_label(
                node.label.as_deref(),
                markdown,
                font_size,
                width,
                text_style,
                None,
            )
        };

        let (view, mut c) = if node.is_group {
            let label = label_of(4000.0);
            let mut c = cnode(0.0, 0.0, -1);
            c.is_group = true;
            c.dir = node
                .dir
                .as_deref()
                .filter(|dir| !dir.is_empty())
                .map(|dir| direction(Some(dir)));
            c.pad_x = GROUP_PAD;
            c.pad_top = if label.height > 0.0 {
                label.height + 16.0
            } else {
                GROUP_PAD
            };
            c.pad_bottom = GROUP_PAD;
            c.min_w = label.width + 2.0 * GROUP_PAD - 12.0;
            let view = NodeView {
                dy: 0.0,
                h: 0.0,
                label,
                node,
                shape: "group",
                style,
                w: 0.0,
            };
            (view, c)
        } else if node.shape == "collapsedGroup" {
            let label = label_of(wrap_width);
            let w = label.width.max(min_width) + 2.0 * PAD_X;
            let h = label.height + 2.0 * PAD_Y + DOTS;
            let view = NodeView {
                dy: -DOTS / 2.0,
                h,
                label,
                node,
                shape: COLLAPSED,
                style,
                w,
            };
            (view, cnode(w, h, -1))
        } else {
            let shape = canonical_shape(Some(&node.shape));
            let label = label_of(wrap_width);
            // A diamond grows twice as fast as its label, so it keeps the label's own width.
            let width = if shape == "diam" {
                label.width
            } else {
                label.width.max(min_width)
            };
            let s = shape_size(shape, width, label.height);
            let view = NodeView {
                dy: s.dy,
                h: s.h,
                label,
                node,
                shape,
                style,
                w: s.w,
            };
            (view, cnode(s.w, s.h, -1))
        };
        c.seq = to_f64(views.len());
        views.push(view);
        cnodes.push(c);
    }

    for (i, node) in graph.nodes.iter().enumerate() {
        let parent = node
            .parent_id
            .as_deref()
            .and_then(|id| index.get(id))
            .map_or(-1, |&p| i32::try_from(p).unwrap_or(-1));
        if let Some(c) = cnodes.get_mut(i) {
            c.parent = parent;
        }
    }

    let root_dir = if turned {
        Dir::Tb
    } else {
        direction(db.direction.as_deref())
    };
    let parent_of = |cnodes: &[CNode], i: usize| -> Option<usize> {
        cnodes.get(i).and_then(|c| usize::try_from(c.parent).ok())
    };
    // The direction of the level a node sits in.
    let flow_of = |cnodes: &[CNode], i: usize| -> Dir {
        let mut dir: Option<Dir> = None;
        let mut p = parent_of(cnodes, i);
        let mut hops = 0;
        while let Some(at) = p {
            if dir.is_some() || hops >= 10_000 {
                break;
            }
            let Some(parent) = cnodes.get(at) else { break };
            dir = parent.dir;
            p = usize::try_from(parent.parent).ok();
            hops += 1;
        }
        dir.unwrap_or(root_dir)
    };
    let flows_sideways =
        |cnodes: &[CNode], i: usize| matches!(flow_of(cnodes, i), Dir::Lr | Dir::Rl);

    let n = cnodes.len();
    let mut depth: Vec<i64> = vec![-1; n];
    for i in 0..n {
        let mut chain: Vec<usize> = Vec::new();
        let mut j = Some(i);
        while let Some(at) = j {
            if depth.get(at).is_none_or(|&d| d >= 0) || chain.len() > n {
                break;
            }
            chain.push(at);
            j = parent_of(&cnodes, at);
        }
        let first = j.and_then(|at| depth.get(at)).map_or(0, |&d| d + 1);
        for (d, &k) in (first..).zip(chain.iter().rev()) {
            if let Some(slot) = depth.get_mut(k) {
                *slot = d;
            }
        }
    }
    let contains = |cnodes: &[CNode], group: usize, i: usize| -> bool {
        let (Some(&from), Some(&to)) = (depth.get(i), depth.get(group)) else {
            return false;
        };
        let mut j = Some(i);
        let mut d = from;
        while d > to {
            j = j.and_then(|at| parent_of(cnodes, at));
            d -= 1;
        }
        j == Some(group) && i != group
    };

    let mut cedges: Vec<CEdge> = Vec::with_capacity(graph.edges.len());
    let mut edge_labels: Vec<Label> = Vec::with_capacity(graph.edges.len());
    let mut drawn: Vec<&GraphEdge> = Vec::with_capacity(graph.edges.len());
    // Edges between a node and a flow around it. The layout has no place for them, so they are
    // drawn afterwards, between the node and the flow's border.
    let mut held: Vec<Held<'_>> = Vec::new();
    for edge in &graph.edges {
        let (Some(&src), Some(&dst)) =
            (index.get(edge.start.as_str()), index.get(edge.end.as_str()))
        else {
            continue;
        };
        let markdown = edge.label_type == LabelType::Markdown;
        let label = layout_label(Some(&edge.label), markdown, edge_size, wrap_width, 0, None);
        if contains(&cnodes, src, dst) || contains(&cnodes, dst, src) {
            held.push(Held {
                dst,
                edge,
                label,
                src,
            });
            continue;
        }

        if src == dst {
            // A self-loop sits beside the node, across the flow, and the node's box grows to hold
            // it.
            let sideways = flows_sideways(&cnodes, src);
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
            minlen: edge.minlen.and_then(|n| u32::try_from(n).ok()).unwrap_or(1),
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

    let title = layout_label(db.title.as_deref(), false, size, 4000.0, BOLD, None);
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
    let mut links: Vec<LinkInfo> = Vec::new();

    // An edge between a node and a flow around it runs straight between the node and the flow's
    // border: out along the flow, or in against it. If another node is in the way it takes a
    // side instead, and failing that the far end.
    let mut held_paths: Vec<EdgePath> = Vec::with_capacity(held.len());
    let mut held_labels: Vec<(f64, f64)> = Vec::with_capacity(held.len());
    let mut held_routes: Vec<[f64; 6]> = Vec::with_capacity(held.len());
    let careful = to_f64(held.len()) * to_f64(n) < 2e6;
    for h in &held {
        let out = contains(&cnodes, h.dst, h.src);
        let outer = if out { h.dst } else { h.src };
        let i = if out { h.src } else { h.dst };
        let (Some(g), Some(nd), Some(view)) = (cnodes.get(outer), cnodes.get(i), views.get(i))
        else {
            continue;
        };
        let end = flow_end(g.dir.unwrap_or_else(|| flow_of(&cnodes, outer)));
        let first = if out { end } else { (end + 2) & 3 };
        let toward = |side: u8| -> [f64; 6] {
            let upright = side & 1 == 0;
            let sign = if side == 0 || side == 3 { -1.0 } else { 1.0 };
            let nx = if upright {
                nd.x
            } else {
                nd.x + sign * nd.w / 2.0
            };
            let ny = if upright {
                nd.y + sign * nd.h / 2.0
            } else {
                nd.y
            };
            let bx = if upright {
                nd.x
            } else {
                g.x + sign * g.w / 2.0
            };
            let by = if upright {
                g.y + sign * g.h / 2.0
            } else {
                nd.y
            };
            let axis = if upright { 0.0 } else { 1.0 };
            if out {
                [nx, ny, axis, bx, by, axis]
            } else {
                [bx, by, axis, nx, ny, axis]
            }
        };
        let blocked = |r: &[f64; 6]| -> bool {
            cnodes.iter().enumerate().any(|(k, c)| {
                k != i
                    && !c.is_group
                    && c.x + c.w / 2.0 > r[0].min(r[3]) - 1.0
                    && c.x - c.w / 2.0 < r[0].max(r[3]) + 1.0
                    && c.y + c.h / 2.0 > r[1].min(r[4]) - 1.0
                    && c.y - c.h / 2.0 < r[1].max(r[4]) + 1.0
            })
        };
        let mut route = toward(first);
        if careful && blocked(&route) {
            for side in [(first + 1) & 3, (first + 3) & 3, (first + 2) & 3] {
                let other = toward(side);
                if !blocked(&other) {
                    route = other;
                    break;
                }
            }
        }
        held_routes.push(route);
        held_labels.push((
            f64::midpoint(route[0], route[3]),
            f64::midpoint(route[1], route[4]),
        ));
        let mut drawn_route = route.to_vec();
        inset_route(
            &mut drawn_route,
            if out { 0 } else { 3 },
            view.shape,
            view.w,
            view.h,
            nd,
        );
        held_paths.push(route_path(
            &drawn_route,
            Some("linear"),
            0.0,
            marker_trim(h.edge.arrow_type_end),
            None,
            None,
        ));
    }

    let mut crossings = Crossings::new();
    if cnodes.iter().any(|c| c.is_group) {
        for e in &cedges {
            mark_crossings(&mut crossings, &e.route);
        }
        for route in &held_routes {
            mark_crossings(&mut crossings, route);
        }
    }

    let mut clusters = String::new();
    let mut nodes_out = String::new();
    for (i, view) in views.iter().enumerate() {
        let Some(c) = cnodes.get(i) else { continue };
        let node = view.node;
        let classes = class_names(strip_default(&node.css_classes));
        let id = esc_text(&node.id);
        let slot = node.color_index.unwrap_or(KIND_COUNT);

        if c.is_group {
            let title_x = cluster_title_x(c, view.label.width, &crossings, GROUP_PAD);
            let mut title_attrs =
                String::from(" class=\"ondine-cluster-label\" fill=\"var(--_m)\"");
            title_attrs.push_str(&view.style.text);
            clusters.push_str("<g class=\"ondine-cluster ondine-flow");
            clusters.push_str(&classes);
            clusters.push_str("\" data-id=\"");
            clusters.push_str(&id);
            clusters.push_str("\"><rect x=\"");
            push_num(&mut clusters, c.x - c.w / 2.0);
            clusters.push_str("\" y=\"");
            push_num(&mut clusters, c.y - c.h / 2.0);
            clusters.push_str("\" width=\"");
            push_num(&mut clusters, c.w);
            clusters.push_str("\" height=\"");
            push_num(&mut clusters, c.h);
            clusters.push_str("\" rx=\"");
            clusters.push_str(RADIUS);
            clusters.push('"');
            clusters.push_str(&tint(slot, "0.06", &view.style));
            clusters.push_str(&view.style.shape);
            clusters.push_str("/>");
            clusters.push_str(&label_svg(
                &view.label,
                title_x,
                c.y - c.h / 2.0 + 8.0 + view.label.height / 2.0,
                &title_attrs,
                None,
            ));
            clusters.push_str("</g>");
            continue;
        }

        let (w, h) = (view.w, view.h);
        let mut kind = String::new();
        let shape = if view.shape == COLLAPSED {
            collapsed(view, w, h, slot)
        } else {
            let name = node.kind.unwrap_or("task");
            let slot = kind_slot(name).unwrap_or(1);
            kind.push_str(" ondine-kind-");
            kind.push_str(name);
            let mut attrs = tint(slot, "0.14", &view.style);
            attrs.push_str(&view.style.shape);
            let mut line = String::from(" stroke=\"");
            line.push_str(&series_color(i64::from(slot)));
            line.push('"');
            line.push_str(&view.style.line);
            draw_shape(view.shape, w, h, &attrs, &line)
        };

        let mut label_attrs = String::from(" class=\"ondine-label\"");
        label_attrs.push_str(&view.style.text);
        let mut text = label_svg(&view.label, 0.0, view.dy, &label_attrs, None);

        let mut spaced = classes.clone();
        spaced.push(' ');
        let internal = spaced.contains(" internal-link ");
        if let Some(label) = node
            .label
            .as_deref()
            .filter(|label| internal && !label.is_empty())
            && url.links != Some(false)
        {
            // The label is a note name, but it still ends up in an href, so it gets the same
            // check as a URL.
            let name = entities::decode(label).into_owned();
            let safe = url::link_url(&url::sanitize_url(&name), url).is_some();
            let target = esc(&name).into_owned();
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
            anchor.push_str(&text);
            anchor.push_str("</a>");
            text = anchor;
            links.push(LinkInfo {
                href: name,
                id: node.id.clone(),
                internal: true,
            });
        }

        let mut body = String::new();
        if let Some(tooltip) = node
            .tooltip
            .as_deref()
            .filter(|tooltip| !tooltip.is_empty())
        {
            body.push_str("<title>");
            body.push_str(&esc_text(tooltip));
            body.push_str("</title>");
        }
        body.push_str(&shape);
        body.push_str(&text);
        if let Some(link) = node.link.as_deref().filter(|link| !link.is_empty()) {
            let mut attrs = String::new();
            if let Some(target) = node
                .link_target
                .as_deref()
                .filter(|target| !target.is_empty())
            {
                attrs.push_str(" target=\"");
                attrs.push_str(&esc(target));
                attrs.push('"');
            }
            body = url::linked(&body, link, &node.id, &mut links, url, &attrs);
        }

        nodes_out.push_str("<g class=\"ondine-node ondine-shape-");
        nodes_out.push_str(view.shape);
        nodes_out.push_str(&kind);
        nodes_out.push_str(&classes);
        nodes_out.push_str("\" data-id=\"");
        nodes_out.push_str(&id);
        nodes_out.push_str("\" transform=\"translate(");
        push_num(&mut nodes_out, c.x);
        nodes_out.push(',');
        push_num(&mut nodes_out, c.y);
        nodes_out.push_str(")\">");
        nodes_out.push_str(&body);
        nodes_out.push_str("</g>");
    }

    let mut edges_out = String::new();
    let mut labels_out = String::new();
    let mut put = |edge: &GraphEdge, label: &Label, path: &EdgePath, x: f64, y: f64| {
        let style = if edge.style.is_empty() && edge.css_compiled_styles.is_empty() {
            None
        } else {
            let mut all = edge.css_compiled_styles.clone();
            all.extend(edge.style.iter().cloned());
            Some(resolve_style(&all))
        };
        let end_type = edge.arrow_type_end;
        let color = match style.as_ref().and_then(|style| style.stroke.as_deref()) {
            Some(stroke) if !stroke.is_empty() => esc(stroke).into_owned(),
            _ => String::from("var(--_l)"),
        };
        let dotted = edge.thickness == "dotted";
        let kind = if dotted {
            "reference"
        } else if end_type == "arrow_cross" {
            "failure"
        } else {
            "sequence"
        };
        let classes = class_names(&strip_edge_classes(GraphEdge::CLASSES));

        edges_out.push_str("<g class=\"ondine-edge ondine-edge-");
        edges_out.push_str(kind);
        edges_out.push_str(&classes);
        edges_out.push_str("\" data-id=\"");
        edges_out.push_str(&esc_text(&edge.id));
        edges_out.push('"');
        if let Some(style) = &style {
            edges_out.push_str(&style.line);
        }
        edges_out.push_str("><path d=\"");
        edges_out.push_str(&path.d);
        edges_out.push('"');
        if dotted {
            edges_out.push_str(" stroke-dasharray=\"3 4\"");
        }
        edges_out.push_str("/>");
        edges_out.push_str(&marker(
            end_type, path.ex, path.ey, path.edx, path.edy, &color,
        ));
        edges_out.push_str("</g>");

        if label.width > 0.0 {
            let attrs = style.as_ref().map_or("", |style| style.text.as_str());
            labels_out.push_str(&edge_label_svg(&edge.id, &[], label, x, y, attrs, None));
        }
    };

    let mut loop_seen = vec![0usize; views.len()];
    for i in 0..cedges.len() {
        let (Some(edge), Some(label)) = (drawn.get(i), edge_labels.get(i)) else {
            continue;
        };
        let end_type = edge.arrow_type_end;
        let Some(ce) = cedges.get(i) else { continue };
        let (src, dst) = (ce.src, ce.dst);
        let path = if src == dst {
            let k = loop_seen.get(src).copied().unwrap_or(0);
            if let Some(seen) = loop_seen.get_mut(src) {
                *seen += 1;
            }
            let sideways = flows_sideways(&cnodes, src);
            let (w, h) = views.get(src).map_or((0.0, 0.0), |view| (view.w, view.h));
            let c = cnodes
                .get(src)
                .cloned()
                .unwrap_or_else(|| cnode(0.0, 0.0, -1));
            let Some(ce) = cedges.get_mut(i) else {
                continue;
            };
            loop_path(
                &c,
                w,
                h,
                to_f64(k),
                sideways,
                LOOP,
                marker_trim(end_type),
                ce,
            )
        } else {
            let (Some(src_node), Some(dst_node), Some(src_view), Some(dst_view)) = (
                cnodes.get(src),
                cnodes.get(dst),
                views.get(src),
                views.get(dst),
            ) else {
                continue;
            };
            let mut route = ce.route.clone();
            let last = route.len().saturating_sub(3);
            inset_route(
                &mut route,
                0,
                src_view.shape,
                src_view.w,
                src_view.h,
                src_node,
            );
            inset_route(
                &mut route,
                last,
                dst_view.shape,
                dst_view.w,
                dst_view.h,
                dst_node,
            );
            let curve = match &edge.curve {
                Some(Value::String(curve)) => Some(curve.as_str()),
                _ => None,
            };
            route_path(&route, curve, 0.0, marker_trim(end_type), None, None)
        };
        let Some(ce) = cedges.get(i) else { continue };
        put(edge, label, &path, ce.label_x, ce.label_y);
    }
    for (k, h) in held.iter().enumerate() {
        if let (Some(path), Some(&(x, y))) = (held_paths.get(k), held_labels.get(k)) {
            put(h.edge, &h.label, path, x, y);
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
        push_num(&mut content, edge_size);
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
    let svg = svg_document(
        "agentflow",
        width,
        height,
        size,
        options,
        accessible,
        &content,
    );

    Rendered {
        height,
        links,
        svg,
        width,
    }
}

/// A collapsed flow: its box, a dashed rule, and three dots below the title.
fn collapsed(view: &NodeView<'_>, w: f64, h: f64, slot: u32) -> String {
    let color = series_color(i64::from(slot));
    let line = h / 2.0 - DOTS - 2.0;
    let dots = line + f64::midpoint(DOTS, 2.0);

    let mut out = String::from("<rect x=\"");
    push_num(&mut out, -w / 2.0);
    out.push_str("\" y=\"");
    push_num(&mut out, -h / 2.0);
    out.push_str("\" width=\"");
    push_num(&mut out, w);
    out.push_str("\" height=\"");
    push_num(&mut out, h);
    out.push_str("\" rx=\"");
    out.push_str(RADIUS);
    out.push('"');
    out.push_str(&tint(slot, "0.06", &view.style));
    out.push_str(&view.style.shape);
    out.push_str("/><path d=\"M");
    push_num(&mut out, -w / 2.0 + 10.0);
    out.push(',');
    push_num(&mut out, line);
    out.push('H');
    push_num(&mut out, w / 2.0 - 10.0);
    out.push_str("\" fill=\"none\" stroke=\"");
    out.push_str(&color);
    out.push_str("\" stroke-dasharray=\"3 3\"");
    out.push_str(&view.style.line);
    out.push_str("/><path class=\"ondine-collapsed-mark\" d=\"");
    for x in [-9.0, 0.0, 9.0] {
        dot(&mut out, x, dots);
    }
    out.push_str("\" fill=\"");
    out.push_str(&color);
    out.push_str("\" stroke=\"none\"/>");
    out
}

/// One dot of the collapsed mark.
fn dot(out: &mut String, x: f64, y: f64) {
    out.push('M');
    push_num(out, x - 1.75);
    out.push(',');
    push_num(out, y);
    out.push_str("a1.75,1.75 0 1 0 3.5,0a1.75,1.75 0 1 0 -3.5,0Z");
}
