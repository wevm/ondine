//! Lays out and draws a state diagram. Port of Pele's `src/diagrams/state/render.ts`.

use std::collections::BTreeMap;

use super::db::StateDb;
use super::graph::{StateEdge, StateNode};
use crate::diagrams::Rendered;
use crate::diagrams::common::fit_width::{runs_across, turn_to_fit};
use crate::layout::compound::{CEdge, CNode, Dir, cnode, compound_layout, direction};
use crate::layout::layered::LayeredOptions;
use crate::svg::builder::{esc, esc_text, js_round, label_svg, num, push_num};
use crate::svg::edges::{EdgePath, SideReach, edge_label_svg, marker, marker_trim, route_path};
use crate::svg::root::{Accessible, RenderOptions, svg_document};
use crate::svg::shapes::end_side;
use crate::svg::theme::{ResolvedStyle, class_names, resolve_style};
use crate::svg::title::with_title;
use crate::text::entities;
use crate::text::label::{Label, layout_label};
use crate::text::measure::{BOLD, ITALIC};
use crate::util::url::{self, LinkInfo, UrlOptions};
use crate::yaml::Value;

const NODE_SEP: f64 = 40.0;
const EDGE_SEP: f64 = 16.0;
const RANK_SEP: f64 = 44.0;
const GROUP_PAD: f64 = 16.0;
const REGION_PAD: f64 = 10.0;
const PAD_X: f64 = 16.0;
const PAD_Y: f64 = 10.0;
const LOOP: f64 = 26.0;
const FOLD: f64 = 8.0;
const NOTE_WRAP: f64 = 320.0;
const CORNER: f64 = 10.0;
const BAR: f64 = 8.0;
const SURFACE: &str = " fill=\"var(--_s)\" stroke=\"var(--_b)\"";
const RULE: &str = " fill=\"none\" stroke=\"var(--_b)\"";

/// What a node is drawn as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum K {
    Choice,
    Composite,
    Described,
    End,
    Fork,
    /// An unseen helper the layout needs, or a divider outside a body.
    Hidden,
    Join,
    Note,
    /// A part of a composite between dividers.
    Region,
    Start,
    State,
}

impl K {
    /// The classes of the drawn element, or `None` for kinds that draw no element of their own.
    fn classes(self) -> Option<&'static str> {
        match self {
            Self::State | Self::Described => Some("ondine-node ondine-state"),
            Self::Start => Some("ondine-node ondine-state-start"),
            Self::End => Some("ondine-node ondine-state-end"),
            Self::Choice => Some("ondine-node ondine-state-choice"),
            Self::Fork => Some("ondine-node ondine-state-fork"),
            Self::Join => Some("ondine-node ondine-state-join"),
            Self::Note => Some("ondine-note"),
            Self::Composite => Some("ondine-cluster ondine-state-composite"),
            Self::Region | Self::Hidden => None,
        }
    }

    /// Pele's `KINDS` lookup for a node that is not a container.
    fn of(shape: Option<&str>) -> Self {
        match shape {
            Some("rectWithTitle") => Self::Described,
            Some("stateStart") => Self::Start,
            Some("stateEnd") => Self::End,
            Some("choice") => Self::Choice,
            Some("fork") => Self::Fork,
            Some("join") => Self::Join,
            Some("note") => Self::Note,
            Some("divider") => Self::Hidden,
            _ => Self::State,
        }
    }
}

/// What the transitions and notes of one state add up to.
#[derive(Clone, Copy, Default)]
struct Stats {
    /// Transitions that reach the state.
    arriving: u32,
    /// The most transitions on one side: leaving or arriving.
    degree: u32,
    /// Transitions that leave the state.
    leaving: u32,
    /// How many transitions the state makes to itself.
    loop_count: u32,
    /// The height of their labels together.
    loop_height: f64,
    /// How many of them are drawn so far.
    loop_seen: u32,
    /// The height of the labels drawn so far.
    loop_text: f64,
    /// Their widest label.
    loop_width: f64,
    /// Whether a note sits beside the state.
    noted: bool,
    /// For a note, the state it belongs to.
    owner: Option<usize>,
}

/// A node as drawn, beside its layout box.
struct View<'g> {
    h: f64,
    kind: K,
    label: Label,
    lines: Vec<Label>,
    /// Absent for the unseen helpers the layout needs.
    node: Option<&'g StateNode>,
    style: Option<ResolvedStyle>,
    title_h: f64,
    w: f64,
}

fn view(node: Option<&StateNode>, kind: K) -> View<'_> {
    View {
        h: 0.0,
        kind,
        label: Label::default(),
        lines: Vec::new(),
        node,
        style: None,
        title_h: 0.0,
        w: 0.0,
    }
}

fn node_of<'g>(views: &[View<'g>], i: usize) -> Option<&'g StateNode> {
    views.get(i).and_then(|v| v.node)
}

/// As in Mermaid, a composite that sets no direction runs top to bottom whatever surrounds it.
fn inner_dir(views: &[View<'_>], composite: usize) -> Dir {
    direction(node_of(views, composite).and_then(|node| node.dir))
}

fn vertical(dir: Option<Dir>) -> bool {
    matches!(dir, Some(Dir::Tb | Dir::Bt))
}

/// The value under `key` of a YAML mapping.
fn get<'v>(value: &'v Value, key: &str) -> Option<&'v Value> {
    match value {
        Value::Object(entries) => entries.iter().find(|(own, _)| own == key).map(|(_, v)| v),
        _ => None,
    }
}

fn number_option(config: &Value, key: &str, fallback: f64) -> f64 {
    match get(config, "state").and_then(|section| get(section, key)) {
        Some(Value::Number(value)) if *value > 0.0 => value.min(2000.0),
        _ => fallback,
    }
}

/// Mermaid keeps the quotes of a click statement's strings and strips them when it draws.
fn unquote(text: &str) -> &str {
    text.trim_matches('"')
}

fn clamp(value: f64, min: f64, max: f64) -> f64 {
    if value < min {
        min
    } else if value > max {
        max
    } else {
        value
    }
}

/// JavaScript's `Math.sign`.
fn js_sign(value: f64) -> f64 {
    if value == 0.0 || value.is_nan() {
        value
    } else {
        value.signum()
    }
}

/// `value || 1`.
fn or_one(value: f64) -> f64 {
    if value == 0.0 || value.is_nan() {
        1.0
    } else {
        value
    }
}

#[expect(clippy::cast_precision_loss, reason = "counts stay far below 2^52")]
fn to_f64(value: usize) -> f64 {
    value as f64
}

fn to_parent(parent: Option<usize>) -> i32 {
    parent.and_then(|p| i32::try_from(p).ok()).unwrap_or(-1)
}

fn rect(x: f64, y: f64, w: f64, h: f64, rest: &str) -> String {
    let mut out = String::from("<rect x=\"");
    push_num(&mut out, x);
    out.push_str("\" y=\"");
    push_num(&mut out, y);
    out.push_str("\" width=\"");
    push_num(&mut out, w);
    out.push_str("\" height=\"");
    push_num(&mut out, h);
    out.push('"');
    out.push_str(rest);
    out.push_str("/>");
    out
}

fn group(dir: Dir, pad_x: f64, pad_top: f64, pad_bottom: f64) -> CNode {
    let mut c = cnode(0.0, 0.0, -1);
    c.is_group = true;
    c.dir = Some(dir);
    c.pad_x = pad_x;
    c.pad_top = pad_top;
    c.pad_bottom = pad_bottom;
    c
}

/// Removes Mermaid's own state classes, as Pele's
/// `/(?:^| )statediagram-(?:state|cluster-alt|cluster|note)(?= |$)/g` does.
fn strip_mermaid_classes(classes: &str) -> String {
    const PREFIX: &str = "statediagram-";
    let mut out = String::with_capacity(classes.len());
    let mut at = 0;
    let mut kept = 0;
    while at < classes.len() {
        let name = if at == 0 && classes.starts_with(PREFIX) {
            Some(PREFIX.len())
        } else if classes.get(at..).is_some_and(|rest| rest.starts_with(' ')) {
            classes
                .get(at + 1..)
                .filter(|rest| rest.starts_with(PREFIX))
                .map(|_| 1 + PREFIX.len())
        } else {
            None
        };
        let end = name.and_then(|skip| {
            let rest = classes.get(at + skip..)?;
            ["state", "cluster-alt", "cluster", "note"]
                .iter()
                .find(|word| {
                    rest.strip_prefix(**word)
                        .is_some_and(|after| after.is_empty() || after.starts_with(' '))
                })
                .map(|word| at + skip + word.len())
        });
        if let Some(end) = end {
            out.push_str(classes.get(kept..at).unwrap_or(""));
            kept = end;
            at = end;
        } else {
            at += classes
                .get(at..)
                .and_then(|rest| rest.chars().next())
                .map_or(1, char::len_utf8);
        }
    }
    out.push_str(classes.get(kept..).unwrap_or(""));
    out
}

/// Draws a state diagram, turning one that runs across to run down when it does not fit the
/// host's width.
pub(crate) fn render(
    db: &StateDb,
    config: &Value,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
) -> Rendered {
    if !runs_across(Some(db.direction())) {
        return draw(db, config, options, url, false);
    }
    turn_to_fit(options, |down| draw(db, config, options, url, down))
}

/// Pele's comparison of the order states take in their flow: initial states first, then the
/// order transitions introduce them, then declaration order.
fn by_flow(views: &[View<'_>], appear: &[f64], a: usize, b: usize) -> std::cmp::Ordering {
    let initial = |i: usize| u8::from(views.get(i).is_none_or(|v| v.kind != K::Start));
    let (ia, ib) = (initial(a), initial(b));
    if ia != ib {
        return ia.cmp(&ib);
    }
    let (pa, pb) = (
        appear.get(a).copied().unwrap_or(f64::INFINITY),
        appear.get(b).copied().unwrap_or(f64::INFINITY),
    );
    if pa < pb {
        return std::cmp::Ordering::Less;
    }
    if pa > pb {
        return std::cmp::Ordering::Greater;
    }
    a.cmp(&b)
}

/// `turned` draws a diagram that runs across as one that runs down.
#[expect(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    reason = "mirrors Pele's draw and its geometry names, one pass per drawing stage"
)]
fn draw(
    db: &StateDb,
    config: &Value,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
    turned: bool,
) -> Rendered {
    let graph = &db.graph;
    let size = options.font_size.unwrap_or(16.0);
    let small = js_round(size * 0.875);
    let wrap_width = number_option(config, "wrappingWidth", 200.0);
    let pad = options.padding.unwrap_or(8.0);
    let root_dir = if turned {
        Dir::Tb
    } else {
        direction(Some(db.direction()))
    };
    let text = |raw: Option<&str>, font_size: f64, wrap: f64, style: u8| {
        layout_label(raw, true, font_size, wrap, style, None)
    };

    let mut index: BTreeMap<&str, usize> = BTreeMap::new();
    let mut views: Vec<View<'_>> = Vec::new();
    let mut cnodes: Vec<CNode> = Vec::new();
    for node in &graph.nodes {
        if node.shape == Some("noteGroup") {
            continue;
        }
        let kind = if node.is_group {
            if node.shape == Some("divider") {
                K::Region
            } else {
                K::Composite
            }
        } else {
            K::of(node.shape)
        };
        index.insert(&node.id, views.len());
        views.push(view(Some(node), kind));
    }
    let states = views.len();

    let mut logical: Vec<Option<usize>> = (0..states)
        .map(|i| {
            node_of(&views, i)
                .and_then(|node| node.parent_id.as_deref())
                .and_then(|id| index.get(id).copied())
                .filter(|&parent| {
                    parent != i && node_of(&views, parent).is_some_and(|node| node.is_group)
                })
        })
        .collect();
    // Composites that are redeclared inside each other form a ring with no way in from the root.
    // One member of each ring moves to the root, which leaves a tree.
    {
        let mut kids: Vec<Vec<usize>> = vec![Vec::new(); states];
        let mut queue: Vec<usize> = Vec::new();
        let mut placed: Vec<bool> = logical.iter().map(Option::is_none).collect();
        for (i, parent) in logical.iter().enumerate() {
            match parent {
                None => queue.push(i),
                Some(parent) => {
                    if let Some(list) = kids.get_mut(*parent) {
                        list.push(i);
                    }
                }
            }
        }
        let mut done = 0;
        let settle = |queue: &mut Vec<usize>, placed: &mut [bool], done: &mut usize| {
            while let Some(&at) = queue.get(*done) {
                for &kid in kids.get(at).map_or(&[][..], Vec::as_slice) {
                    if let Some(slot) = placed.get_mut(kid)
                        && !*slot
                    {
                        *slot = true;
                        queue.push(kid);
                    }
                }
                *done += 1;
            }
        };
        settle(&mut queue, &mut placed, &mut done);
        let mut seen: Vec<Option<usize>> = vec![None; states];
        for i in 0..states {
            if placed.get(i).copied().unwrap_or(true) {
                continue;
            }
            let mut ring = i;
            while seen.get(ring).copied().flatten() != Some(i) {
                if let Some(slot) = seen.get_mut(ring) {
                    *slot = Some(i);
                }
                let Some(next) = logical.get(ring).copied().flatten() else {
                    break;
                };
                ring = next;
            }
            if let Some(slot) = logical.get_mut(ring) {
                *slot = None;
            }
            if let Some(slot) = placed.get_mut(ring) {
                *slot = true;
            }
            queue.push(ring);
            settle(&mut queue, &mut placed, &mut done);
        }
    }
    let mut stats = vec![Stats::default(); states];
    for (i, stat) in stats.iter_mut().enumerate() {
        let owner = node_of(&views, i)
            .and_then(|node| node.note_for.as_deref())
            .and_then(|id| index.get(id).copied());
        let Some(owner) = owner else { continue };
        stat.owner = Some(owner);
        let parent = logical.get(owner).copied().flatten();
        if let Some(slot) = logical.get_mut(i) {
            *slot = parent;
        }
    }

    // Whether the flow around each state runs top to bottom or bottom to top.
    let across: Vec<bool> = (0..states)
        .map(|i| {
            vertical(Some(
                logical
                    .get(i)
                    .copied()
                    .flatten()
                    .map_or(root_dir, |parent| inner_dir(&views, parent)),
            ))
        })
        .collect();
    let across_of = |i: usize| across.get(i).copied().unwrap_or(false);

    // When each state first takes part in a transition. Layout follows this order rather than
    // the order of declaration, so a state declared up front does not become the top of the flow.
    let mut appear: Vec<f64> = vec![f64::INFINITY; states];
    let mut appeared = 0.0;
    for edge in &graph.edges {
        let (Some(&src), Some(&dst)) =
            (index.get(edge.start.as_str()), index.get(edge.end.as_str()))
        else {
            continue;
        };
        if edge.pattern == Some("dashed") {
            let at = if views.get(src).is_some_and(|v| v.kind == K::Note) {
                dst
            } else {
                src
            };
            if let Some(stat) = stats.get_mut(at) {
                stat.noted = true;
            }
            continue;
        }
        if let Some(stat) = stats.get_mut(src) {
            stat.leaving += 1;
            stat.degree = stat.degree.max(stat.leaving);
        }
        if let Some(stat) = stats.get_mut(dst) {
            stat.arriving += 1;
            stat.degree = stat.degree.max(stat.arriving);
        }
        for end in [src, dst] {
            if let Some(at) = appear.get_mut(end)
                && *at == f64::INFINITY
            {
                *at = appeared;
                appeared += 1.0;
            }
        }
    }

    for i in 0..states {
        let dir = inner_dir(&views, i);
        let Some(view) = views.get_mut(i) else {
            continue;
        };
        let Some(node) = view.node else { continue };
        let style = if node.css_styles.is_empty() {
            resolve_style(&node.css_compiled_styles)
        } else {
            let mut all = node.css_compiled_styles.clone();
            all.extend(node.css_styles.iter().cloned());
            resolve_style(&all)
        };
        let text_style =
            (if style.bold { BOLD } else { 0 }) | (if style.italic { ITALIC } else { 0 });
        let font_size = style.font_size.unwrap_or(size);
        view.style = Some(style);
        let mut c: Option<CNode> = None;
        match view.kind {
            K::Composite => {
                view.label = text(node.label.as_deref(), font_size, 4000.0, text_style);
                view.title_h = if view.label.height > 0.0 {
                    view.label.height + 12.0
                } else {
                    0.0
                };
                let mut g = group(dir, GROUP_PAD, view.title_h + GROUP_PAD, GROUP_PAD);
                g.min_w = view.label.width + 2.0 * GROUP_PAD;
                c = Some(g);
            }
            K::Region => c = Some(group(dir, REGION_PAD, REGION_PAD, REGION_PAD)),
            K::Start => {
                view.w = 14.0;
                view.h = 14.0;
            }
            K::End => {
                view.w = 18.0;
                view.h = 18.0;
            }
            K::Choice => {
                view.w = 30.0;
                view.h = 30.0;
            }
            K::Fork | K::Join => {
                let degree = stats.get(i).map_or(0, |stat| stat.degree);
                let length = clamp(40.0 * f64::from(degree) + 16.0, 72.0, 320.0);
                view.w = if across_of(i) { length } else { BAR };
                view.h = if across_of(i) { BAR } else { length };
            }
            K::Hidden => {}
            K::Note => {
                view.label = text(node.label.as_deref(), small, wrap_width.max(NOTE_WRAP), 0);
                view.w = view.label.width + 24.0;
                view.h = view.label.height + 16.0;
            }
            K::State | K::Described => {
                view.label = text(node.label.as_deref(), font_size, wrap_width, text_style);
                let mut width = view.label.width;
                let mut height = view.label.height + 2.0 * PAD_Y;
                if view.kind == K::Described {
                    height += 14.0;
                    for line in &node.description {
                        let raw = if line.is_empty() { " " } else { line.as_str() };
                        let label = text(Some(raw), small, wrap_width, text_style);
                        if label.width > width {
                            width = label.width;
                        }
                        height += label.height;
                        view.lines.push(label);
                    }
                }
                view.w = (width + 2.0 * PAD_X).max(56.0);
                view.h = height;
            }
        }
        let mut c = c.unwrap_or_else(|| cnode(view.w, view.h, -1));
        // The transitions on one side of a state's box spread along it, clear of its corners.
        if matches!(view.kind, K::State | K::Described) {
            c.span = (if across_of(i) { view.w } else { view.h }) - 24.0;
        }
        cnodes.push(c);
    }

    let mut cedges: Vec<CEdge> = Vec::new();
    let mut edge_labels: Vec<Label> = Vec::new();
    let mut drawn: Vec<&StateEdge> = Vec::new();
    // A transition between a composite and a state inside it runs between that state and a point
    // just inside the composite, and is then drawn on to the composite's border.
    let mut anchors: BTreeMap<usize, usize> = BTreeMap::new();
    let encloses = |cnodes: &[CNode], composite: usize, item: usize| {
        if !cnodes.get(composite).is_some_and(|c| c.is_group) {
            return false;
        }
        let mut p = logical.get(item).copied().flatten();
        let mut hops = 0;
        while let Some(at) = p {
            if at == composite {
                return true;
            }
            hops += 1;
            if hops > states {
                break;
            }
            p = logical.get(at).copied().flatten();
        }
        false
    };
    for edge in &graph.edges {
        let (Some(&src0), Some(&dst0)) =
            (index.get(edge.start.as_str()), index.get(edge.end.as_str()))
        else {
            continue;
        };
        let (mut src, mut dst) = (src0, dst0);
        let label = text(Some(&edge.label), small, wrap_width, 0);
        let label_w = if label.width > 0.0 {
            label.width + 12.0
        } else {
            0.0
        };
        let enclosed_dst = src != dst && encloses(&cnodes, src, dst);
        let enclosed_src = src != dst && !enclosed_dst && encloses(&cnodes, dst, src);
        if src == dst {
            // The transitions each state makes to itself: how many, their widest label, and
            // their labels' height.
            if let Some(stat) = stats.get_mut(src) {
                stat.loop_count += 1;
                stat.loop_width = stat.loop_width.max(label_w);
                stat.loop_height += label.height;
            }
        } else if enclosed_dst || enclosed_src {
            let (composite, first) = if enclosed_dst {
                (src, true)
            } else {
                (dst, false)
            };
            anchors.insert(cedges.len(), composite);
            appear.push(if first { -1.0 } else { f64::INFINITY });
            let anchor = views.len();
            views.push(view(None, K::Hidden));
            cnodes.push(cnode(0.0, 0.0, to_parent(Some(composite))));
            if first {
                src = anchor;
            } else {
                dst = anchor;
            }
        }
        drawn.push(edge);
        cedges.push(CEdge {
            dst,
            label_h: if label.height > 0.0 {
                label.height + 4.0
            } else {
                0.0
            },
            label_w,
            label_x: 0.0,
            label_y: 0.0,
            minlen: 1,
            route: Vec::new(),
            src,
        });
        edge_labels.push(label);
    }

    // A state with notes, or a composite with self-transitions, sits in an unseen group of its
    // own: the notes go beside the state, across the flow, without disturbing its transitions.
    for (i, stat) in stats.iter().enumerate() {
        if stat.owner.is_some() {
            continue;
        }
        let across = across_of(i);
        let parent = to_parent(logical.get(i).copied().flatten());
        let Some(item) = cnodes.get_mut(i) else {
            continue;
        };
        // Loops hang off the right, or below when the flow is horizontal, with their labels
        // beyond them.
        let mut room = 0.0;
        if stat.loop_count > 0 {
            let reach = LOOP + f64::from(stat.loop_count - 1) * 8.0;
            room = if across {
                reach + stat.loop_width
            } else {
                reach * 0.75 + 4.0 + stat.loop_height
            };
            if !item.is_group {
                if across {
                    item.w += 2.0 * room;
                    item.h = item.h.max(stat.loop_height);
                } else {
                    item.h += 2.0 * room;
                    item.w = item.w.max(stat.loop_width);
                }
                room = 0.0;
            }
        }
        if !stat.noted && room == 0.0 {
            item.parent = parent;
            continue;
        }
        let mut c = if across {
            group(Dir::Lr, room, 0.0, 0.0)
        } else {
            group(Dir::Tb, 0.0, room, room)
        };
        c.parent = parent;
        appear.push(appear.get(i).copied().unwrap_or(f64::INFINITY));
        let holder = views.len();
        views.push(view(None, K::Hidden));
        cnodes.push(c);
        if let Some(item) = cnodes.get_mut(i) {
            item.parent = to_parent(Some(holder));
        }
    }
    for (i, stat) in stats.iter().enumerate() {
        if let Some(owner) = stat.owner {
            let parent = cnodes.get(owner).map_or(-1, |c| c.parent);
            if let Some(c) = cnodes.get_mut(i) {
                c.parent = parent;
            }
        }
    }

    {
        // Numbers the tree so that, at every level, initial states come first and the rest follow
        // in the order the transitions introduce them.
        let mut members: Vec<Vec<usize>> = vec![Vec::new(); cnodes.len()];
        let mut top: Vec<usize> = Vec::new();
        for (i, c) in cnodes.iter().enumerate() {
            match usize::try_from(c.parent)
                .ok()
                .and_then(|p| members.get_mut(p))
            {
                Some(list) => list.push(i),
                None => top.push(i),
            }
        }
        let mut seq = 0usize;
        top.sort_by(|&a, &b| by_flow(&views, &appear, a, b));
        top.reverse();
        let mut pending = top;
        while let Some(i) = pending.pop() {
            if let Some(c) = cnodes.get_mut(i) {
                c.seq = to_f64(seq);
            }
            seq += 1;
            let mut inside = members.get(i).cloned().unwrap_or_default();
            inside.sort_by(|&a, &b| by_flow(&views, &appear, a, b));
            pending.extend(inside.into_iter().rev());
        }
    }

    let layout = compound_layout(
        &mut cnodes,
        &mut cedges,
        root_dir,
        &LayeredOptions {
            apart: false,
            edge_sep: EDGE_SEP,
            head_room: marker_trim("arrow_point"),
            node_sep: number_option(config, "nodeSpacing", NODE_SEP),
            port_sep: 20.0,
            rank_sep: number_option(config, "rankSpacing", RANK_SEP),
            straight: false,
            tracks: false,
        },
    );

    let width = (layout.width + 2.0 * pad).ceil();
    let height = (layout.height + 2.0 * pad).ceil();
    let mut links: Vec<LinkInfo> = Vec::new();

    // Route points by rounded y, to find the edges that cross a composite's title bar.
    let mut crossings: BTreeMap<i64, Vec<f64>> = BTreeMap::new();
    for e in &cedges {
        let route = &e.route;
        let mut k = 3;
        while k + 3 < route.len() {
            if let (Some(&x), Some(&y)) = (route.get(k), route.get(k + 1)) {
                crossings.entry(key(y)).or_default().push(x);
            }
            k += 3;
        }
    }

    let mut children: Vec<Vec<usize>> = vec![Vec::new(); cnodes.len()];
    let mut order: Vec<usize> = Vec::new();
    for (i, c) in cnodes.iter().enumerate() {
        match usize::try_from(c.parent)
            .ok()
            .and_then(|p| children.get_mut(p))
        {
            Some(list) => list.push(i),
            None => order.push(i),
        }
    }
    let roots = order.clone();
    let mut q = 0;
    while let Some(&at) = order.get(q) {
        if let Some(list) = children.get(at) {
            order.extend(list.iter().copied());
        }
        q += 1;
    }

    let mut clusters = String::new();
    let mut dividers = String::new();
    let mut nodes_out = String::new();
    let mut notes_out = String::new();
    let frame = Frame {
        cnodes: &cnodes,
        pad,
        views: &views,
    };
    frame.divide(
        &mut dividers,
        &roots,
        pad / 2.0,
        pad / 2.0,
        width - pad / 2.0,
        height - pad / 2.0,
    );

    for &i in &order {
        let (Some(view), Some(c)) = (views.get(i), cnodes.get(i)) else {
            continue;
        };
        let (Some(node), Some(style), Some(classes)) =
            (view.node, &view.style, view.kind.classes())
        else {
            continue;
        };
        let x = c.x + pad;
        let y = c.y + pad;
        let (w, h) = (view.w, view.h);
        let mut label_attrs = String::from(" class=\"ondine-label\"");
        label_attrs.push_str(&style.text);
        let mut small_attrs = String::from(" font-size=\"");
        push_num(&mut small_attrs, small);
        small_attrs.push('"');

        let mut body = String::new();
        match view.kind {
            K::Composite => {
                let left = x - c.w / 2.0;
                let top = y - c.h / 2.0;
                let mut rest = String::from(
                    " rx=\"10\" fill=\"var(--_a)\" fill-opacity=\"0.5\" stroke=\"var(--_b)\"",
                );
                rest.push_str(&style.shape);
                body.push_str(&rect(left, top, c.w, c.h, &rest));
                if view.title_h > 0.0 {
                    body.push_str("<path d=\"M");
                    push_num(&mut body, left);
                    body.push(',');
                    push_num(&mut body, top + view.title_h);
                    body.push('H');
                    push_num(&mut body, left + c.w);
                    body.push('"');
                    body.push_str(RULE);
                    body.push_str(&style.line);
                    body.push_str("/>");
                    let mut attrs = String::from(" class=\"ondine-cluster-label\"");
                    attrs.push_str(&style.text);
                    body.push_str(&label_svg(
                        &view.label,
                        title_x(c, view.label.width, &crossings) + pad,
                        top + view.title_h / 2.0,
                        &attrs,
                        None,
                    ));
                }
                frame.divide(
                    &mut dividers,
                    children.get(i).map_or(&[][..], Vec::as_slice),
                    left,
                    top + view.title_h,
                    left + c.w,
                    top + c.h,
                );
            }
            K::Start => {
                body.push_str("<circle r=\"7\" fill=\"var(--_l)\"");
                body.push_str(&style.shape);
                body.push_str("/>");
            }
            K::End => {
                body.push_str("<circle r=\"8.5\" fill=\"var(--_bg)\" stroke=\"var(--_l)\"");
                body.push_str(&style.line);
                body.push_str("/><circle r=\"5\" fill=\"var(--_l)\"");
                body.push_str(&style.shape);
                body.push_str("/>");
            }
            K::Choice => {
                body.push_str("<polygon points=\"0,");
                push_num(&mut body, -h / 2.0);
                body.push(' ');
                push_num(&mut body, w / 2.0);
                body.push_str(",0 0,");
                push_num(&mut body, h / 2.0);
                body.push(' ');
                push_num(&mut body, -w / 2.0);
                body.push_str(",0\"");
                body.push_str(SURFACE);
                body.push_str(&style.shape);
                body.push_str("/>");
            }
            K::Fork | K::Join => {
                let mut rest = String::from(" rx=\"2\" fill=\"var(--_l)\"");
                rest.push_str(&style.shape);
                body.push_str(&rect(-w / 2.0, -h / 2.0, w, h, &rest));
            }
            K::Note => {
                let r = num(w / 2.0);
                let b = num(h / 2.0);
                let fold = num(w / 2.0 - FOLD);
                let corner = num(FOLD - h / 2.0);
                for part in [
                    "<path d=\"M-",
                    &r,
                    ",-",
                    &b,
                    "H",
                    &fold,
                    "L",
                    &r,
                    ",",
                    &corner,
                    "V",
                    &b,
                    "H-",
                    &r,
                    "Z\" fill=\"var(--_bg)\" stroke=\"var(--_b)\"/><path d=\"M",
                    &fold,
                    ",-",
                    &b,
                    "V",
                    &corner,
                    "H",
                    &r,
                    "\"",
                    RULE,
                    "/>",
                ] {
                    body.push_str(part);
                }
                let mut attrs = String::from(" class=\"ondine-label\"");
                attrs.push_str(&small_attrs);
                attrs.push_str(" fill=\"var(--_m)\"");
                body.push_str(&label_svg(&view.label, 0.0, 0.0, &attrs, None));
            }
            K::State | K::Described | K::Region | K::Hidden => {
                let mut rest = String::from(" rx=\"");
                push_num(&mut rest, CORNER.min(h / 2.0));
                rest.push('"');
                rest.push_str(SURFACE);
                rest.push_str(&style.shape);
                body.push_str(&rect(-w / 2.0, -h / 2.0, w, h, &rest));
                if view.kind == K::State {
                    body.push_str(&label_svg(&view.label, 0.0, 0.0, &label_attrs, None));
                } else {
                    let mut at = PAD_Y - h / 2.0 + view.label.height;
                    body.push_str("<path d=\"M");
                    push_num(&mut body, -w / 2.0);
                    body.push(',');
                    push_num(&mut body, at + 6.0);
                    body.push('H');
                    push_num(&mut body, w / 2.0);
                    body.push('"');
                    body.push_str(RULE);
                    body.push_str(&style.line);
                    body.push_str("/>");
                    body.push_str(&label_svg(
                        &view.label,
                        0.0,
                        at - view.label.height / 2.0,
                        &label_attrs,
                        None,
                    ));
                    at += 14.0;
                    let mut attrs = String::from(" class=\"ondine-state-description\"");
                    attrs.push_str(&small_attrs);
                    attrs.push_str(&style.text);
                    for line in &view.lines {
                        body.push_str(&label_svg(line, 0.0, at + line.height / 2.0, &attrs, None));
                        at += line.height;
                    }
                }
            }
        }

        if let Some(link) = db.link(&node.id) {
            // Entity codes are resolved first, so the URL is checked in the form a browser would
            // follow.
            let tooltip = entities::decode(unquote(&link.tooltip));
            if !tooltip.is_empty() {
                let mut titled = String::from("<title>");
                titled.push_str(&esc(&tooltip));
                titled.push_str("</title>");
                titled.push_str(&body);
                body = titled;
            }
            body = url::linked(
                &body,
                &url::safe_url(unquote(&link.url)),
                &node.id,
                &mut links,
                url,
                " target=\"_blank\"",
            );
        }

        let mut open = String::from("<g class=\"");
        open.push_str(classes);
        open.push_str(&class_names(&strip_mermaid_classes(&node.css_classes)));
        open.push_str("\" data-id=\"");
        open.push_str(&esc_text(&node.id));
        open.push('"');
        let out = match view.kind {
            K::Composite => &mut clusters,
            K::Note => &mut notes_out,
            _ => &mut nodes_out,
        };
        out.push_str(&open);
        if view.kind != K::Composite {
            out.push_str(" transform=\"translate(");
            push_num(out, x);
            out.push(',');
            push_num(out, y);
            out.push(')');
            out.push('"');
        }
        out.push('>');
        out.push_str(&body);
        out.push_str("</g>");
    }

    let mut reach = SideReach::default();
    for (ce, edge) in cedges.iter().zip(&drawn) {
        if ce.src == ce.dst || ce.route.len() < 6 || edge.pattern == Some("dashed") {
            continue;
        }
        if let Some(dst) = cnodes.get(ce.dst) {
            reach.add(
                ce.dst,
                end_side(&ce.route, ce.route.len() - 3, dst),
                "arrow_point",
            );
        }
    }

    let mut edges_out = String::new();
    let mut labels_out = String::new();
    for i in 0..cedges.len() {
        let (Some(edge), Some(label)) = (drawn.get(i), edge_labels.get(i)) else {
            continue;
        };
        let note = edge.pattern == Some("dashed");
        let end_type = if note { "none" } else { "arrow_point" };
        let Some(ce) = cedges.get(i) else { continue };
        let (src, dst) = (ce.src, ce.dst);
        let path: EdgePath = if src == dst {
            let (Some(a), Some(view), Some(stat)) =
                (cnodes.get(src), views.get(src), stats.get_mut(src))
            else {
                continue;
            };
            let k = f64::from(stat.loop_seen);
            stat.loop_seen += 1;
            // The loop hangs off a side the flow does not use: the right, or below when the flow
            // is horizontal.
            let side = across_of(src);
            let w = if a.is_group { a.w } else { view.w };
            let h = if a.is_group { a.h } else { view.h };
            let out = (if side { w } else { h }) / 2.0;
            let spread = ((if side { h } else { w }) / 2.0 - 4.0)
                .min(8.0 + k * 6.0)
                .max(2.0);
            let loop_reach = LOOP + k * 8.0;
            let trim = marker_trim(end_type) / or_one(loop_reach.hypot(spread));
            // Offsets are given for a loop on the right and turned a quarter for one below.
            let px = |dx: f64, dy: f64| a.x + pad + if side { out + dx } else { -dy };
            let py = |dx: f64, dy: f64| a.y + pad + if side { dy } else { out + dx };
            let at = |d: &mut String, dx: f64, dy: f64| {
                push_num(d, px(dx, dy));
                d.push(',');
                push_num(d, py(dx, dy));
            };
            let mut d = String::from("M");
            at(&mut d, 0.0, -spread);
            d.push('C');
            at(&mut d, loop_reach, -2.0 * spread);
            d.push(' ');
            at(&mut d, loop_reach, 2.0 * spread);
            d.push(' ');
            at(&mut d, loop_reach * trim, spread + spread * trim);
            let edx = px(-loop_reach, -spread) - px(0.0, 0.0);
            let edy = py(-loop_reach, -spread) - py(0.0, 0.0);
            let len = or_one(edx.hypot(edy));
            let path = EdgePath {
                d,
                edx: edx / len,
                edy: edy / len,
                ex: px(0.0, spread),
                ey: py(0.0, spread),
                sdx: 0.0,
                sdy: 0.0,
                sx: 0.0,
                sy: 0.0,
            };
            // Labels stack beyond the outermost loop, in the order of their transitions.
            let furthest = LOOP + f64::from(stat.loop_count.saturating_sub(1)) * 8.0;
            let below = stat.loop_text + label.height / 2.0;
            stat.loop_text += label.height;
            let loop_height = stat.loop_height;
            let (ax, ay) = (a.x, a.y);
            if let Some(ce) = cedges.get_mut(i) {
                ce.label_x = if side {
                    ax + out + furthest + 4.0 + ce.label_w / 2.0
                } else {
                    ax
                };
                ce.label_y = if side {
                    ay - loop_height / 2.0 + below
                } else {
                    ay + out + furthest * 0.75 + 4.0 + below
                };
            }
            path
        } else {
            let (Some(src_node), Some(dst_node)) = (cnodes.get(src), cnodes.get(dst)) else {
                continue;
            };
            let mut route = ce.route.clone();
            let last = route.len().saturating_sub(3);
            let leaves = reach.of(src, end_side(&route, 0, src_node));
            let arrives = reach.of(dst, end_side(&route, last, dst_node));
            for point in route.chunks_exact_mut(3) {
                if let [x, y, _] = point {
                    *x += pad;
                    *y += pad;
                }
            }
            frame.attach(&mut route, 0, src);
            frame.attach(&mut route, last, dst);
            if let Some(&composite) = anchors.get(&i)
                && route.len() >= 6
                && let (Some(g), Some(owner)) = (cnodes.get(composite), views.get(composite))
            {
                let inward = views.get(src).is_some_and(|v| v.kind == K::Hidden);
                let at = if inward { 0 } else { route.len() - 3 };
                let near = matches!(g.dir, Some(Dir::Tb | Dir::Lr)) == inward;
                let point = if vertical(g.dir) {
                    [
                        route.get(at).copied().unwrap_or(f64::NAN),
                        g.y + pad
                            + if near {
                                owner.title_h - g.h / 2.0
                            } else {
                                g.h / 2.0
                            },
                        0.0,
                    ]
                } else {
                    [
                        g.x + pad + if near { -g.w / 2.0 } else { g.w / 2.0 },
                        route.get(at + 1).copied().unwrap_or(f64::NAN),
                        1.0,
                    ]
                };
                if inward {
                    route.splice(0..0, point);
                } else {
                    route.extend(point);
                }
            }
            route_path(
                &route,
                None,
                0.0,
                marker_trim(end_type),
                Some(leaves),
                Some(arrives),
            )
        };

        edges_out.push_str(if note {
            "<g class=\"ondine-edge ondine-note-edge\" stroke=\"var(--_b)\" stroke-dasharray=\"3 4\" data-id=\""
        } else {
            "<g class=\"ondine-edge ondine-transition\" data-id=\""
        });
        edges_out.push_str(&esc_text(&edge.id));
        edges_out.push_str("\"><path d=\"");
        edges_out.push_str(&path.d);
        edges_out.push_str("\"/>");
        edges_out.push_str(&marker(
            end_type,
            path.ex,
            path.ey,
            path.edx,
            path.edy,
            "var(--_l)",
        ));
        edges_out.push_str("</g>");
        if label.width > 0.0
            && let Some(ce) = cedges.get(i)
        {
            labels_out.push_str(&edge_label_svg(
                &edge.id,
                &[],
                label,
                ce.label_x + pad,
                ce.label_y + pad,
                "",
                None,
            ));
        }
    }

    let mut content = String::new();
    if !clusters.is_empty() {
        content.push_str("<g class=\"ondine-clusters\">");
        content.push_str(&clusters);
        content.push_str("</g>");
    }
    if !dividers.is_empty() {
        content.push_str("<g class=\"ondine-dividers\" stroke-dasharray=\"4 4\"");
        content.push_str(RULE);
        content.push('>');
        content.push_str(&dividers);
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
        push_num(&mut content, small);
        content.push_str("\">");
        content.push_str(&labels_out);
        content.push_str("</g>");
    }
    content.push_str("<g class=\"ondine-nodes\">");
    content.push_str(&nodes_out);
    content.push_str(&notes_out);
    content.push_str("</g>");

    let titled = with_title(db.title.as_deref(), size, pad, width, height, content);
    let accessible = Accessible {
        acc_descr: db.acc_descr.as_deref(),
        acc_title: db.acc_title.as_deref(),
    };
    let svg = svg_document(
        "state",
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

/// A rounded coordinate as a map key.
#[expect(
    clippy::cast_possible_truncation,
    reason = "layout coordinates stay far inside i64"
)]
fn key(value: f64) -> i64 {
    js_round(value) as i64
}

/// The laid-out boxes the drawing helpers read.
struct Frame<'a> {
    cnodes: &'a [CNode],
    pad: f64,
    views: &'a [View<'a>],
}

impl Frame<'_> {
    /// Draws the dashed lines between the regions among a composite's members.
    fn divide(
        &self,
        out: &mut String,
        items: &[usize],
        left: f64,
        top: f64,
        right: f64,
        bottom: f64,
    ) {
        let regions: Vec<usize> = items
            .iter()
            .copied()
            .filter(|&i| self.views.get(i).is_some_and(|v| v.kind == K::Region))
            .collect();
        if regions.len() < 2 {
            return;
        }
        let lo = |i: usize, x: bool| {
            self.cnodes.get(i).map_or(
                f64::NAN,
                |c| if x { c.x - c.w / 2.0 } else { c.y - c.h / 2.0 },
            )
        };
        let hi = |i: usize, x: bool| {
            self.cnodes.get(i).map_or(
                f64::NAN,
                |c| if x { c.x + c.w / 2.0 } else { c.y + c.h / 2.0 },
            )
        };
        for x in [true, false] {
            let mut sorted = regions.clone();
            sorted.sort_by(|&a, &b| {
                let d = lo(a, x) - lo(b, x);
                if d == 0.0 || d.is_nan() {
                    a.cmp(&b)
                } else if d < 0.0 {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                }
            });
            let pairs = || sorted.iter().zip(sorted.iter().skip(1));
            // Pele asks whether any region reaches past the next, so `NaN` counts as apart.
            let apart = pairs().all(|(&a, &b)| {
                hi(a, x).partial_cmp(&(lo(b, x) + 0.5)) != Some(std::cmp::Ordering::Greater)
            });
            if !apart {
                continue;
            }
            for (&a, &b) in pairs() {
                let at = num(hi(a, x).midpoint(lo(b, x)) + self.pad);
                out.push_str("<path class=\"ondine-divider\" d=\"M");
                if x {
                    out.push_str(&at);
                    out.push(',');
                    push_num(out, top);
                    out.push('V');
                    push_num(out, bottom);
                } else {
                    push_num(out, left);
                    out.push(',');
                    out.push_str(&at);
                    out.push('H');
                    push_num(out, right);
                }
                out.push_str("\"/>");
            }
            return;
        }
        // Regions that sit neither side by side nor in a stack each get an outline.
        for &i in &regions {
            if let Some(c) = self.cnodes.get(i) {
                out.push_str(&rect(
                    c.x - c.w / 2.0 + self.pad,
                    c.y - c.h / 2.0 + self.pad,
                    c.w,
                    c.h,
                    " class=\"ondine-divider\" rx=\"10\"",
                ));
            }
        }
    }

    /// Moves a route end from the layout box onto the drawn shape, and spreads the ends along a
    /// bar.
    fn attach(&self, route: &mut [f64], at: usize, i: usize) {
        let (Some(c), Some(view)) = (self.cnodes.get(i), self.views.get(i)) else {
            return;
        };
        if c.is_group {
            return;
        }
        let bar = matches!(view.kind, K::Fork | K::Join);
        let other = if at == 0 { 3 } else { at - 3 };
        // 1 is the y axis and 0 the x axis. The end sits on a side that faces along `axis`.
        for axis in [1, 0] {
            let y = axis == 1;
            let extent = if y { c.h } else { c.w };
            let drawn_extent = if y { view.h } else { view.w };
            let breadth = if y { view.w } else { view.h };
            let center = (if y { c.y } else { c.x }) + self.pad;
            let cross = (if y { c.x } else { c.y }) + self.pad;
            let Some(&value) = route.get(at + axis) else {
                return;
            };
            let delta = value - center;
            if (delta.abs() - extent / 2.0).abs() >= 0.5 {
                continue;
            }
            if let Some(slot) = route.get_mut(at + axis) {
                *slot -= js_sign(delta) * (extent - drawn_extent) / 2.0;
            }
            if bar && breadth > drawn_extent {
                let from = route.get(other + 1 - axis).copied().unwrap_or(f64::NAN);
                if let Some(slot) = route.get_mut(at + 1 - axis) {
                    *slot = clamp(
                        from,
                        cross - breadth / 2.0 + 6.0,
                        cross + breadth / 2.0 - 6.0,
                    );
                }
            }
            return;
        }
    }
}

/// Picks where a composite's title sits in its title bar so that edges entering from above do
/// not cross it.
fn title_x(c: &CNode, width: f64, crossings: &BTreeMap<i64, Vec<f64>>) -> f64 {
    let Some(xs) = crossings.get(&key(c.y - c.h / 2.0)) else {
        return c.x;
    };
    if width == 0.0 {
        return c.x;
    }
    let left = c.x - c.w / 2.0;
    let half = width / 2.0;
    let mut best = c.x;
    let mut best_gap = -1.0;
    for x in [c.x, left + GROUP_PAD + half, left + c.w - GROUP_PAD - half] {
        let mut gap = f64::INFINITY;
        for &cx in xs {
            if cx > left && cx < left + c.w {
                gap = gap.min((cx - x).abs() - half);
            }
        }
        if gap >= 6.0 {
            return x;
        }
        if gap > best_gap + 0.5 {
            best_gap = gap;
            best = x;
        }
    }
    best
}
