//! Draws an architecture diagram: services and junctions on a grid, groups around them, and
//! orthogonal edges between their sides. Port of Pele's `src/diagrams/architecture/render.ts`.

use super::db::{ArchitectureDb, NodeKind};
use super::icons::builtin_icon;
use super::layout::{place_on_grid, side_index};
use crate::diagrams::Rendered;
use crate::diagrams::common::fit_width::{tighten, title_room};
use crate::svg::builder::{esc_text, js_round, label_svg, push_num};
use crate::svg::edges::{edge_label_svg, marker, marker_trim};
use crate::svg::root::{Accessible, RenderOptions, svg_document};
use crate::svg::theme::RADIUS;
use crate::text::label::{Label, Span, layout_label, trim_end};
use crate::text::measure::{self, BOLD};
use crate::yaml::{Value, push_number};

const STUB: f64 = 12.0;
const CORNER: f64 = 6.0;
const LABEL_GAP: f64 = 4.0;
const DOT: f64 = 3.0;
const DX: [f64; 4] = [-1.0, 1.0, 0.0, 0.0];
const DY: [f64; 4] = [0.0, 0.0, -1.0, 1.0];

fn at<T: Copy + Default>(values: &[T], i: usize) -> T {
    values.get(i).copied().unwrap_or_default()
}

/// Raises `values[i]` to at least `value`.
fn raise(values: &mut [f64], i: usize, value: f64) {
    if let Some(slot) = values.get_mut(i) {
        *slot = slot.max(value);
    }
}

fn set<T>(values: &mut [T], i: usize, value: T) {
    if let Some(slot) = values.get_mut(i) {
        *slot = value;
    }
}

/// A finite number of the `architecture` section of `config`, clamped to `min` and `max`, or
/// `fallback`.
fn setting(config: &Value, key: &str, fallback: f64, min: f64, max: f64) -> f64 {
    let Value::Object(entries) = config else {
        return fallback;
    };
    let Some((_, Value::Object(section))) = entries.iter().find(|(own, _)| own == "architecture")
    else {
        return fallback;
    };
    match section.iter().find(|(own, _)| own == key) {
        Some((_, Value::Number(value))) if value.is_finite() => value.max(min).min(max),
        _ => fallback,
    }
}

/// Maps the grid lines in use to consecutive indexes, so empty rows and columns take no room.
fn rank(mut values: Vec<i64>) -> Vec<i64> {
    values.sort_unstable();
    values.dedup();
    values
}

/// The index of `value` in a [`rank`].
fn rank_of(ranks: &[i64], value: i64) -> Option<usize> {
    ranks.binary_search(&value).ok()
}

fn icon_slot(out: &mut String, name: &str, x: f64, y: f64, side: f64) {
    out.push_str("<svg class=\"ondine-icon\" data-icon=\"");
    out.push_str(&esc_text(name));
    out.push_str("\" x=\"");
    push_num(out, x);
    out.push_str("\" y=\"");
    push_num(out, y);
    out.push_str("\" width=\"");
    push_num(out, side);
    out.push_str("\" height=\"");
    push_num(out, side);
    out.push_str(
        "\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\">",
    );
    // Icons need a host resolver, which Ondine does not take yet, so only built-ins draw.
    out.push_str(&builtin_icon(name));
    out.push_str("</svg>");
}

/// The width of the first `count` UTF-16 units of `units`. A cut inside a surrogate pair
/// measures as the whole pair, as Pele's measurer counts the high surrogate as the wide half.
fn prefix_width(units: &[u16], count: usize, size: f64, style: u8) -> f64 {
    let mut end = count.min(units.len());
    if end > 0
        && (0xd800..=0xdbff).contains(&at(units, end - 1))
        && (0xdc00..=0xdfff).contains(&at(units, end))
    {
        end += 1;
    }
    measure::width(
        &String::from_utf16_lossy(units.get(..end).unwrap_or(&[])),
        size,
        style,
    )
}

/// Cuts a label down to a box, ending what is left with an ellipsis.
fn fit(label: Label, max_width: f64, max_lines: usize) -> Label {
    let cut = label.lines.len() > max_lines;
    if !cut && label.width <= max_width {
        return label;
    }
    let dots = measure::width("…", label.size, 0);
    let mut lines: Vec<Vec<Span>> = Vec::new();
    let mut widths: Vec<f64> = Vec::new();
    let mut widest: f64 = 0.0;
    for (i, source) in label.lines.iter().take(max_lines).enumerate() {
        let mut line = source.clone();
        let mut width = at(&label.widths, i);
        if width > max_width || (cut && i + 1 == max_lines) {
            loop {
                let Some(span) = line.last_mut() else {
                    line.push(Span {
                        icon: None,
                        style: 0,
                        text: String::from("…"),
                        width: dots,
                    });
                    width = dots;
                    break;
                };
                let rest = width - span.width;
                if span.icon.is_none() && rest + dots < max_width {
                    let units: Vec<u16> = span.text.encode_utf16().collect();
                    let mut lo = 0;
                    let mut hi = units.len();
                    while lo < hi {
                        let mid = (lo + hi + 1) >> 1;
                        if rest + prefix_width(&units, mid, label.size, span.style) + dots
                            <= max_width
                        {
                            lo = mid;
                        } else {
                            hi = mid - 1;
                        }
                    }
                    if lo > 0 && (0xd800..=0xdbff).contains(&at(&units, lo - 1)) {
                        lo -= 1;
                    }
                    let kept = String::from_utf16_lossy(units.get(..lo).unwrap_or(&[]));
                    let mut text = trim_end(&kept).to_owned();
                    text.push('…');
                    span.width = measure::width(&text, label.size, span.style);
                    span.text = text;
                    width = rest + span.width;
                    break;
                }
                line.pop();
                width = rest;
            }
        }
        lines.push(line);
        widths.push(width);
        widest = widest.max(width);
    }
    #[expect(clippy::cast_precision_loss, reason = "line counts are small")]
    let height = lines.len() as f64 * label.line_height;
    Label {
        height,
        lines,
        width: widest,
        widths,
        ..label
    }
}

/// Where an edge leaves or enters a node.
#[derive(Clone, Copy, Debug, Default)]
struct Port {
    x: f64,
    y: f64,
    dx: f64,
    dy: f64,
    /// The box the route has to stay out of when it cannot leave straight.
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    /// How far short of the port an arrowhead stops: the radius of a junction's dot.
    gap: f64,
    /// Set where a junction is reached through the border of its group: the line carries on to
    /// the junction.
    wire: bool,
}

/// An orthogonal route from one port to another, each left along its side's normal.
#[expect(clippy::float_cmp, reason = "directions are exact unit steps")]
fn route(s: &Port, e: &Port) -> Vec<f64> {
    let (sx, sy, sdx, sdy) = (s.x, s.y, s.dx, s.dy);
    let (ex, ey, edx, edy) = (e.x, e.y, e.dx, e.dy);
    let s1x = sx + sdx * STUB;
    let s1y = sy + sdy * STUB;
    let e1x = ex + edx * STUB;
    let e1y = ey + edy * STUB;
    if sdx * edx + sdy * edy == 0.0 {
        let cx = if sdx == 0.0 { sx } else { ex };
        let cy = if sdx == 0.0 { ey } else { sy };
        if (cx - sx) * sdx + (cy - sy) * sdy > 0.0 && (cx - ex) * edx + (cy - ey) * edy > 0.0 {
            return vec![sx, sy, cx, cy, ex, ey];
        }
        return if sdx == 0.0 {
            vec![sx, sy, s1x, s1y, e1x, s1y, e1x, e1y, ex, ey]
        } else {
            vec![sx, sy, s1x, s1y, s1x, e1y, e1x, e1y, ex, ey]
        };
    }
    let sideways = sdx != 0.0;
    let offset = if sideways { ey - sy } else { ex - sx };
    if sdx == -edx && sdy == -edy {
        let ahead = (ex - sx) * sdx + (ey - sy) * sdy;
        if ahead > 0.0 {
            if offset.abs() < 0.5 {
                return vec![sx, sy, ex, ey];
            }
            // Turn midway between the two boxes, which is clear of both titles, when there is
            // such a place.
            if sideways {
                let lo = if sdx > 0.0 { s.x1 } else { e.x1 };
                let hi = if sdx > 0.0 { e.x0 } else { s.x0 };
                let mx = if lo < hi {
                    f64::midpoint(lo, hi)
                } else {
                    f64::midpoint(sx, ex)
                };
                return vec![sx, sy, mx, sy, mx, ey, ex, ey];
            }
            let lo = if sdy > 0.0 { s.y1 } else { e.y1 };
            let hi = if sdy > 0.0 { e.y0 } else { s.y0 };
            let my = if lo < hi {
                f64::midpoint(lo, hi)
            } else {
                f64::midpoint(sy, ey)
            };
            return vec![sx, sy, sx, my, ex, my, ex, ey];
        }
    } else if offset.abs() >= 1.0 {
        if sideways {
            let x = if sdx > 0.0 {
                sx.max(ex) + STUB
            } else {
                sx.min(ex) - STUB
            };
            return vec![sx, sy, x, sy, x, ey, ex, ey];
        }
        let y = if sdy > 0.0 {
            sy.max(ey) + STUB
        } else {
            sy.min(ey) - STUB
        };
        return vec![sx, sy, sx, y, ex, y, ex, ey];
    }
    // The straight way is blocked by one of the two nodes: go between them, or around both.
    if sideways {
        let lane = if s.y1 + STUB <= e.y0 {
            f64::midpoint(s.y1, e.y0)
        } else if e.y1 + STUB <= s.y0 {
            f64::midpoint(e.y1, s.y0)
        } else {
            s.y1.max(e.y1) + STUB
        };
        return vec![sx, sy, s1x, sy, s1x, lane, e1x, lane, e1x, ey, ex, ey];
    }
    let lane = if s.x1 + STUB <= e.x0 {
        f64::midpoint(s.x1, e.x0)
    } else if e.x1 + STUB <= s.x0 {
        f64::midpoint(e.x1, s.x0)
    } else {
        s.x1.max(e.x1) + STUB
    };
    vec![sx, sy, sx, s1y, lane, s1y, lane, e1y, ex, e1y, ex, ey]
}

/// Drops repeated points and joins runs that continue in a straight line.
fn simplify(p: &[f64]) -> Vec<f64> {
    let mut out = vec![at(p, 0), at(p, 1)];
    for pair in p.chunks(2).skip(1) {
        let x = at(pair, 0);
        let y = pair.get(1).copied().unwrap_or(f64::NAN);
        let n = out.len();
        let same_x = (x - at(&out, n - 2)).abs() < 0.01;
        let same_y = (y - at(&out, n - 1)).abs() < 0.01;
        if same_x && same_y {
            continue;
        }
        if n >= 4
            && ((same_x && (at(&out, n - 4) - at(&out, n - 2)).abs() < 0.01)
                || (same_y && (at(&out, n - 3) - at(&out, n - 1)).abs() < 0.01))
        {
            set(&mut out, n - 2, x);
            set(&mut out, n - 1, y);
        } else {
            out.push(x);
            out.push(y);
        }
    }
    out
}

fn push_point(out: &mut String, x: f64, y: f64) {
    push_num(out, x);
    out.push(',');
    push_num(out, y);
}

/// Appends the path through points `p`, with rounded corners and its ends trimmed for markers.
/// `p` holds at least two points.
#[expect(
    clippy::many_single_char_names,
    reason = "names mirror Pele's pathData"
)]
fn path_data(out: &mut String, p: &[f64], start_trim: f64, end_trim: f64) {
    let n = p.len() / 2;
    let mut xs: Vec<f64> = p.iter().step_by(2).copied().collect();
    let mut ys: Vec<f64> = p.iter().skip(1).step_by(2).copied().collect();
    let mut trim = |at_: usize, toward: usize, amount: f64| {
        let (ax, ay, tx, ty) = (at(&xs, at_), at(&ys, at_), at(&xs, toward), at(&ys, toward));
        let len = (tx - ax).hypot(ty - ay);
        let t = amount.min(0.0_f64.max(len - 1.0));
        if len > 0.0 && t > 0.0 {
            set(&mut xs, at_, ax + (tx - ax) / len * t);
            set(&mut ys, at_, ay + (ty - ay) / len * t);
        }
    };
    trim(0, 1, start_trim);
    trim(n - 1, n - 2, end_trim);
    out.push('M');
    push_point(out, at(&xs, 0), at(&ys, 0));
    for i in 1..n - 1 {
        let (px, py) = (at(&xs, i - 1), at(&ys, i - 1));
        let (x, y) = (at(&xs, i), at(&ys, i));
        let (nx, ny) = (at(&xs, i + 1), at(&ys, i + 1));
        let before = (x - px).hypot(y - py);
        let after = (nx - x).hypot(ny - y);
        let r = CORNER.min(before / 2.0).min(after / 2.0);
        if r < 0.5 {
            out.push('L');
            push_point(out, x, y);
            continue;
        }
        let in_x = x - (x - px) / before * r;
        let in_y = y - (y - py) / before * r;
        let out_x = x + (nx - x) / after * r;
        let out_y = y + (ny - y) / after * r;
        out.push('L');
        push_point(out, in_x, in_y);
        out.push('Q');
        push_point(out, x, y);
        out.push(' ');
        push_point(out, out_x, out_y);
    }
    out.push('L');
    push_point(out, at(&xs, n - 1), at(&ys, n - 1));
}

/// Draws `model` with the `architecture` section of `config`, tightened to fit the host's width.
pub(crate) fn render_architecture(
    model: &ArchitectureDb,
    config: &Value,
    options: &RenderOptions,
) -> Rendered {
    tighten(options, |tight| draw(model, config, options, tight))
}

/// Draws `model` once, tightly spaced when `tight`.
#[expect(clippy::many_single_char_names, reason = "names mirror Pele's draw")]
fn draw(model: &ArchitectureDb, config: &Value, options: &RenderOptions, tight: bool) -> Rendered {
    let icon_size = setting(config, "iconSize", 48.0, 8.0, 512.0);
    let size = setting(
        config,
        "fontSize",
        options.font_size.unwrap_or(16.0),
        4.0,
        128.0,
    );
    let group_pad =
        setting(config, "padding", 20.0, 0.0, 256.0).min(if tight { 12.0 } else { f64::INFINITY });
    let small = js_round(size * 0.875);
    let pad = options.padding.unwrap_or(8.0);
    let half = icon_size / 2.0;

    let nodes = &model.nodes;
    let groups = &model.groups;
    let n = nodes.len();
    let g_count = groups.len();
    let node_refs: Vec<_> = nodes.iter().collect();
    let group_refs: Vec<_> = groups.iter().collect();
    let grid = place_on_grid(&node_refs, &group_refs, &model.edges, &model.layout_hints);

    // A service is its icon square with the title beneath; a junction takes the same room.
    let wrap = if tight {
        icon_size.max(64.0)
    } else {
        (icon_size * 2.25).max(96.0)
    };
    let mut labels: Vec<Label> = Vec::with_capacity(n);
    let mut box_w = vec![0.0_f64; n];
    let mut box_h = vec![0.0_f64; n];
    for (i, node) in nodes.iter().enumerate() {
        let title = if node.kind == NodeKind::Service {
            node.title.as_deref()
        } else {
            None
        };
        let label = layout_label(title, true, size, wrap, 0, None);
        set(
            &mut box_w,
            i,
            icon_size.max(2.0 * (label.width / 2.0).ceil()),
        );
        set(
            &mut box_h,
            i,
            icon_size
                + if label.height > 0.0 {
                    LABEL_GAP + label.height
                } else {
                    0.0
                },
        );
        labels.push(label);
    }

    let mut col_values: Vec<i64> = grid.col.clone();
    let mut row_values: Vec<i64> = grid.row.clone();
    for g in 0..g_count {
        if !at(&grid.group_empty, g) {
            continue;
        }
        col_values.push(at(&grid.group_col, g));
        row_values.push(at(&grid.group_row, g));
    }
    let col_of = rank(col_values);
    let row_of = rank(row_values);
    let c_count = col_of.len();
    let r_count = row_of.len();
    let mut col_w = vec![0.0_f64; c_count];
    let mut row_h = vec![0.0_f64; r_count];
    let gap = if tight {
        24.0_f64.max(2.0 * STUB)
    } else {
        40.0_f64.max(js_round(icon_size * 0.75))
    };
    let mut gap_x = vec![gap; c_count];
    let mut gap_y = vec![gap; r_count];
    let mut node_col = vec![0; n];
    let mut node_row = vec![0; n];
    for i in 0..n {
        let c = rank_of(&col_of, at(&grid.col, i)).unwrap_or(0);
        let r = rank_of(&row_of, at(&grid.row, i)).unwrap_or(0);
        set(&mut node_col, i, c);
        set(&mut node_row, i, r);
        raise(&mut col_w, c, at(&box_w, i));
        raise(&mut row_h, r, at(&box_h, i));
    }
    for g in 0..g_count {
        if !at(&grid.group_empty, g) {
            continue;
        }
        let c = rank_of(&col_of, at(&grid.group_col, g)).unwrap_or(0);
        let r = rank_of(&row_of, at(&grid.group_row, g)).unwrap_or(0);
        raise(&mut col_w, c, icon_size);
        raise(&mut row_h, r, half);
    }

    // An edge label needs the gap its edge crosses to be wide enough to hold it.
    let edges = &model.edges;
    let mut edge_labels: Vec<Label> = Vec::with_capacity(edges.len());
    let mut ends: Vec<(Option<usize>, Option<usize>)> = Vec::with_capacity(edges.len());
    for edge in edges {
        let a = model.position(&edge.lhs_id);
        let b = model.position(&edge.rhs_id);
        ends.push((a, b));
        let label = layout_label(edge.title.as_deref(), true, small, 160.0, 0, None);
        if let (Some(a), Some(b)) = (a, b)
            && label.width != 0.0
        {
            let (ca, cb) = (at(&node_col, a), at(&node_col, b));
            if ca == cb {
                raise(&mut col_w, ca, 2.0 * (label.width / 2.0).ceil() + 8.0);
                let (ra, rb) = (at(&node_row, a), at(&node_row, b));
                if ra != rb {
                    let r = ra.min(rb);
                    raise(&mut gap_y, r, label.height.ceil() + 24.0);
                }
            } else {
                let c = ca.min(cb);
                raise(&mut gap_x, c, label.width.ceil() + 28.0);
            }
        }
        edge_labels.push(label);
    }

    // Room around each group, kept on the rows and columns at its edges. A group that shares an
    // edge with a group inside it stands that much further out. Children come after their
    // parents, so walking backwards settles every group before the one that holds it.
    let mut group_labels: Vec<Label> = Vec::with_capacity(g_count);
    let mut c0 = vec![0; g_count];
    let mut c1 = vec![0; g_count];
    let mut r0 = vec![0; g_count];
    let mut r1 = vec![0; g_count];
    let mut ins_l = vec![0.0_f64; g_count];
    let mut ins_r = vec![0.0_f64; g_count];
    let mut ins_t = vec![0.0_f64; g_count];
    let mut ins_b = vec![0.0_f64; g_count];
    let mut head_w = vec![0.0_f64; g_count];
    let mut head_h = vec![0.0_f64; g_count];
    let mut col_l = vec![0.0_f64; c_count];
    let mut col_r = vec![0.0_f64; c_count];
    let mut row_t = vec![0.0_f64; r_count];
    let mut row_b = vec![0.0_f64; r_count];
    let badge = js_round(size * 1.25);
    for (g, group) in groups.iter().enumerate() {
        let (gc, gr) = (at(&grid.group_col, g), at(&grid.group_row, g));
        set(&mut c0, g, rank_of(&col_of, gc).unwrap_or(0));
        set(
            &mut c1,
            g,
            rank_of(&col_of, gc + at(&grid.group_cols, g) - 1).unwrap_or(0),
        );
        set(&mut r0, g, rank_of(&row_of, gr).unwrap_or(0));
        set(
            &mut r1,
            g,
            rank_of(&row_of, gr + at(&grid.group_rows, g) - 1).unwrap_or(0),
        );
        let room = if tight {
            80.0_f64.max(title_room(options) / 2.0)
        } else {
            320.0
        };
        let label = layout_label(group.title.as_deref(), true, size, room, 0, None);
        let icon = if group.icon.as_deref().is_some_and(|icon| !icon.is_empty()) {
            badge
        } else {
            0.0
        };
        let joint = if label.width > 0.0 && icon > 0.0 {
            6.0
        } else {
            0.0
        };
        set(&mut head_w, g, label.width + icon + joint);
        set(&mut head_h, g, label.height.max(icon));
        group_labels.push(label);
    }
    for g in (0..g_count).rev() {
        let left = group_pad + at(&ins_l, g);
        let mut right = group_pad + at(&ins_r, g);
        let top = if at(&head_h, g) > 0.0 {
            group_pad.max(at(&head_h, g) + 12.0)
        } else {
            group_pad
        } + at(&ins_t, g);
        let bottom = group_pad + at(&ins_b, g);
        let (gc0, gc1, gr0, gr1) = (at(&c0, g), at(&c1, g), at(&r0, g), at(&r1, g));
        if at(&head_w, g) > 0.0 {
            let mut inner = left + right;
            for c in gc0..=gc1 {
                inner += at(&col_w, c)
                    + if c < gc1 {
                        at(&col_r, c) + at(&gap_x, c) + at(&col_l, c + 1)
                    } else {
                        0.0
                    };
            }
            if at(&head_w, g) + 20.0 > inner {
                right += (at(&head_w, g) + 20.0 - inner).ceil();
            }
        }
        set(&mut ins_l, g, left);
        set(&mut ins_r, g, right);
        set(&mut ins_t, g, top);
        set(&mut ins_b, g, bottom);
        raise(&mut col_l, gc0, left);
        raise(&mut col_r, gc1, right);
        raise(&mut row_t, gr0, top);
        raise(&mut row_b, gr1, bottom);
        let Ok(p) = usize::try_from(at(&grid.group_parent, g)) else {
            continue;
        };
        if at(&c0, p) == gc0 {
            raise(&mut ins_l, p, left);
        }
        if at(&c1, p) == gc1 {
            raise(&mut ins_r, p, right);
        }
        if at(&r0, p) == gr0 {
            raise(&mut ins_t, p, top);
        }
        if at(&r1, p) == gr1 {
            raise(&mut ins_b, p, bottom);
        }
    }

    let mut xs = vec![0.0_f64; c_count];
    let mut ys = vec![0.0_f64; r_count];
    let mut inner_w = 0.0;
    for c in 0..c_count {
        let x = inner_w + at(&col_l, c);
        set(&mut xs, c, x);
        inner_w =
            x + at(&col_w, c) + at(&col_r, c) + if c + 1 < c_count { at(&gap_x, c) } else { 0.0 };
    }
    let mut inner_h = 0.0;
    for r in 0..r_count {
        let y = inner_h + at(&row_t, r);
        set(&mut ys, r, y);
        inner_h =
            y + at(&row_h, r) + at(&row_b, r) + if r + 1 < r_count { at(&gap_y, r) } else { 0.0 };
    }
    // What is drawn, which an edge that has to go around a node can push past the grid.
    let mut min_x: f64 = 0.0;
    let mut min_y: f64 = 0.0;
    let mut max_x: f64 = inner_w;
    let mut max_y: f64 = inner_h;

    let left = |g: usize| at(&xs, at(&c0, g)) - at(&ins_l, g);
    let right = |g: usize| at(&xs, at(&c1, g)) + at(&col_w, at(&c1, g)) + at(&ins_r, g);
    let top = |g: usize| at(&ys, at(&r0, g)) - at(&ins_t, g);
    let bottom = |g: usize| at(&ys, at(&r1, g)) + at(&row_h, at(&r1, g)) + at(&ins_b, g);
    let center_x = |i: usize| at(&xs, at(&node_col, i)) + at(&col_w, at(&node_col, i)) / 2.0;
    let row_y = |i: usize| at(&ys, at(&node_row, i));
    let is_junction = |i: usize| {
        nodes
            .get(i)
            .is_some_and(|node| node.kind == NodeKind::Junction)
    };

    let port = |i: usize, side: usize, via_group: bool| -> Port {
        let dx = at(&DX, side);
        let dy = at(&DY, side);
        let cx = center_x(i);
        let y = row_y(i);
        let mut p = Port {
            x: cx,
            y: y + half,
            dx,
            dy,
            x0: cx - at(&box_w, i) / 2.0,
            y0: y,
            x1: cx + at(&box_w, i) / 2.0,
            y1: y + at(&box_h, i),
            gap: 0.0,
            wire: false,
        };
        if is_junction(i) {
            p.x0 = cx;
            p.x1 = cx;
            p.y0 = p.y;
            p.y1 = p.y;
            p.gap = DOT;
        } else if dx != 0.0 {
            p.x = cx + dx * half;
        } else {
            let below = if labels.get(i).is_some_and(|label| label.height > 0.0) {
                2.0
            } else {
                0.0
            };
            p.y = if dy < 0.0 {
                y
            } else {
                y + at(&box_h, i) + below
            };
        }
        let g = if via_group {
            at(&grid.node_parent, i)
        } else {
            -1
        };
        if let Ok(g) = usize::try_from(g) {
            p.x0 = left(g);
            p.x1 = right(g);
            p.y0 = top(g);
            p.y1 = bottom(g);
            p.gap = 0.0;
            p.wire = is_junction(i);
            if dx == 0.0 {
                p.y = if dy < 0.0 { p.y0 } else { p.y1 };
            } else {
                p.x = if dx < 0.0 { p.x0 } else { p.x1 };
            }
        }
        p
    };

    let mut edges_out = String::new();
    let mut labels_out = String::new();
    // Vertical runs of the edges, to keep group titles out from under them.
    let mut runs: Vec<f64> = Vec::new();
    let trim_length = marker_trim("arrow_point");
    for (k, edge) in edges.iter().enumerate() {
        let Some((Some(a), Some(b))) = ends.get(k).copied() else {
            continue;
        };
        let mut s = port(a, side_index(edge.lhs_dir), edge.lhs_group);
        let mut e = port(b, side_index(edge.rhs_dir), edge.rhs_group);
        #[expect(clippy::float_cmp, reason = "Pele compares exactly")]
        if s.x == e.x && s.y == e.y && s.dx == e.dx {
            // An edge from a side back to the same side: part the two ends so it shows as a
            // loop.
            s.x -= 8.0 * s.dy.abs();
            s.y -= 8.0 * s.dx.abs();
            e.x += 8.0 * s.dy.abs();
            e.y += 8.0 * s.dx.abs();
        }
        let mut routed = route(&s, &e);
        if s.wire {
            routed.splice(0..0, [center_x(a), row_y(a) + half]);
        }
        if e.wire {
            routed.push(center_x(b));
            routed.push(row_y(b) + half);
        }
        let points = simplify(&routed);
        if points.len() < 4 {
            continue;
        }
        let mut id = edge.lhs_id.clone();
        id.push('-');
        id.push_str(&edge.rhs_id);
        edges_out.push_str("<g class=\"ondine-edge\" data-id=\"");
        edges_out.push_str(&esc_text(&id));
        edges_out.push_str("\"><path d=\"");
        let start_trim = if edge.lhs_into && !s.wire {
            trim_length + s.gap
        } else {
            0.0
        };
        let end_trim = if edge.rhs_into && !e.wire {
            trim_length + e.gap
        } else {
            0.0
        };
        path_data(&mut edges_out, &points, start_trim, end_trim);
        edges_out.push_str("\"/>");
        for (into, p) in [(edge.lhs_into, &s), (edge.rhs_into, &e)] {
            if into {
                edges_out.push_str(&marker(
                    "arrow_point",
                    p.x + p.dx * p.gap,
                    p.y + p.dy * p.gap,
                    -p.dx,
                    -p.dy,
                    "var(--_l)",
                ));
            }
        }
        edges_out.push_str("</g>");

        let mut best = None;
        let mut best_length = -1.0;
        let mut flat = None;
        let mut flat_length = -1.0;
        let label = edge_labels.get(k).cloned().unwrap_or_default();
        for pair in points.chunks(2) {
            let (x, y) = (at(pair, 0), at(pair, 1));
            min_x = min_x.min(x - 4.0);
            max_x = max_x.max(x + 4.0);
            min_y = min_y.min(y - 4.0);
            max_y = max_y.max(y + 4.0);
        }
        let mut i = 0;
        while i + 3 < points.len() {
            let (x0, y0, x1, y1) = (
                at(&points, i),
                at(&points, i + 1),
                at(&points, i + 2),
                at(&points, i + 3),
            );
            let horizontal = (y0 - y1).abs() < 0.01;
            let length = (x0 - x1).abs() + (y0 - y1).abs();
            if !horizontal {
                runs.push(x0);
                runs.push(y0.min(y1));
                runs.push(y0.max(y1));
            }
            if length > best_length {
                best = Some(i);
                best_length = length;
            }
            if horizontal && length > flat_length {
                flat = Some(i);
                flat_length = length;
            }
            i += 2;
        }
        if label.width > 0.0 {
            let chosen = if flat_length >= label.width + 8.0 {
                flat
            } else {
                best
            };
            let Some(chosen) = chosen else {
                continue;
            };
            let (x0, y0, x1, y1) = (
                at(&points, chosen),
                at(&points, chosen + 1),
                at(&points, chosen + 2),
                at(&points, chosen + 3),
            );
            let mut x = f64::midpoint(x0, x1);
            let mut y = f64::midpoint(y0, y1);
            let w = label.width + 8.0;
            let upright = (x0 - x1).abs() < 0.01;
            // On a short run next to a node the label would land on the node, so it moves off
            // to the side.
            for p in [&s, &e] {
                if x + w / 2.0 <= p.x0
                    || x - w / 2.0 >= p.x1
                    || y + label.height / 2.0 <= p.y0
                    || y - label.height / 2.0 >= p.y1
                {
                    continue;
                }
                if upright {
                    x = if x >= f64::midpoint(p.x0, p.x1) {
                        p.x1 + w / 2.0 + 4.0
                    } else {
                        p.x0 - w / 2.0 - 4.0
                    };
                } else {
                    y = if y >= f64::midpoint(p.y0, p.y1) {
                        p.y1 + label.height / 2.0 + 4.0
                    } else {
                        p.y0 - label.height / 2.0 - 4.0
                    };
                }
            }
            min_x = min_x.min(x - w / 2.0);
            max_x = max_x.max(x + w / 2.0);
            min_y = min_y.min(y - label.height / 2.0);
            max_y = max_y.max(y + label.height / 2.0);
            labels_out.push_str(&edge_label_svg(&id, &[], &label, x, y, "", None));
        }
    }

    let mut clusters = String::new();
    for (g, group) in groups.iter().enumerate() {
        let x = left(g);
        let y = top(g);
        let w = right(g) - x;
        let mut head = String::new();
        let hw = at(&head_w, g);
        let hh = at(&head_h, g);
        if hw > 0.0 {
            // The title starts at the left, or after the edges that come down through that
            // corner.
            let band = y + hh + 12.0;
            let mut crossings: Vec<f64> = runs
                .chunks(3)
                .filter(|run| {
                    let (rx, ry0, ry1) = (at(run, 0), at(run, 1), at(run, 2));
                    ry0 < band && ry1 > y && rx > x && rx < x + w
                })
                .map(|run| at(run, 0))
                .collect();
            crossings.sort_by(f64::total_cmp);
            crossings.push(x + w - 4.0);
            let mut start = x + 10.0;
            for crossing in crossings {
                if crossing - 6.0 - start >= hw {
                    break;
                }
                start = start.max(crossing + 6.0);
            }
            if start + hw > x + w - 4.0 {
                start = x + 10.0;
            }
            let cy = y + 6.0 + hh / 2.0;
            let mut text_x = start;
            if let Some(icon) = group.icon.as_deref().filter(|icon| !icon.is_empty()) {
                icon_slot(&mut head, icon, start, cy - badge / 2.0, badge);
                text_x += badge + 6.0;
            }
            let label = group_labels.get(g).cloned().unwrap_or_default();
            head.push_str(&label_svg(
                &label,
                text_x + label.width / 2.0,
                cy,
                " class=\"ondine-cluster-label\" fill=\"var(--_m)\"",
                None,
            ));
        }
        clusters.push_str("<g class=\"ondine-cluster\" data-id=\"");
        clusters.push_str(&esc_text(&group.id));
        clusters.push_str("\"><rect x=\"");
        push_num(&mut clusters, x);
        clusters.push_str("\" y=\"");
        push_num(&mut clusters, y);
        clusters.push_str("\" width=\"");
        push_num(&mut clusters, w);
        clusters.push_str("\" height=\"");
        push_num(&mut clusters, bottom(g) - y);
        clusters.push_str("\" rx=\"");
        clusters.push_str(RADIUS);
        clusters.push_str("\" fill=\"var(--_a)\" fill-opacity=\"0.5\" stroke=\"var(--_b)\"/>");
        clusters.push_str(&head);
        clusters.push_str("</g>");
    }

    let mut nodes_out = String::new();
    let glyph = icon_size * 0.5;
    let text_size = 6.0_f64.max(js_round(size * 0.75));
    for (i, node) in nodes.iter().enumerate() {
        let mut at_attrs = String::from("data-id=\"");
        at_attrs.push_str(&esc_text(&node.id));
        at_attrs.push_str("\" transform=\"translate(");
        push_point(&mut at_attrs, center_x(i), row_y(i));
        at_attrs.push_str(")\"");
        if node.kind == NodeKind::Junction {
            nodes_out.push_str("<g class=\"ondine-node ondine-junction\" ");
            nodes_out.push_str(&at_attrs);
            nodes_out.push_str("><circle cy=\"");
            push_num(&mut nodes_out, half);
            nodes_out.push_str("\" r=\"");
            push_number(&mut nodes_out, DOT);
            nodes_out.push_str("\" fill=\"var(--_l)\"/></g>");
            continue;
        }
        let mut inner = String::new();
        if let Some(icon) = node.icon.as_deref().filter(|icon| !icon.is_empty()) {
            icon_slot(
                &mut inner,
                icon,
                -glyph / 2.0,
                (icon_size - glyph) / 2.0,
                glyph,
            );
        } else if let Some(text) = node.icon_text.as_deref().filter(|text| !text.is_empty()) {
            let text = layout_label(Some(text), false, text_size, icon_size - 8.0, 0, None);
            let line_height = if text.line_height == 0.0 {
                1.0
            } else {
                text.line_height
            };
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "a small positive line count"
            )]
            let lines = 1.max(((icon_size - 4.0) / line_height).floor() as usize);
            let mut attrs = String::from(" class=\"ondine-icon-text\" font-size=\"");
            push_number(&mut attrs, text_size);
            attrs.push('"');
            inner = label_svg(&fit(text, icon_size - 8.0, lines), 0.0, half, &attrs, None);
        }
        let label = labels.get(i).cloned().unwrap_or_default();
        nodes_out.push_str("<g class=\"ondine-node ondine-service\" ");
        nodes_out.push_str(&at_attrs);
        nodes_out.push_str("><rect x=\"");
        push_num(&mut nodes_out, -half);
        nodes_out.push_str("\" width=\"");
        push_num(&mut nodes_out, icon_size);
        nodes_out.push_str("\" height=\"");
        push_num(&mut nodes_out, icon_size);
        nodes_out.push_str("\" rx=\"");
        nodes_out.push_str(RADIUS);
        nodes_out.push_str("\" fill=\"var(--_s)\" stroke=\"var(--_b)\"/>");
        nodes_out.push_str(&inner);
        nodes_out.push_str(&label_svg(
            &label,
            0.0,
            icon_size + LABEL_GAP + label.height / 2.0,
            " class=\"ondine-label\"",
            None,
        ));
        nodes_out.push_str("</g>");
    }

    let title_width = if tight { title_room(options) } else { 4000.0 };
    let title = layout_label(model.title.as_deref(), false, size, title_width, BOLD, None);
    let title_height = if title.height > 0.0 {
        title.height + 12.0
    } else {
        0.0
    };
    let width = ((max_x - min_x).max(title.width) + 2.0 * pad).ceil();
    let height = (max_y - min_y + title_height + 2.0 * pad).ceil();
    let ox = js_round((width - (max_x - min_x)) / 2.0 - min_x);
    let oy = js_round(pad + title_height - min_y);

    let mut body = String::new();
    if !clusters.is_empty() {
        body.push_str("<g class=\"ondine-clusters\">");
        body.push_str(&clusters);
        body.push_str("</g>");
    }
    if !edges_out.is_empty() {
        body.push_str("<g class=\"ondine-edges\" fill=\"none\" stroke=\"var(--_l)\" stroke-linecap=\"round\" stroke-linejoin=\"round\">");
        body.push_str(&edges_out);
        body.push_str("</g>");
    }
    if !labels_out.is_empty() {
        body.push_str("<g class=\"ondine-edge-labels\" font-size=\"");
        push_number(&mut body, small);
        body.push_str("\">");
        body.push_str(&labels_out);
        body.push_str("</g>");
    }
    if !nodes_out.is_empty() {
        body.push_str("<g class=\"ondine-nodes\">");
        body.push_str(&nodes_out);
        body.push_str("</g>");
    }

    let mut content = label_svg(
        &title,
        width / 2.0,
        pad + title.height / 2.0,
        " class=\"ondine-title\" font-weight=\"var(--_tw)\"",
        None,
    );
    if !body.is_empty() {
        content.push_str("<g transform=\"translate(");
        push_number(&mut content, ox);
        content.push(',');
        push_number(&mut content, oy);
        content.push_str(")\">");
        content.push_str(&body);
        content.push_str("</g>");
    }
    let accessible = Accessible {
        acc_descr: model.acc_descr.as_deref(),
        acc_title: model.acc_title.as_deref(),
    };
    let svg = svg_document(
        "architecture",
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
