//! Lays out and draws a flowchart, or a variant of one such as a swimlane diagram. Port of Pele's
//! `src/diagrams/flowchart/render.ts`.

use super::db::FlowDb;
use super::graph::{self, FlowGraph, GraphEdge, GraphNode};
use super::shapes::canonical_shape;
use crate::diagrams::Rendered;
use crate::diagrams::common::fit_width::{runs_across, tighten, title_room, turn_to_fit};
use crate::layout::compound::{
    CEdge, CNode, CompoundResult, Dir, cnode, compound_layout, direction, shift_layout,
};
use crate::layout::layered::LayeredOptions;
use crate::lines;
use crate::svg::builder::{esc, esc_text, label_svg, push_num};
use crate::svg::cluster::{
    Crossings, cluster_title_x, mark_crossings, struck_title, title_crossed,
};
use crate::svg::edges::{
    EdgePath, SideReach, edge_label_svg, loop_path, marker, marker_trim, route_path,
};
use crate::svg::root::{Accessible, RenderOptions, svg_document};
use crate::svg::shapes::{
    draw_shape, end_side, inset_route, shape_has_label, shape_size, shape_span,
};
use crate::svg::theme::{RADIUS, ResolvedStyle, class_names, resolve_style};
use crate::text::entities;
use crate::text::label::{Label, layout_label};
use crate::text::measure::{BOLD, ITALIC};
use crate::util::url::{self, LinkInfo, UrlOptions};
use crate::yaml::Value;

const NODE_SEP: f64 = 40.0;
const EDGE_SEP: f64 = 16.0;
const RANK_SEP: f64 = 48.0;
const GROUP_PAD: f64 = 20.0;
const LOOP: f64 = 26.0;
const SHAPE_ATTRS: &str = " fill=\"var(--_s)\" stroke=\"var(--_b)\"";
const LINE_ATTRS: &str = " stroke=\"var(--_b)\"";

/// Draws a top-level group from its box, title, style, classes, escaped id, authored source
/// lines, and the diagram's direction.
pub(crate) type DrawGroup = fn(&CNode, &Label, &ResolvedStyle, &str, &str, &[u32], Dir) -> String;

/// A diagram type that is a flowchart with a different layout, which may draw its top-level groups
/// itself: Pele's `FlowVariant`.
pub(crate) struct Variant {
    /// Draws top-level groups. Groups use the flowchart's drawing without it.
    pub(crate) draw_group: Option<DrawGroup>,
    /// Lays out the nodes and edges in a direction. The flag asks for tighter spacing.
    pub(crate) layout: fn(&mut [CNode], &mut [CEdge], Dir, &LayeredOptions, bool) -> CompoundResult,
    /// Adjusts the graph before it is measured.
    pub(crate) prepare: fn(&mut FlowGraph),
    /// The diagram type, which names its config section and the SVG's type.
    pub(crate) r#type: &'static str,
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

/// The value under `key` of a YAML mapping.
fn get<'v>(value: &'v Value, key: &str) -> Option<&'v Value> {
    match value {
        Value::Object(entries) => entries.iter().find(|(own, _)| own == key).map(|(_, v)| v),
        _ => None,
    }
}

fn number_option(section: Option<&Value>, key: &str, fallback: f64) -> f64 {
    match section.and_then(|section| get(section, key)) {
        Some(Value::Number(value)) if *value > 0.0 => value.min(2000.0),
        _ => fallback,
    }
}

fn asset_size(value: Option<f64>, fallback: f64) -> f64 {
    match value {
        Some(value) if value > 0.0 && value <= 4000.0 => value,
        _ => fallback,
    }
}

/// Draws a flowchart, or the flowchart `variant`, turning one that runs across to run down, and
/// tightening its spacing, when it does not fit the host's width.
pub(crate) fn render(
    db: &FlowDb,
    config: &Value,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
    variant: Option<&Variant>,
) -> Rendered {
    let fitted = |down: bool| {
        tighten(options, |tight| {
            draw(db, config, options, url, variant, down, tight)
        })
    };

    if runs_across(db.direction.as_deref()) {
        turn_to_fit(options, fitted)
    } else {
        fitted(false)
    }
}

/// `turned` draws a flowchart that runs across as one that runs down. `tight` sets nodes closer.
#[expect(
    clippy::too_many_lines,
    reason = "mirrors Pele's draw, one pass per drawing stage"
)]
fn draw(
    db: &FlowDb,
    config: &Value,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
    variant: Option<&Variant>,
    turned: bool,
    tight: bool,
) -> Rendered {
    let group_pad = if tight { 12.0 } else { GROUP_PAD };
    let own = variant
        .and_then(|variant| get(config, variant.r#type))
        .filter(|value| matches!(value, Value::Object(_)));
    let flow =
        own.or_else(|| get(config, "flowchart").filter(|value| matches!(value, Value::Object(_))));
    let curve = match flow.and_then(|flow| get(flow, "curve")) {
        Some(Value::String(curve)) => Some(curve.as_str()),
        _ => None,
    };
    let mut graph = graph::build(db, curve);
    if let Some(variant) = variant {
        (variant.prepare)(&mut graph);
    }
    let size = options.font_size.unwrap_or(16.0);
    let edge_size = crate::svg::builder::js_round(size * 0.875);
    let squeezed = variant.is_some() && tight;
    let wrap_width = number_option(flow, "wrappingWidth", 200.0).min(if squeezed {
        100.0
    } else {
        f64::INFINITY
    });
    let group_title_width = if squeezed {
        80.0_f64.max(title_room(options) / 2.0)
    } else {
        4000.0
    };
    let pad = options.padding.unwrap_or(8.0);

    let mut index: std::collections::BTreeMap<&str, usize> = std::collections::BTreeMap::new();
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
        let markdown = node.label_type == super::types::LabelType::Markdown;
        let font_size = style.font_size.unwrap_or(size);

        let (view, mut c) = if node.is_group {
            let label = layout_label(
                node.label.as_deref(),
                markdown,
                font_size,
                group_title_width,
                text_style,
                None,
            );
            let mut c = cnode(0.0, 0.0, -1);
            c.is_group = true;
            c.dir = node.dir.as_deref().map(|dir| direction(Some(dir)));
            c.pad_x = group_pad;
            c.pad_top = if label.height > 0.0 {
                label.height + 16.0
            } else {
                GROUP_PAD
            };
            c.pad_bottom = group_pad;
            c.min_w = label.width + 2.0 * group_pad - 12.0;
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
        } else {
            let shape = canonical_shape(Some(&node.shape));
            let has_asset = present(node.img.as_deref()) || present(node.icon.as_deref());
            let text = if has_asset || shape_has_label(shape) {
                node.label.as_deref()
            } else {
                None
            };
            let label = layout_label(text, markdown, font_size, wrap_width, text_style, None);

            let (w, h, dy) = if has_asset {
                let boxed = if present(node.img.as_deref()) {
                    80.0
                } else {
                    48.0
                };
                let aw = asset_size(node.asset_width, boxed);
                let ah = asset_size(node.asset_height, boxed);
                let w = aw.max(label.width + 8.0);
                let h = ah
                    + if label.height > 0.0 {
                        label.height + 6.0
                    } else {
                        0.0
                    };
                let dy = if label.height == 0.0 {
                    0.0
                } else if node.pos.as_deref() == Some("t") {
                    -ah / 2.0
                } else {
                    ah / 2.0
                };
                (w, h, dy)
            } else {
                let s = shape_size(shape, label.width, label.height);
                (s.w, s.h, s.dy)
            };

            let view = NodeView {
                dy,
                h,
                label,
                node,
                shape,
                style,
                w,
            };
            (view, cnode(w, h, -1))
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
    let flows_sideways = |cnodes: &[CNode], i: usize| -> bool {
        let mut dir: Option<Dir> = None;
        let mut p = cnodes.get(i).map_or(-1, |c| c.parent);
        let mut hops = 0;
        while let Ok(at) = usize::try_from(p) {
            if dir.is_some() || hops >= 10_000 {
                break;
            }
            let Some(parent) = cnodes.get(at) else { break };
            dir = parent.dir;
            p = parent.parent;
            hops += 1;
        }
        matches!(dir.unwrap_or(root_dir), Dir::Lr | Dir::Rl)
    };

    // The edges on one side of a node spread along it; a picture or an icon keeps them at its middle.
    for i in 0..views.len() {
        let Some(view) = views.get(i) else { continue };
        let span = if view.node.is_group {
            Some(f64::INFINITY)
        } else if !present(view.node.img.as_deref()) && !present(view.node.icon.as_deref()) {
            Some(shape_span(
                view.shape,
                view.w,
                view.h,
                flows_sideways(&cnodes, i),
            ))
        } else {
            None
        };
        if let (Some(span), Some(c)) = (span, cnodes.get_mut(i)) {
            c.span = span;
        }
    }

    let mut cedges: Vec<CEdge> = Vec::with_capacity(graph.edges.len());
    let mut edge_labels: Vec<Label> = Vec::with_capacity(graph.edges.len());
    let mut drawn: Vec<&GraphEdge> = Vec::with_capacity(graph.edges.len());
    let mut loops = vec![0usize; views.len()];

    for edge in &graph.edges {
        let (Some(&src), Some(&dst)) =
            (index.get(edge.start.as_str()), index.get(edge.end.as_str()))
        else {
            continue;
        };
        let markdown = edge.label_type == super::types::LabelType::Markdown;
        let label = layout_label(Some(&edge.label), markdown, edge_size, wrap_width, 0, None);

        if src == dst {
            // A self-loop sits beside the node, across the flow, and the node's box grows to hold it.
            if let Some(count) = loops.get_mut(src) {
                *count += 1;
            }
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

    let layout_options = LayeredOptions {
        apart: false,
        edge_sep: EDGE_SEP,
        head_room: marker_trim("arrow_point"),
        node_sep: number_option(flow, "nodeSpacing", NODE_SEP) * if tight { 0.5 } else { 1.0 },
        port_sep: 20.0,
        rank_sep: number_option(flow, "rankSpacing", RANK_SEP) * if tight { 0.75 } else { 1.0 },
        straight: curve == Some("linear"),
        tracks: curve.is_some_and(|curve| curve.starts_with("step")),
    };
    let layout = match variant {
        Some(variant) => {
            (variant.layout)(&mut cnodes, &mut cedges, root_dir, &layout_options, tight)
        }
        None => compound_layout(&mut cnodes, &mut cedges, root_dir, &layout_options),
    };

    let title_width = if tight { title_room(options) } else { 4000.0 };
    let title = layout_label(db.title.as_deref(), false, size, title_width, BOLD, None);
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

    let mut crossings = Crossings::new();
    if cnodes.iter().any(|c| c.is_group) {
        for e in &cedges {
            mark_crossings(&mut crossings, &e.route);
        }
    }

    let mut clusters = String::new();
    let mut struck_titles = String::new();
    let mut nodes_out = String::new();

    for (i, view) in views.iter().enumerate() {
        let Some(c) = cnodes.get(i) else { continue };
        let node = view.node;
        let classes = class_names(strip_default(&node.css_classes));
        let id = esc_text(&node.id);

        if c.is_group
            && c.parent < 0
            && let Some(draw_group) = variant.and_then(|variant| variant.draw_group)
        {
            clusters.push_str(&draw_group(
                c,
                &view.label,
                &view.style,
                &classes,
                &id,
                &node.lines,
                root_dir,
            ));
            continue;
        }

        if c.is_group {
            let title_x = cluster_title_x(c, view.label.width, &crossings, group_pad);
            let title_y = c.y - c.h / 2.0 + 8.0 + view.label.height / 2.0;
            let mut title_attrs =
                String::from(" class=\"ondine-cluster-label\" fill=\"var(--_m)\"");
            title_attrs.push_str(&view.style.text);
            let struck = title_crossed(c, view.label.width, title_x, &crossings);
            if struck {
                struck_titles.push_str(&struck_title(
                    &view.label,
                    title_x,
                    title_y,
                    &title_attrs,
                    &id,
                    &node.lines,
                    &cnodes,
                    i,
                    None,
                ));
            }

            clusters.push_str("<g class=\"ondine-cluster");
            clusters.push_str(&classes);
            clusters.push_str("\" data-id=\"");
            clusters.push_str(&id);
            clusters.push('"');
            lines::push_attribute(&mut clusters, &node.lines);
            clusters.push_str("><rect x=\"");
            push_num(&mut clusters, c.x - c.w / 2.0);
            clusters.push_str("\" y=\"");
            push_num(&mut clusters, c.y - c.h / 2.0);
            clusters.push_str("\" width=\"");
            push_num(&mut clusters, c.w);
            clusters.push_str("\" height=\"");
            push_num(&mut clusters, c.h);
            clusters.push_str("\" rx=\"");
            clusters.push_str(RADIUS);
            clusters.push_str("\" fill=\"var(--_a)\" fill-opacity=\"0.5\" stroke=\"var(--_b)\"");
            clusters.push_str(&view.style.shape);
            clusters.push_str("/>");
            if !struck {
                clusters.push_str(&label_svg(
                    &view.label,
                    title_x,
                    title_y,
                    &title_attrs,
                    None,
                ));
            }
            clusters.push_str("</g>");
            continue;
        }

        let (w, h) = (view.w, view.h);
        let inner = if let Some(img) = node.img.as_deref().filter(|img| !img.is_empty()) {
            image(node, view, img, url)
        } else if let Some(icon) = node.icon.as_deref().filter(|icon| !icon.is_empty()) {
            icon_svg(node, view, icon)
        } else {
            let mut shape_attrs = String::from(SHAPE_ATTRS);
            shape_attrs.push_str(&view.style.shape);
            let mut line_attrs = String::from(LINE_ATTRS);
            line_attrs.push_str(&view.style.line);
            draw_shape(view.shape, w, h, &shape_attrs, &line_attrs)
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
            // The label is a note name, but it still ends up in an href, so it gets the same check
            // as a URL.
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
        body.push_str(&inner);
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
        nodes_out.push_str(&classes);
        nodes_out.push_str("\" data-id=\"");
        nodes_out.push_str(&id);
        nodes_out.push('"');
        lines::push_attribute(&mut nodes_out, &node.lines);
        nodes_out.push_str(" transform=\"translate(");
        push_num(&mut nodes_out, c.x);
        nodes_out.push(',');
        push_num(&mut nodes_out, c.y);
        nodes_out.push_str(")\">");
        nodes_out.push_str(&body);
        nodes_out.push_str("</g>");
    }

    let mut reach = SideReach::default();
    for (ce, edge) in cedges.iter().zip(&drawn) {
        if ce.src == ce.dst || edge.thickness.as_deref() == Some("invisible") || ce.route.len() < 6
        {
            continue;
        }
        if let (Some(src), Some(dst)) = (cnodes.get(ce.src), cnodes.get(ce.dst)) {
            reach.add(ce.src, end_side(&ce.route, 0, src), edge.arrow_type_start);
            reach.add(
                ce.dst,
                end_side(&ce.route, ce.route.len() - 3, dst),
                edge.arrow_type_end,
            );
        }
    }

    let mut edges_out = String::new();
    let mut labels_out = String::new();
    let mut loop_seen = vec![0usize; views.len()];

    for i in 0..cedges.len() {
        let (Some(edge), Some(label)) = (drawn.get(i), edge_labels.get(i)) else {
            continue;
        };
        let style = if edge.style.is_empty() && edge.css_compiled_styles.is_empty() {
            None
        } else {
            let mut all = edge.css_compiled_styles.clone();
            all.extend(edge.style.iter().cloned());
            Some(resolve_style(&all))
        };
        let invisible = edge.thickness.as_deref() == Some("invisible");

        if !invisible {
            let start_type = edge.arrow_type_start;
            let end_type = edge.arrow_type_end;
            let Some(ce) = cedges.get(i) else { continue };
            let (src, dst) = (ce.src, ce.dst);

            let path: EdgePath = if src == dst {
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
                let leaves = reach.of(src, end_side(&route, 0, src_node));
                let arrives = reach.of(dst, end_side(&route, last, dst_node));
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
                route_path(
                    &route,
                    edge.curve.as_deref(),
                    marker_trim(start_type),
                    marker_trim(end_type),
                    Some(leaves),
                    Some(arrives),
                )
            };

            let color = match style.as_ref().and_then(|style| style.stroke.as_deref()) {
                Some(stroke) if !stroke.is_empty() => esc(stroke).into_owned(),
                _ => String::from("var(--_l)"),
            };
            let attrs = match edge.thickness.as_deref() {
                Some("thick") => " stroke-width=\"2.5\"",
                Some("dotted") => " stroke-dasharray=\"3 4\"",
                _ => "",
            };
            let classes = class_names(&strip_edge_classes(edge.classes));

            edges_out.push_str("<g class=\"ondine-edge");
            edges_out.push_str(&classes);
            edges_out.push_str("\" data-id=\"");
            edges_out.push_str(&esc_text(&edge.id));
            edges_out.push('"');
            lines::push_attribute(&mut edges_out, &edge.lines);
            if let Some(style) = &style {
                edges_out.push_str(&style.line);
            }
            edges_out.push_str("><path d=\"");
            edges_out.push_str(&path.d);
            edges_out.push('"');
            edges_out.push_str(attrs);
            edges_out.push_str("/>");
            edges_out.push_str(&marker(
                start_type, path.sx, path.sy, path.sdx, path.sdy, &color,
            ));
            edges_out.push_str(&marker(
                end_type, path.ex, path.ey, path.edx, path.edy, &color,
            ));
            edges_out.push_str("</g>");
        }

        if label.width > 0.0
            && let Some(ce) = cedges.get(i)
        {
            let attrs = style.as_ref().map_or("", |style| style.text.as_str());
            labels_out.push_str(&edge_label_svg(
                &edge.id,
                &edge.lines,
                label,
                ce.label_x,
                ce.label_y,
                attrs,
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
    if !struck_titles.is_empty() {
        content.push_str("<g class=\"ondine-cluster-titles\">");
        content.push_str(&struck_titles);
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
        variant.map_or("flowchart", |variant| variant.r#type),
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

fn present(value: Option<&str>) -> bool {
    value.is_some_and(|value| !value.is_empty())
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

fn image(node: &GraphNode, view: &NodeView<'_>, img: &str, url: &UrlOptions<'_>) -> String {
    let aw = asset_size(node.asset_width, 80.0);
    let ah = asset_size(node.asset_height, 80.0);
    let top = if view.label.height > 0.0 {
        if node.pos.as_deref() == Some("t") {
            view.h / 2.0 - ah
        } else {
            -view.h / 2.0
        }
    } else {
        -ah / 2.0
    };
    let safe = url::safe_url(img);
    let Some(src) = url::image_url(&safe, url) else {
        return String::new();
    };

    let mut out = String::from("<image href=\"");
    out.push_str(&esc(src));
    out.push_str("\" x=\"");
    push_num(&mut out, -aw / 2.0);
    out.push_str("\" y=\"");
    push_num(&mut out, top);
    out.push_str("\" width=\"");
    push_num(&mut out, aw);
    out.push_str("\" height=\"");
    push_num(&mut out, ah);
    out.push_str("\" preserveAspectRatio=\"");
    out.push_str(if node.constraint.as_deref() == Some("on") {
        "xMidYMid meet"
    } else {
        "none"
    });
    out.push_str("\"/>");
    out
}

/// An icon node. Icon glyphs need a host resolver, which WebAssembly hosts do not pass yet, so
/// only the frame and an empty glyph box are drawn.
fn icon_svg(node: &GraphNode, view: &NodeView<'_>, icon: &str) -> String {
    let side = asset_size(node.asset_height, 48.0);
    let top = if view.label.height > 0.0 {
        if node.pos.as_deref() == Some("t") {
            view.h / 2.0 - side
        } else {
            -view.h / 2.0
        }
    } else {
        -side / 2.0
    };

    let mut out = String::new();
    match node.shape.as_str() {
        "icon" => {}
        "iconCircle" => {
            out.push_str("<circle cy=\"");
            push_num(&mut out, top + side / 2.0);
            out.push_str("\" r=\"");
            push_num(&mut out, side / 2.0);
            out.push('"');
            out.push_str(SHAPE_ATTRS);
            out.push_str(&view.style.shape);
            out.push_str("/>");
        }
        _ => {
            out.push_str("<rect x=\"");
            push_num(&mut out, -side / 2.0);
            out.push_str("\" y=\"");
            push_num(&mut out, top);
            out.push_str("\" width=\"");
            push_num(&mut out, side);
            out.push_str("\" height=\"");
            push_num(&mut out, side);
            out.push_str("\" rx=\"");
            out.push_str(if node.shape == "iconRounded" {
                "8"
            } else {
                "0"
            });
            out.push('"');
            out.push_str(SHAPE_ATTRS);
            out.push_str(&view.style.shape);
            out.push_str("/>");
        }
    }

    let glyph = side * 0.6;
    out.push_str("<svg class=\"ondine-icon\" data-icon=\"");
    out.push_str(&esc_text(icon));
    out.push_str("\" x=\"");
    push_num(&mut out, -glyph / 2.0);
    out.push_str("\" y=\"");
    push_num(&mut out, top + (side - glyph) / 2.0);
    out.push_str("\" width=\"");
    push_num(&mut out, glyph);
    out.push_str("\" height=\"");
    push_num(&mut out, glyph);
    out.push_str(
        "\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" \
         stroke-linecap=\"round\" stroke-linejoin=\"round\"></svg>",
    );
    out
}
