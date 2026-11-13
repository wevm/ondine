//! Lays out and draws a class diagram. Port of Pele's `src/diagrams/class/render.ts`.

use std::collections::BTreeMap;

use super::db::ClassDb;
use super::graph::{self, GraphEdge, GraphNode, NodeShape};
use super::markers::{class_marker, class_marker_trim};
use super::members::ClassMember;
use super::types::ClassNode;
use crate::diagrams::Rendered;
use crate::diagrams::common::fit_width::{runs_across, tighten, turn_to_fit};
use crate::diagrams::common::generics::parse_generic_types;
use crate::layout::compound::{CEdge, CNode, Dir, cnode, compound_layout, direction};
use crate::layout::layered::LayeredOptions;
use crate::svg::builder::{esc, esc_text, js_round, label_svg, push_num};
use crate::svg::cluster::{
    Crossings, cluster_title_x, mark_crossings, struck_title, title_crossed,
};
use crate::svg::edges::{EdgePath, edge_label_svg, route_path};
use crate::svg::root::{Accessible, RenderOptions, svg_document};
use crate::svg::shapes::end_side;
use crate::svg::theme::{RADIUS, ResolvedStyle, class_names, resolve_style};
use crate::svg::title::with_title;
use crate::text::entities;
use crate::text::label::{Label, layout_label, trim};
use crate::text::measure::{BOLD, ITALIC, width};
use crate::util::chars::is_space;
use crate::util::url::{LinkInfo, UrlOptions, linked};
use crate::yaml::Value;

/// Room between nodes in a rank.
const NODE_SEP: f64 = 40.0;
/// Room between edges.
const EDGE_SEP: f64 = 16.0;
/// Room between ranks.
const RANK_SEP: f64 = 56.0;
/// Padding inside a namespace.
const GROUP_PAD: f64 = 20.0;
/// How far a self-relation reaches out.
const LOOP: f64 = 26.0;
/// Padding beside a class's text.
const PAD_X: f64 = 12.0;
/// Padding above and below a class's name.
const TITLE_PAD: f64 = 8.0;
/// Padding above and below a member section.
const SECTION_PAD: f64 = 6.0;
/// Height of an empty member section.
const EMPTY_SECTION: f64 = 10.0;
/// Least width of a class.
const MIN_WIDTH: f64 = 64.0;
/// Size of a note's folded corner.
const FOLD: f64 = 8.0;
/// The color of lines and markers.
const LINE: &str = "var(--_l)";
/// The link targets written out.
const TARGETS: [&str; 4] = ["_self", "_blank", "_parent", "_top"];

/// The value under `key` of a YAML mapping.
fn get<'v>(value: &'v Value, key: &str) -> Option<&'v Value> {
    match value {
        Value::Object(entries) => entries.iter().find(|(own, _)| own == key).map(|(_, v)| v),
        _ => None,
    }
}

/// A positive number option of the class section, capped at 2000, or `fallback`.
fn number_option(section: Option<&Value>, key: &str, fallback: f64) -> f64 {
    match section.and_then(|section| get(section, key)) {
        Some(Value::Number(value)) if *value > 0.0 => value.min(2000.0),
        _ => fallback,
    }
}

/// A member line, measured.
struct Line {
    classifier: String,
    text: String,
    width: f64,
}

/// What a node is drawn as, beside its layout box.
struct View {
    annotations: Vec<String>,
    attributes: Vec<Line>,
    attributes_h: f64,
    font_size: f64,
    h: f64,
    label: Label,
    methods: Vec<Line>,
    style: ResolvedStyle,
    title_h: f64,
    w: f64,
}

/// An end of an edge on a side of a class.
#[derive(Clone, Copy)]
struct End {
    /// Whether the text of this end goes on the right of, or below, its line.
    after: bool,
    /// The index of the route's point.
    at: usize,
    /// The edge.
    edge: usize,
    order: usize,
    room: f64,
    side: i32,
    toward: f64,
}

/// A cardinality text.
struct Card {
    text: String,
    x: f64,
    y: f64,
}

/// Measures a member's shown text at `size`.
fn member_line(member: &ClassMember, size: f64) -> Line {
    let decoded = entities::decode(&member.display_text()).into_owned();
    let text = trim(&decoded).to_owned();
    let style = if member.classifier == "*" { ITALIC } else { 0 };
    Line {
        classifier: member.classifier.clone(),
        width: width(&text, size, style),
        text,
    }
}

/// A count as a coordinate.
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
        Some(c) if is_space(u32::from(c)) => rest.get(c.len_utf8()..).unwrap_or(""),
        _ => rest,
    }
}

/// Draws a class diagram, turning one that runs across to run down, and tightening it, when it
/// does not fit the host's width.
pub(crate) fn render(
    db: &ClassDb,
    config: &Value,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
) -> Rendered {
    let fitted = |down: bool| tighten(options, |tight| draw(db, config, options, url, down, tight));
    if runs_across(Some(db.direction)) {
        turn_to_fit(options, fitted)
    } else {
        fitted(false)
    }
}

/// The style of a class: the effective declarations of its class definitions and its own, the
/// last of each property winning. A class may carry the declarations of many definitions many
/// times over, so each distinct declaration is checked once.
struct Styler<'d> {
    db: &'d ClassDb,
    /// The property each declaration sets, or empty when it has no effect.
    effect: BTreeMap<String, String>,
    /// The kept declarations of each list of class names.
    from_defs: BTreeMap<String, Vec<(String, String)>>,
    /// The kept declarations by property, in the order the properties were first set.
    kept: Vec<(String, String)>,
}

impl Styler<'_> {
    /// Keeps `decl` when it has an effect, as the last value of its property.
    fn keep(&mut self, decl: &str) {
        let prop = if let Some(prop) = self.effect.get(decl) {
            prop.clone()
        } else {
            let one = resolve_style(&[decl]);
            let prop = if one.shape.is_empty() && one.text.is_empty() {
                String::new()
            } else {
                let end = decl
                    .find(':')
                    .unwrap_or_else(|| decl.char_indices().last().map_or(0, |(at, _)| at));
                trim(decl.get(..end).unwrap_or("")).to_ascii_lowercase()
            };
            self.effect.insert(decl.to_owned(), prop.clone());
            prop
        };
        if prop.is_empty() {
            return;
        }
        match self.kept.iter_mut().find(|(own, _)| *own == prop) {
            Some(entry) => decl.clone_into(&mut entry.1),
            None => self.kept.push((prop, decl.to_owned())),
        }
    }

    /// The resolved style of `cls`.
    fn style(&mut self, cls: &ClassNode) -> ResolvedStyle {
        self.kept.clear();
        if let Some(base) = self.from_defs.get(&cls.css_classes) {
            self.kept.extend(base.iter().cloned());
        } else {
            let db = self.db;
            for name in cls.css_classes.split(' ') {
                if let Some(def) = db.style_classes.get(name) {
                    for s in &def.styles {
                        for part in s.split(',') {
                            self.keep(part);
                        }
                    }
                }
            }
            self.from_defs
                .insert(cls.css_classes.clone(), self.kept.clone());
        }
        for s in &cls.styles {
            self.keep(s);
        }
        let decls: Vec<&str> = self.kept.iter().map(|(_, decl)| decl.as_str()).collect();
        resolve_style(&decls)
    }
}

/// `turned` draws a diagram that runs across as one that runs down. `tight` gives up the room
/// that keeps a namespace's name clear of an edge coming in beside it.
#[expect(
    clippy::float_cmp,
    clippy::many_single_char_names,
    clippy::too_many_lines,
    reason = "mirrors Pele's draw, one pass per drawing stage, and its exact comparisons"
)]
fn draw(
    db: &ClassDb,
    config: &Value,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
    turned: bool,
    tight: bool,
) -> Rendered {
    let conf = get(config, "class").filter(|value| matches!(value, Value::Object(_)));
    let hierarchical = !matches!(
        conf.and_then(|conf| get(conf, "hierarchicalNamespaces")),
        Some(Value::Bool(false))
    );
    let graph = graph::build(db, hierarchical);
    let hide_empty = matches!(
        conf.and_then(|conf| get(conf, "hideEmptyMembersBox")),
        Some(Value::Bool(true))
    );
    let size = options.font_size.unwrap_or(16.0);
    let small = js_round(size * 0.875);
    let tiny = js_round(size * 0.8);
    let tiny_height = js_round(tiny * 1.4);
    let pad = options.padding.unwrap_or(8.0);
    let root_dir = if turned {
        Dir::Tb
    } else {
        direction(Some(db.direction))
    };
    let n = graph.nodes.len();

    let mut styler = Styler {
        db,
        effect: BTreeMap::new(),
        from_defs: BTreeMap::new(),
        kept: Vec::new(),
    };

    let parent_of = |node: &GraphNode<'_>| usize::try_from(node.parent).ok();
    let mut children = vec![0usize; n];
    for node in &graph.nodes {
        if let Some(count) = parent_of(node).and_then(|p| children.get_mut(p)) {
            *count += 1;
        }
    }

    // Namespaces that an edge may cross into are made wide enough for the title to sit beside it.
    let mut crossed = vec![false; n];
    for edge in &graph.edges {
        let (Some(source), Some(target)) = (edge.source, edge.target) else {
            continue;
        };
        let a = graph.nodes.get(source).map_or(-1, |node| node.parent);
        let b = graph.nodes.get(target).map_or(-1, |node| node.parent);
        if a == b {
            continue;
        }
        for start in [a, b] {
            let mut g = start;
            while let Ok(at) = usize::try_from(g) {
                match crossed.get_mut(at) {
                    Some(flag) if !*flag => *flag = true,
                    _ => break,
                }
                g = graph.nodes.get(at).map_or(-1, |node| node.parent);
            }
        }
    }

    let base = |label: Label, w: f64, h: f64, font_size: f64| View {
        annotations: Vec::new(),
        attributes: Vec::new(),
        attributes_h: 0.0,
        font_size,
        h,
        label,
        methods: Vec::new(),
        style: resolve_style::<&str>(&[]),
        title_h: 0.0,
        w,
    };

    let mut views: Vec<View> = Vec::with_capacity(n);
    let mut cnodes: Vec<CNode> = Vec::with_capacity(n);
    for (i, node) in graph.nodes.iter().enumerate() {
        let (view, mut c) = if node.is_group {
            let label = layout_label(Some(&node.label), false, size, 4000.0, 0, None);
            if children.get(i).copied().unwrap_or(0) > 0 {
                let mut c = cnode(0.0, 0.0, -1);
                c.is_group = true;
                c.pad_x = GROUP_PAD;
                c.pad_top = if label.height > 0.0 {
                    label.height + 16.0
                } else {
                    GROUP_PAD
                };
                c.pad_bottom = GROUP_PAD;
                c.min_w = label.width + 2.0 * GROUP_PAD - 12.0
                    + if crossed.get(i).copied().unwrap_or(false) && !tight {
                        (label.width + 24.0).min(240.0)
                    } else {
                        0.0
                    };
                (base(label, 0.0, 0.0, size), c)
            } else {
                let w = (label.width + 2.0 * GROUP_PAD).max(80.0);
                let h = label.height + 2.0 * GROUP_PAD;
                (base(label, w, h, size), cnode(w, h, -1))
            }
        } else if let Some(cls) = node.class_node {
            let view = class_view(&mut styler, cls, size, tiny, tiny_height, hide_empty);
            let c = cnode(view.w, view.h, -1);
            (view, c)
        } else if node.shape == NodeShape::Note {
            let label = layout_label(Some(&node.label), true, small, 260.0, 0, None);
            let w = (label.width + 2.0 * PAD_X).max(40.0);
            let h = label.height + 2.0 * TITLE_PAD;
            (base(label, w, h, small), cnode(w, h, -1))
        } else {
            let label = layout_label(Some(&node.label), false, small, 200.0, 0, None);
            let w = label.width.max(12.0) + 4.0;
            let h = label.height.max(12.0);
            (base(label, w, h, small), cnode(w, h, -1))
        };
        c.parent = node.parent;
        c.seq = to_f64(i);
        views.push(view);
        cnodes.push(c);
    }

    let mut cedges: Vec<CEdge> = Vec::new();
    let mut drawn: Vec<&GraphEdge> = Vec::new();
    let mut edge_labels: Vec<Label> = Vec::new();
    let mut start_room: Vec<f64> = Vec::new();
    let mut end_room: Vec<f64> = Vec::new();
    // Edges leave one side of a class and arrive at the other, so a class with many of them is
    // widened until they, and the text at their ends, fit side by side.
    let mut crowd = vec![0.0f64; n * 4];
    let mut meet = |item: usize, slot: usize, room: f64| {
        if let Some(count) = crowd.get_mut(item * 4 + slot) {
            *count += 1.0;
        }
        if let Some(most) = crowd.get_mut(item * 4 + slot + 1)
            && room > *most
        {
            *most = room;
        }
    };
    for edge in &graph.edges {
        let (Some(source), Some(target)) = (edge.source, edge.target) else {
            continue;
        };
        let label = layout_label(edge.label.as_deref(), true, small, 200.0, 0, None);
        let start_w = if edge.start_label_right.is_empty() {
            0.0
        } else {
            width(&entities::decode(&edge.start_label_right), tiny, 0)
        };
        let end_w = if edge.end_label_left.is_empty() {
            0.0
        } else {
            width(&entities::decode(&edge.end_label_left), tiny, 0)
        };
        if source == target {
            if let Some(c) = cnodes.get_mut(source) {
                c.w += 2.0
                    * (LOOP
                        + if label.width > 0.0 {
                            label.width + 12.0
                        } else {
                            0.0
                        });
            }
        } else {
            meet(source, 0, start_w);
            meet(target, 2, end_w);
        }
        start_room.push(start_w);
        end_room.push(end_w);
        drawn.push(edge);
        cedges.push(CEdge {
            dst: target,
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
            src: source,
        });
        edge_labels.push(label);
    }

    if matches!(root_dir, Dir::Tb | Dir::Bt) {
        for (i, node) in graph.nodes.iter().enumerate() {
            if node.class_node.is_none() {
                continue;
            }
            let mut need: f64 = 0.0;
            for slot in [0, 2] {
                let count = crowd.get(i * 4 + slot).copied().unwrap_or(0.0);
                let room = crowd.get(i * 4 + slot + 1).copied().unwrap_or(0.0);
                if count > 1.0 {
                    need = need
                        .max((count - 1.0) * if room > 0.0 { room + 24.0 } else { 14.0 } + 24.0);
                }
            }
            need = need.min(480.0);
            if let (Some(view), Some(c)) = (views.get_mut(i), cnodes.get_mut(i))
                && need > view.w
            {
                c.w += need - view.w;
                view.w = need;
            }
        }
    }

    let lettered = start_room
        .iter()
        .zip(&end_room)
        .any(|(&s, &e)| s > 0.0 || e > 0.0);
    let layout = compound_layout(
        &mut cnodes,
        &mut cedges,
        root_dir,
        &LayeredOptions {
            apart: true,
            edge_sep: EDGE_SEP,
            head_room: 0.0,
            node_sep: number_option(conf, "nodeSpacing", NODE_SEP),
            port_sep: 20.0,
            rank_sep: number_option(
                conf,
                "rankSpacing",
                if lettered { RANK_SEP + 16.0 } else { RANK_SEP },
            ),
            straight: false,
            tracks: false,
        },
    );

    // Every edge reaches the middle of a side. Where several share a side they are spread along
    // it, in the order of where they come from. An end with text runs straight for the length of
    // the text, which sits beside it on the side the line then bends toward, away from the
    // neighbors that bend the other way. Neighbors are set apart far enough for the text between
    // them.
    let mut side_keys: BTreeMap<usize, usize> = BTreeMap::new();
    let mut sides: Vec<(usize, Vec<End>)> = Vec::new();
    let mut attach = |cedges: &mut [CEdge], edge: usize, at: usize, item: usize, room: f64| {
        let (Some(c), Some(view), Some(ce)) =
            (cnodes.get(item), views.get(item), cedges.get_mut(edge))
        else {
            return None;
        };
        let route = &mut ce.route;
        let side = end_side(route, at, c);
        let side_at = usize::try_from(side).ok()?;
        let slack = (c.w - view.w) / 2.0;
        if let Some(x) = route.get_mut(at) {
            if side == 1 {
                *x -= slack;
            } else if side == 3 {
                *x += slack;
            }
        }
        let across = usize::try_from(side & 1).unwrap_or(0);
        let toward_at = if at == 0 { 3 } else { at - 3 } + across;
        let toward = route.get(toward_at).copied().unwrap_or(f64::NAN);
        let bend = toward - route.get(at + across).copied().unwrap_or(f64::NAN);
        let end = End {
            after: if across == 1 { bend > 0.0 } else { bend >= 0.0 },
            at,
            edge,
            order: edge,
            room,
            side,
            toward,
        };
        let key = item * 4 + side_at;
        if let Some(&k) = side_keys.get(&key) {
            if let Some((_, list)) = sides.get_mut(k) {
                list.push(end);
            }
        } else {
            side_keys.insert(key, sides.len());
            sides.push((key, vec![end]));
        }
        Some(end)
    };
    let mut starts: Vec<Option<End>> = Vec::with_capacity(cedges.len());
    let mut ends: Vec<Option<End>> = Vec::with_capacity(cedges.len());
    for i in 0..cedges.len() {
        let Some(ce) = cedges.get(i) else { continue };
        let (src, dst, len) = (ce.src, ce.dst, ce.route.len());
        let routed = src != dst && len >= 6;
        let start = if routed {
            attach(
                &mut cedges,
                i,
                0,
                src,
                start_room.get(i).copied().unwrap_or(0.0),
            )
        } else {
            None
        };
        starts.push(start);
        let end = if routed {
            attach(
                &mut cedges,
                i,
                len - 3,
                dst,
                end_room.get(i).copied().unwrap_or(0.0),
            )
        } else {
            None
        };
        ends.push(end);
    }

    // Whether a point lies on the border of one of the namespaces around a class.
    let on_border = |cnodes: &[CNode], item: usize, x: f64, y: f64| -> bool {
        let mut g = cnodes.get(item).map_or(-1, |c| c.parent);
        let mut hops = 0;
        while let Ok(at) = usize::try_from(g) {
            if hops >= 10_000 {
                break;
            }
            let Some(c) = cnodes.get(at) else { break };
            let dx = (x - c.x).abs();
            let dy = (y - c.y).abs();
            if ((dy - c.h / 2.0).abs() < 0.5 && dx <= c.w / 2.0 + 0.5)
                || ((dx - c.w / 2.0).abs() < 0.5 && dy <= c.h / 2.0 + 0.5)
            {
                return true;
            }
            g = c.parent;
            hops += 1;
        }
        false
    };
    let mut gaps: Vec<f64> = Vec::new();
    for (key, list) in &mut sides {
        if list.len() < 2 {
            continue;
        }
        let item = *key >> 2;
        let across = *key & 1;
        let Some(view) = views.get(item) else {
            continue;
        };
        let length = if across == 1 { view.h } else { view.w };
        // Equal values, both zeros included, fall back to authored order, as `a - b || ...`
        // does in JavaScript.
        list.sort_by(|a, b| {
            if a.toward == b.toward {
                a.order.cmp(&b.order)
            } else {
                a.toward.total_cmp(&b.toward)
            }
        });
        let mut total = 0.0;
        gaps.clear();
        for pair in list.windows(2) {
            let [a, b] = pair else { continue };
            let mut gap = if across == 1 { 20.0 } else { 24.0 };
            if a.room > 0.0 && a.after {
                gap += if across == 1 {
                    tiny_height + 6.0
                } else {
                    a.room
                };
            }
            if b.room > 0.0 && !b.after {
                gap += if across == 1 {
                    tiny_height + 6.0
                } else {
                    b.room
                };
            }
            gaps.push(gap);
            total += gap;
        }
        let scale = ((length - 24.0) / total).min(1.0);
        if scale <= 0.0 {
            continue;
        }
        let Some(c) = cnodes.get(item) else { continue };
        let mut at = if across == 1 { c.y } else { c.x } - (total * scale) / 2.0;
        for (j, end) in list.iter().enumerate() {
            let Some(route) = cedges.get_mut(end.edge).map(|ce| &mut ce.route) else {
                continue;
            };
            // An edge that crosses the border of the class's namespace close by runs straight to
            // it: the point where it crosses moves into line with its end.
            let forward = end.at == 0;
            let first = if forward {
                end.at + 3
            } else {
                end.at.wrapping_sub(3)
            };
            let level = route.get(first + across).copied().unwrap_or(f64::NAN);
            let mut border = None;
            let mut k = first;
            while let (Some(&x), Some(&y), Some(&v)) =
                (route.get(k), route.get(k + 1), route.get(k + across))
            {
                if (v - level).abs().partial_cmp(&0.5) != Some(std::cmp::Ordering::Less) {
                    break;
                }
                if on_border(&cnodes, item, x, y) {
                    border = Some(k);
                    break;
                }
                if forward {
                    k += 3;
                } else if let Some(back) = k.checked_sub(3) {
                    k = back;
                } else {
                    break;
                }
            }
            if let Some(border) = border
                && (level - at).abs() <= 24.0
            {
                let mut k = first;
                loop {
                    if let Some(v) = route.get_mut(k + across) {
                        *v = at;
                    }
                    if k == border {
                        break;
                    }
                    k = if forward { k + 3 } else { k - 3 };
                }
            }
            if let Some(v) = route.get_mut(end.at + across) {
                *v = at;
            }
            at += gaps.get(j).copied().unwrap_or(f64::NAN) * scale;
        }
    }

    let straighten = |route: &mut Vec<f64>, end: Option<End>, room: f64, first: bool| {
        let Some(end) = end else { return };
        if room <= 0.0 || route.len() < 6 {
            return;
        }
        let across = usize::try_from(end.side & 1).unwrap_or(0);
        let at = if first { 0 } else { route.len() - 3 };
        let along = at + 1 - across;
        let next = if first { 3 } else { at - 3 } + 1 - across;
        let reach = (route.get(next).copied().unwrap_or(f64::NAN)
            - route.get(along).copied().unwrap_or(f64::NAN))
        .abs();
        let length = (if across == 1 {
            room + 10.0
        } else {
            tiny_height + 6.0
        })
        .min(reach * 0.35);
        if length < 4.0 {
            return;
        }
        let mut point = [
            route.get(at).copied().unwrap_or(0.0),
            route.get(at + 1).copied().unwrap_or(0.0),
            route.get(at + 2).copied().unwrap_or(0.0),
        ];
        if let Some(v) = point.get_mut(1 - across) {
            *v += if end.side == 0 || end.side == 3 {
                -1.0
            } else {
                1.0
            } * length;
        }
        let insert = if first { 3 } else { at };
        route.splice(insert..insert, point);
    };
    for i in 0..cedges.len() {
        let Some(ce) = cedges.get_mut(i) else {
            continue;
        };
        let start = starts.get(i).copied().flatten();
        let end = ends.get(i).copied().flatten();
        straighten(
            &mut ce.route,
            start,
            start_room.get(i).copied().unwrap_or(0.0),
            true,
        );
        straighten(
            &mut ce.route,
            end,
            end_room.get(i).copied().unwrap_or(0.0),
            false,
        );
    }

    let mut min_x: f64 = 0.0;
    let mut min_y: f64 = 0.0;
    let mut max_x = layout.width;
    let mut max_y = layout.height;
    let mut cards: Vec<Card> = Vec::new();
    let mut card = |raw: &str, width: f64, x: f64, y: f64, side: i32, after: bool| {
        if raw.is_empty() {
            return;
        }
        let mut cx = x;
        let mut cy = y;
        if side & 1 == 1 {
            cx += if side == 3 { -1.0 } else { 1.0 } * (5.0 + width / 2.0);
            cy += if after { 1.0 } else { -1.0 } * (7.0 + tiny_height / 2.0);
        } else {
            cx += if after { 1.0 } else { -1.0 } * (9.0 + width / 2.0);
            cy += if side == 0 { -1.0 } else { 1.0 } * (3.0 + tiny_height / 2.0);
        }
        cards.push(Card {
            text: entities::decode(raw).into_owned(),
            x: cx,
            y: cy,
        });
        min_x = min_x.min(cx - width / 2.0);
        max_x = max_x.max(cx + width / 2.0);
        min_y = min_y.min(cy - tiny_height / 2.0);
        max_y = max_y.max(cy + tiny_height / 2.0);
    };
    let mut loop_seen = vec![0usize; n];
    let mut loop_at = vec![0usize; cedges.len()];
    for (i, ce) in cedges.iter().enumerate() {
        let Some(edge) = drawn.get(i) else { continue };
        let s_room = start_room.get(i).copied().unwrap_or(0.0);
        let e_room = end_room.get(i).copied().unwrap_or(0.0);
        let route = &ce.route;
        if ce.src == ce.dst {
            let k = loop_seen.get(ce.src).copied().unwrap_or(0);
            if let Some(seen) = loop_seen.get_mut(ce.src) {
                *seen = k + 1;
            }
            if let Some(slot) = loop_at.get_mut(i) {
                *slot = k;
            }
            let (Some(c), Some(view)) = (cnodes.get(ce.src), views.get(ce.src)) else {
                continue;
            };
            let x = c.x + view.w / 2.0;
            let spread = (view.h / 2.0 - 4.0).min(8.0 + to_f64(k) * 6.0);
            card(&edge.start_label_right, s_room, x, c.y - spread, 1, false);
            card(&edge.end_label_left, e_room, x, c.y + spread, 1, true);
        } else if route.len() >= 6 {
            let last = route.len() - 3;
            let from = starts.get(i).copied().flatten();
            let to = ends.get(i).copied().flatten();
            card(
                &edge.start_label_right,
                s_room,
                route.first().copied().unwrap_or(0.0),
                route.get(1).copied().unwrap_or(0.0),
                from.map_or(2, |end| end.side),
                from.is_none_or(|end| end.after),
            );
            card(
                &edge.end_label_left,
                e_room,
                route.get(last).copied().unwrap_or(0.0),
                route.get(last + 1).copied().unwrap_or(0.0),
                to.map_or(0, |end| end.side),
                to.is_none_or(|end| end.after),
            );
        }
    }

    let ox = pad - min_x;
    let oy = pad - min_y;
    let width = (max_x - min_x + 2.0 * pad).ceil();
    let height = (max_y - min_y + 2.0 * pad).ceil();
    let mut links: Vec<LinkInfo> = Vec::new();

    let mut crossings = Crossings::new();
    if graph.nodes.first().is_some_and(|node| node.is_group) {
        for e in &cedges {
            mark_crossings(&mut crossings, &e.route);
        }
    }

    let mut clusters = String::new();
    let mut struck_titles = String::new();
    let mut nodes_out = String::new();
    for (i, (view, node)) in views.iter().zip(&graph.nodes).enumerate() {
        let Some(c) = cnodes.get(i) else { continue };
        let x = c.x + ox;
        let y = c.y + oy;
        let id = esc_text(&node.id);
        let w = if c.is_group { c.w } else { view.w };
        let h = if c.is_group { c.h } else { view.h };
        if node.is_group {
            let title_x = if c.is_group {
                cluster_title_x(c, view.label.width, &crossings, GROUP_PAD)
            } else {
                c.x
            };
            let title_y = y - h / 2.0 + 8.0 + view.label.height / 2.0;
            let title_attrs = " class=\"ondine-cluster-label\" fill=\"var(--_m)\"";
            let struck = c.is_group && title_crossed(c, view.label.width, title_x, &crossings);
            if struck {
                struck_titles.push_str(&struck_title(
                    &view.label,
                    title_x + ox,
                    title_y,
                    title_attrs,
                    &id,
                    &[],
                    &cnodes,
                    i,
                    None,
                ));
            }
            clusters.push_str("<g class=\"ondine-cluster ondine-namespace\" data-id=\"");
            clusters.push_str(&id);
            clusters.push_str("\"><rect x=\"");
            push_num(&mut clusters, x - w / 2.0);
            clusters.push_str("\" y=\"");
            push_num(&mut clusters, y - h / 2.0);
            clusters.push_str("\" width=\"");
            push_num(&mut clusters, w);
            clusters.push_str("\" height=\"");
            push_num(&mut clusters, h);
            clusters.push_str("\" rx=\"");
            clusters.push_str(RADIUS);
            clusters.push_str("\" fill=\"var(--_a)\" fill-opacity=\"0.5\" stroke=\"var(--_b)\"/>");
            if !struck {
                clusters.push_str(&label_svg(
                    &view.label,
                    title_x + ox,
                    title_y,
                    title_attrs,
                    None,
                ));
            }
            clusters.push_str("</g>");
            continue;
        }

        let mut at = String::from(" transform=\"translate(");
        push_num(&mut at, x);
        at.push(',');
        push_num(&mut at, y);
        at.push_str(")\"");
        if let Some(cls) = node.class_node {
            let sizes = Sizes {
                size,
                tiny,
                tiny_height,
            };
            nodes_out.push_str(&class_svg(view, cls, &id, &at, &sizes, url, &mut links));
        } else if node.shape == NodeShape::Note {
            let r = w / 2.0;
            let t = -h / 2.0;
            nodes_out.push_str("<g class=\"ondine-node ondine-note\" data-id=\"");
            nodes_out.push_str(&id);
            nodes_out.push('"');
            nodes_out.push_str(&at);
            nodes_out.push_str("><path d=\"M");
            push_num(&mut nodes_out, -r);
            nodes_out.push(',');
            push_num(&mut nodes_out, t);
            nodes_out.push('H');
            push_num(&mut nodes_out, r - FOLD);
            nodes_out.push('L');
            push_num(&mut nodes_out, r);
            nodes_out.push(',');
            push_num(&mut nodes_out, t + FOLD);
            nodes_out.push('V');
            push_num(&mut nodes_out, -t);
            nodes_out.push('H');
            push_num(&mut nodes_out, -r);
            nodes_out.push_str(
                "Z\" fill=\"var(--_a)\" stroke=\"var(--_b)\" stroke-linejoin=\"round\"/><path d=\"M",
            );
            push_num(&mut nodes_out, r - FOLD);
            nodes_out.push(',');
            push_num(&mut nodes_out, t);
            nodes_out.push('V');
            push_num(&mut nodes_out, t + FOLD);
            nodes_out.push('H');
            push_num(&mut nodes_out, r);
            nodes_out.push_str("\" fill=\"none\" stroke=\"var(--_b)\" stroke-linejoin=\"round\"/>");
            nodes_out.push_str(&label_svg(&view.label, 0.0, 0.0, &label_attrs(small), None));
            nodes_out.push_str("</g>");
        } else {
            nodes_out.push_str("<g class=\"ondine-node ondine-interface\" data-id=\"");
            nodes_out.push_str(&id);
            nodes_out.push('"');
            nodes_out.push_str(&at);
            nodes_out.push('>');
            nodes_out.push_str(&label_svg(&view.label, 0.0, 0.0, &label_attrs(small), None));
            nodes_out.push_str("</g>");
        }
    }

    let mut edges_out = String::new();
    let mut labels_out = String::new();
    for i in 0..cedges.len() {
        let (Some(edge), Some(label)) = (drawn.get(i), edge_labels.get(i)) else {
            continue;
        };
        let start_type = edge.arrow_type_start;
        let end_type = edge.arrow_type_end;
        let start_trim = class_marker_trim(start_type);
        let end_trim = class_marker_trim(end_type);
        let Some(ce) = cedges.get_mut(i) else {
            continue;
        };
        let path = if ce.src == ce.dst {
            let (Some(c), Some(view)) = (cnodes.get(ce.src), views.get(ce.src)) else {
                continue;
            };
            let k = to_f64(loop_at.get(i).copied().unwrap_or(0));
            let x = c.x + ox + view.w / 2.0;
            let y = c.y + oy;
            let spread = (view.h / 2.0 - 4.0).min(8.0 + k * 6.0);
            let reach = LOOP + k * 8.0;
            let mut d = String::from("M");
            push_num(&mut d, x + start_trim);
            d.push(',');
            push_num(&mut d, y - spread);
            d.push('C');
            push_num(&mut d, x + reach + start_trim);
            d.push(',');
            push_num(&mut d, y - spread);
            d.push(' ');
            push_num(&mut d, x + reach + end_trim);
            d.push(',');
            push_num(&mut d, y + spread);
            d.push(' ');
            push_num(&mut d, x + end_trim);
            d.push(',');
            push_num(&mut d, y + spread);
            ce.label_x = c.x + view.w / 2.0 + reach + 4.0 + ce.label_w / 2.0;
            ce.label_y = c.y;
            EdgePath {
                d,
                edx: -1.0,
                edy: 0.0,
                ex: x,
                ey: y + spread,
                sdx: -1.0,
                sdy: 0.0,
                sx: x,
                sy: y - spread,
            }
        } else {
            for point in ce.route.chunks_mut(3) {
                if let [x, y, ..] = point {
                    *x += ox;
                    *y += oy;
                }
            }
            route_path(&ce.route, None, start_trim, end_trim, None, None)
        };
        let attrs = if edge.dashed {
            " stroke-dasharray=\"6 4\""
        } else if edge.dotted {
            " stroke-dasharray=\"2 4\""
        } else {
            ""
        };
        edges_out.push_str("<g class=\"ondine-edge");
        edges_out.push_str(if edge.dotted {
            " ondine-note-edge"
        } else {
            " ondine-relation"
        });
        edges_out.push_str("\" data-id=\"");
        edges_out.push_str(&esc_text(&edge.id));
        edges_out.push_str("\"><path d=\"");
        edges_out.push_str(&path.d);
        edges_out.push('"');
        edges_out.push_str(attrs);
        edges_out.push_str("/>");
        edges_out.push_str(&class_marker(
            start_type, path.sx, path.sy, path.sdx, path.sdy, LINE,
        ));
        edges_out.push_str(&class_marker(
            end_type, path.ex, path.ey, path.edx, path.edy, LINE,
        ));
        edges_out.push_str("</g>");
        if label.width > 0.0 {
            labels_out.push_str(&edge_label_svg(
                &edge.id,
                &[],
                label,
                ce.label_x + ox,
                ce.label_y + oy,
                "",
                None,
            ));
        }
    }

    let mut cards_out = String::new();
    for item in &cards {
        cards_out.push_str("<text class=\"ondine-cardinality\" x=\"");
        push_num(&mut cards_out, item.x + ox);
        cards_out.push_str("\" y=\"");
        push_num(&mut cards_out, item.y + oy + tiny * 0.35);
        cards_out.push_str("\" text-anchor=\"middle\">");
        cards_out.push_str(&esc(&item.text));
        cards_out.push_str("</text>");
    }

    let mut content = String::new();
    if !clusters.is_empty() {
        content.push_str("<g class=\"ondine-clusters\">");
        content.push_str(&clusters);
        content.push_str("</g>");
    }
    if !edges_out.is_empty() {
        content.push_str("<g class=\"ondine-edges\" fill=\"none\" stroke=\"");
        content.push_str(LINE);
        content.push_str("\" stroke-linecap=\"round\">");
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
        push_num(&mut content, small);
        content.push_str("\">");
        content.push_str(&labels_out);
        content.push_str("</g>");
    }
    if !cards_out.is_empty() {
        content.push_str("<g class=\"ondine-cardinalities\" font-size=\"");
        push_num(&mut content, tiny);
        content.push_str("\" fill=\"var(--_m)\">");
        content.push_str(&cards_out);
        content.push_str("</g>");
    }
    content.push_str("<g class=\"ondine-nodes\">");
    content.push_str(&nodes_out);
    content.push_str("</g>");

    let titled = with_title(db.title.as_deref(), size, pad, width, height, content);
    let accessible = Accessible {
        acc_descr: db.acc_descr.as_deref(),
        acc_title: db.acc_title.as_deref(),
    };
    let svg = svg_document(
        "class",
        titled.width,
        titled.height,
        size,
        options,
        accessible,
        &titled.content,
    );

    Rendered {
        height: titled.height,
        links,
        svg,
        width: titled.width,
    }
}

/// The attributes of a note's or an interface's label.
fn label_attrs(small: f64) -> String {
    let mut attrs = String::from(" class=\"ondine-label\" font-size=\"");
    push_num(&mut attrs, small);
    attrs.push('"');
    attrs
}

/// The diagram's font sizes that a class box reads.
struct Sizes {
    size: f64,
    tiny: f64,
    tiny_height: f64,
}

/// Measures a class box: its name, annotations, and member sections.
fn class_view(
    styler: &mut Styler<'_>,
    cls: &ClassNode,
    size: f64,
    tiny: f64,
    tiny_height: f64,
    hide_empty: bool,
) -> View {
    let style = styler.style(cls);
    let font_size = style.font_size.unwrap_or(size);
    let member_size = js_round(font_size * 0.875);
    let line_height = js_round(member_size * 1.5);
    // The generic type is escaped so that `<T>` is not read as a tag.
    let mut text = cls.label.clone();
    if !cls.r#type.is_empty() {
        text.push_str("&lt;");
        for c in parse_generic_types(&cls.r#type).chars() {
            match c {
                '<' => text.push_str("&lt;"),
                '>' => text.push_str("&gt;"),
                c => text.push(c),
            }
        }
        text.push_str("&gt;");
    }
    let label = layout_label(
        Some(&text),
        false,
        font_size,
        4000.0,
        BOLD | if style.italic { ITALIC } else { 0 },
        None,
    );
    let annotations: Vec<String> = cls
        .annotations
        .iter()
        .map(|a| {
            let decoded = entities::decode(a);
            let mut out = String::from("\u{ab}");
            out.push_str(trim(&decoded));
            out.push('\u{bb}');
            out
        })
        .collect();
    let attributes: Vec<Line> = cls
        .members
        .iter()
        .map(|m| member_line(m, member_size))
        .collect();
    let methods: Vec<Line> = cls
        .methods
        .iter()
        .map(|m| member_line(m, member_size))
        .collect();
    let mut inner = label.width;
    for a in &annotations {
        inner = inner.max(width(a, tiny, 0));
    }
    for line in attributes.iter().chain(&methods) {
        inner = inner.max(line.width);
    }
    let sections = !(hide_empty && attributes.is_empty() && methods.is_empty());
    let section = |count: usize| -> f64 {
        if !sections {
            0.0
        } else if count > 0 {
            2.0 * SECTION_PAD + to_f64(count) * line_height
        } else {
            EMPTY_SECTION
        }
    };
    let title_h = 2.0 * TITLE_PAD
        + to_f64(annotations.len()) * tiny_height
        + label.height.max(js_round(font_size * 1.5));
    let attributes_h = section(attributes.len());
    let h = title_h + attributes_h + section(methods.len());
    View {
        annotations,
        attributes,
        attributes_h,
        font_size,
        h,
        label,
        methods,
        style,
        title_h,
        w: (inner + 2.0 * PAD_X).max(MIN_WIDTH),
    }
}

/// Draws a class box.
#[expect(clippy::float_cmp, reason = "Pele compares the sizes exactly")]
fn class_svg(
    view: &View,
    cls: &ClassNode,
    id: &str,
    at: &str,
    sizes: &Sizes,
    url: &UrlOptions<'_>,
    links: &mut Vec<LinkInfo>,
) -> String {
    let (w, h, style) = (view.w, view.h, &view.style);
    let left = -w / 2.0;
    let top = -h / 2.0;
    let member_size = js_round(view.font_size * 0.875);
    let line_height = js_round(member_size * 1.5);

    let mut body = String::from("<rect x=\"");
    push_num(&mut body, left);
    body.push_str("\" y=\"");
    push_num(&mut body, top);
    body.push_str("\" width=\"");
    push_num(&mut body, w);
    body.push_str("\" height=\"");
    push_num(&mut body, h);
    body.push_str("\" rx=\"");
    body.push_str(RADIUS);
    body.push_str("\" fill=\"var(--_s)\" stroke=\"var(--_b)\"");
    body.push_str(&style.shape);
    body.push_str("/>");

    let first = top + view.title_h;
    let second = first + view.attributes_h;
    if h > view.title_h {
        body.push_str("<path class=\"ondine-divider\" d=\"M");
        push_num(&mut body, left);
        body.push(',');
        push_num(&mut body, first);
        body.push('H');
        push_num(&mut body, -left);
        body.push('M');
        push_num(&mut body, left);
        body.push(',');
        push_num(&mut body, second);
        body.push('H');
        push_num(&mut body, -left);
        body.push_str("\" fill=\"none\" stroke=\"var(--_b)\"");
        if let Some(stroke) = style.stroke.as_deref().filter(|s| !s.is_empty()) {
            body.push_str(" style=\"stroke:");
            body.push_str(&esc(stroke));
            body.push('"');
        }
        body.push_str("/>");
    }

    let mut y = top + TITLE_PAD;
    for annotation in &view.annotations {
        body.push_str("<text class=\"ondine-annotation\" x=\"0\" y=\"");
        push_num(&mut body, y + sizes.tiny_height / 2.0 + sizes.tiny * 0.35);
        body.push_str("\" text-anchor=\"middle\" font-size=\"");
        push_num(&mut body, sizes.tiny);
        body.push_str("\" fill=\"var(--_m)\">");
        body.push_str(&esc(annotation));
        body.push_str("</text>");
        y += sizes.tiny_height;
    }

    let mut title_attrs = String::from(" class=\"ondine-label\"");
    if view.font_size != sizes.size {
        title_attrs.push_str(" font-size=\"");
        push_num(&mut title_attrs, view.font_size);
        title_attrs.push('"');
    }
    title_attrs.push_str(" font-weight=\"var(--_hw)\"");
    title_attrs.push_str(&style.text);
    body.push_str(&label_svg(
        &view.label,
        0.0,
        (y + first - TITLE_PAD) / 2.0,
        &title_attrs,
        None,
    ));

    let lines = |list: &[Line], kind: &str, from: f64| -> String {
        let mut out = String::new();
        let mut baseline = from + SECTION_PAD + line_height / 2.0 + member_size * 0.35;
        for line in list {
            out.push_str("<text class=\"ondine-member ondine-");
            out.push_str(kind);
            match line.classifier.as_str() {
                "$" => {
                    out.push_str(" ondine-static\"");
                    if style.text.is_empty() {
                        out.push_str(" style=\"text-decoration:underline\"");
                    } else {
                        let mut text = style.text.clone();
                        text.pop();
                        out.push_str(&text);
                        out.push_str("text-decoration:underline\"");
                    }
                }
                "*" => {
                    out.push_str(" ondine-abstract\" font-style=\"italic\"");
                    out.push_str(&style.text);
                }
                _ => {
                    out.push('"');
                    out.push_str(&style.text);
                }
            }
            out.push_str(" x=\"");
            push_num(&mut out, left + PAD_X);
            out.push_str("\" y=\"");
            push_num(&mut out, baseline);
            out.push_str("\">");
            out.push_str(&esc(&line.text));
            out.push_str("</text>");
            baseline += line_height;
        }
        out
    };
    if !view.attributes.is_empty() || !view.methods.is_empty() {
        body.push_str("<g class=\"ondine-members\" font-size=\"");
        push_num(&mut body, member_size);
        body.push_str("\">");
        body.push_str(&lines(&view.attributes, "attribute", first));
        body.push_str(&lines(&view.methods, "method", second));
        body.push_str("</g>");
    }

    if let Some(tooltip) = cls.tooltip.as_deref().filter(|t| !t.is_empty()) {
        let mut titled = String::from("<title>");
        titled.push_str(&esc_text(tooltip));
        titled.push_str("</title>");
        titled.push_str(&body);
        body = titled;
    }
    if let Some(link) = cls.link.as_deref().filter(|l| !l.is_empty()) {
        let mut attrs = String::new();
        if let Some(target) = cls
            .link_target
            .as_deref()
            .filter(|target| TARGETS.contains(target))
        {
            attrs.push_str(" target=\"");
            attrs.push_str(target);
            attrs.push('"');
        }
        body = linked(&body, link, &cls.id, links, url, &attrs);
    }

    let mut out = String::from("<g class=\"ondine-node ondine-class");
    out.push_str(&class_names(strip_default(&cls.css_classes)));
    out.push_str("\" data-id=\"");
    out.push_str(id);
    out.push('"');
    out.push_str(at);
    out.push('>');
    out.push_str(&body);
    out.push_str("</g>");
    out
}
