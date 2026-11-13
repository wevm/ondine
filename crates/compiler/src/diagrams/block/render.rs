//! Lays out and draws a block diagram. Port of Pele's `src/diagrams/block/render.ts`.

use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, FRAC_PI_4, PI};

use super::arrows::{OVERHANG, arrow_axis, arrow_mask, arrow_size, draw_arrow};
use super::db::{BlockDb, ROOT};
use super::layout::{Cell, check_columns, layout};
use super::types::Block;
use crate::diagrams::Rendered;
use crate::diagrams::common::fit_width::{tighten, title_room};
use crate::error::Error;
use crate::svg::builder::{esc_text, js_round, label_svg, push_num};
use crate::svg::edges::{EdgePath, edge_label_svg, marker, marker_trim};
use crate::svg::root::{Accessible, RenderOptions, svg_document};
use crate::svg::shapes::{draw_shape, shape_inset, shape_size};
use crate::svg::theme::{RADIUS, ResolvedStyle, class_names, resolve_style};
use crate::text::label::{Label, layout_label, trim};
use crate::text::measure::{BOLD, ITALIC};
use crate::yaml::{Value, push_number};

const GAP: f64 = 12.0;
const WRAP: f64 = 200.0;
const CORNER: f64 = 8.0;
const SHAPE_ATTRS: &str = " fill=\"var(--_s)\" stroke=\"var(--_b)\"";
const LINE_ATTRS: &str = " stroke=\"var(--_b)\"";
/// Checking every edge against every block is quadratic, so large diagrams route without it.
const OBSTACLE_BUDGET: usize = 2_000_000;

/// The drawn shape of a block type.
fn shape_of(r#type: &str) -> &'static str {
    match r#type {
        "round" => "rounded",
        "circle" => "circle",
        "doublecircle" => "dbl-circ",
        "diamond" => "diam",
        "hexagon" => "hex",
        "stadium" => "stadium",
        "subroutine" => "fr-rect",
        "cylinder" => "cyl",
        "rect_left_inv_arrow" => "odd",
        "lean_right" => "lean-r",
        "lean_left" => "lean-l",
        "trapezoid" => "trap-b",
        "inv_trapezoid" => "trap-t",
        _ => "rect",
    }
}

/// Shapes that keep their own size in a larger cell, where the others stretch to fill it.
fn fixed(shape: &str) -> bool {
    matches!(shape, "circle" | "dbl-circ" | "diam")
}

/// Shapes that only widen, because their slants and curves are set by their height.
fn wide(shape: &str) -> bool {
    matches!(
        shape,
        "stadium" | "hex" | "odd" | "lean-r" | "lean-l" | "trap-b" | "trap-t"
    )
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Kind {
    Arrow,
    Composite,
    #[default]
    Leaf,
    Space,
}

/// What a block is drawn as, beside its grid cell.
struct View {
    block: usize,
    /// Drawn height, which for a fixed shape is smaller than its cell.
    bh: f64,
    /// Drawn width, which for a fixed shape is smaller than its cell.
    bw: f64,
    dx: f64,
    dy: f64,
    kind: Kind,
    label: Label,
    mask: u32,
    shape: &'static str,
    style: ResolvedStyle,
}

/// An edge's route: two points for a straight line, three for a bend, none for a self-loop.
struct Route {
    edge: usize,
    label: Label,
    loop_: f64,
    lx: f64,
    ly: f64,
    points: Vec<f64>,
}

/// What routing reads of a placed view.
#[derive(Clone, Copy, Default)]
struct Shape {
    bh: f64,
    bw: f64,
    kind: Kind,
    mask: u32,
    name: &'static str,
    x: f64,
    y: f64,
}

impl Shape {
    fn is_round(self) -> bool {
        self.kind == Kind::Leaf && matches!(self.name, "circle" | "dbl-circ")
    }

    fn is_diamond(self) -> bool {
        self.kind == Kind::Leaf && self.name == "diam"
    }
}

/// The shape of view `at` as placed.
fn shape(cells: &[Cell], views: &[View], at: usize) -> Shape {
    let (Some(c), Some(v)) = (cells.get(at), views.get(at)) else {
        return Shape::default();
    };
    Shape {
        bh: v.bh,
        bw: v.bw,
        kind: v.kind,
        mask: v.mask,
        name: v.shape,
        x: c.x,
        y: c.y,
    }
}

/// V8's `Math.hypot` for two values, copied from `crate::layout::layered`: a compensated sum of
/// squares scaled by the larger value. It differs from `f64::hypot` in the last bit.
pub(crate) fn hypot(x: f64, y: f64) -> f64 {
    let x = x.abs();
    let y = y.abs();
    if x == f64::INFINITY || y == f64::INFINITY {
        return f64::INFINITY;
    }
    if x.is_nan() || y.is_nan() {
        return f64::NAN;
    }
    let max = if y > x { y } else { x };
    if max == 0.0 {
        return 0.0;
    }

    let mut sum = 0.0;
    let mut compensation = 0.0;
    for value in [x, y] {
        let n = value / max;
        let summand = n * n - compensation;
        let preliminary = sum + summand;
        compensation = (preliminary - sum) - summand;
        sum = preliminary;
    }
    sum.sqrt() * max
}

/// `value || 1`.
fn or_one(value: f64) -> f64 {
    if value == 0.0 || value.is_nan() {
        1.0
    } else {
        value
    }
}

/// JavaScript's `Math.sign`.
fn sign(x: f64) -> f64 {
    if x > 0.0 {
        1.0
    } else if x < 0.0 {
        -1.0
    } else {
        x
    }
}

/// Where a vertical line at `x` meets the outline, below the center (`sign` 1) or above it (-1).
fn edge_y(v: Shape, x: f64, sign: f64) -> f64 {
    let hw = v.bw / 2.0;
    let hh = v.bh / 2.0;
    let dx = (x - v.x).abs();
    let reach = if v.is_round() {
        (hw * hw - dx * dx).max(0.0).sqrt()
    } else if v.is_diamond() {
        hh * (1.0 - dx / hw).max(0.0)
    } else if v.kind == Kind::Leaf {
        hh - shape_inset(v.name, v.bw, v.bh, if sign > 0.0 { 2 } else { 0 }, 0.0)
    } else if v.kind == Kind::Arrow && arrow_axis(v.mask) == b'x' {
        hh - OVERHANG
    } else {
        hh
    };
    v.y + sign * reach
}

/// Where a horizontal line at `y` meets the outline, right of the center (`sign` 1) or left of it
/// (-1).
fn edge_x(v: Shape, y: f64, sign: f64) -> f64 {
    let hw = v.bw / 2.0;
    let hh = v.bh / 2.0;
    let dy = (y - v.y).abs();
    let reach = if v.is_round() {
        (hh * hh - dy * dy).max(0.0).sqrt()
    } else if v.is_diamond() {
        hw * (1.0 - dy / hh).max(0.0)
    } else if v.kind == Kind::Leaf {
        hw - shape_inset(v.name, v.bw, v.bh, if sign > 0.0 { 1 } else { 3 }, 0.0)
    } else if v.kind == Kind::Arrow && arrow_axis(v.mask) == b'y' {
        hw - OVERHANG
    } else {
        hw
    };
    v.x + sign * reach
}

/// Where the line from the center toward (`tx`, `ty`) leaves the outline.
fn border(v: Shape, tx: f64, ty: f64, out: &mut Vec<f64>) {
    let dx = tx - v.x;
    let dy = ty - v.y;
    let hw = v.bw / 2.0;
    let hh = v.bh / 2.0;
    let mut k = if v.is_round() {
        hw / or_one(hypot(dx, dy))
    } else if v.is_diamond() {
        1.0 / or_one(dx.abs() / hw + dy.abs() / hh)
    } else {
        let kx = if dx == 0.0 {
            f64::INFINITY
        } else {
            hw / dx.abs()
        };
        let ky = if dy == 0.0 {
            f64::INFINITY
        } else {
            hh / dy.abs()
        };
        kx.min(ky)
    };
    if k == f64::INFINITY {
        k = 0.0;
    }
    out.push(v.x + dx * k);
    out.push(v.y + dy * k);
}

#[expect(
    clippy::float_cmp,
    clippy::many_single_char_names,
    reason = "mirrors Pele's crossesBox, which compares exactly"
)]
fn crosses_box(x1: f64, y1: f64, x2: f64, y2: f64, v: Shape) -> bool {
    let l = v.x - v.bw / 2.0 + 1.0;
    let r = v.x + v.bw / 2.0 - 1.0;
    let t = v.y - v.bh / 2.0 + 1.0;
    let b = v.y + v.bh / 2.0 - 1.0;
    if (x1 < l && x2 < l) || (x1 > r && x2 > r) || (y1 < t && y2 < t) || (y1 > b && y2 > b) {
        return false;
    }
    if x1 == x2 || y1 == y2 {
        return true;
    }
    // The segment is diagonal and its bounding box overlaps: the box is hit unless all four
    // corners are on one side.
    let dx = x2 - x1;
    let dy = y2 - y1;
    let side = |x: f64, y: f64| sign(dx * (y - y1) - dy * (x - x1));
    let first = side(l, t);
    first == 0.0 || side(r, t) != first || side(r, b) != first || side(l, b) != first
}

/// The views a route may not cross, by view index.
type Obstacles = [(usize, Shape)];

fn hits(points: &[f64], s: usize, t: usize, obstacles: &Obstacles) -> usize {
    let mut count = 0;
    for &(at, v) in obstacles {
        if at == s || at == t {
            continue;
        }
        for segment in points.windows(4).step_by(2) {
            if let [x1, y1, x2, y2] = *segment
                && crosses_box(x1, y1, x2, y2, v)
            {
                count += 1;
                break;
            }
        }
    }
    count
}

/// A straight line where the two blocks share a row or a column, otherwise one bend between
/// facing sides.
#[expect(
    clippy::float_cmp,
    reason = "mirrors Pele's routeEdge, which compares exactly"
)]
fn route_edge(
    (si, s): (usize, Shape),
    (ti, t): (usize, Shape),
    shift: f64,
    obstacles: Option<&Obstacles>,
) -> Vec<f64> {
    let (sx, sy, sw, sh) = (s.x, s.y, s.bw, s.bh);
    let (tx, ty, tw, th) = (t.x, t.y, t.bw, t.bh);
    let sl = sx - sw / 2.0;
    let sr = sx + sw / 2.0;
    let st = sy - sh / 2.0;
    let sb = sy + sh / 2.0;
    let tl = tx - tw / 2.0;
    let tr = tx + tw / 2.0;
    let tt = ty - th / 2.0;
    let tb = ty + th / 2.0;
    let x1 = sl.max(tl);
    let x2 = sr.min(tr);
    let y1 = st.max(tt);
    let y2 = sb.min(tb);
    let over_x = x2 - x1 > 6.0;
    let over_y = y2 - y1 > 6.0;

    if over_x && over_y {
        // One block is inside the other: join the inner one to the nearest side of the outer one.
        let inner_is_s = sw * sh <= tw * th;
        let (inner, outer) = if inner_is_s { (s, t) } else { (t, s) };
        let (ix, iy, iw, ih) = (inner.x, inner.y, inner.bw, inner.bh);
        let (ox, oy, ow, oh) = (outer.x, outer.y, outer.bw, outer.bh);
        let up = iy - ih / 2.0 - (oy - oh / 2.0);
        let down = oy + oh / 2.0 - (iy + ih / 2.0);
        let left = ix - iw / 2.0 - (ox - ow / 2.0);
        let right = ox + ow / 2.0 - (ix + iw / 2.0);
        let least = up.min(down).min(left).min(right);
        let points = if least == down {
            [ix, edge_y(inner, ix, 1.0), ix, oy + oh / 2.0]
        } else if least == up {
            [ix, edge_y(inner, ix, -1.0), ix, oy - oh / 2.0]
        } else if least == right {
            [edge_x(inner, iy, 1.0), iy, ox + ow / 2.0, iy]
        } else {
            [edge_x(inner, iy, -1.0), iy, ox - ow / 2.0, iy]
        };
        return if inner_is_s {
            points.to_vec()
        } else {
            vec![points[2], points[3], points[0], points[1]]
        };
    }
    if over_x {
        let margin = ((x2 - x1) / 2.0).min(6.0);
        let x = (f64::midpoint(x1, x2) + shift)
            .max(x1 + margin)
            .min(x2 - margin);
        let down = if ty > sy { 1.0 } else { -1.0 };
        return vec![x, edge_y(s, x, down), x, edge_y(t, x, -down)];
    }
    if over_y {
        let margin = ((y2 - y1) / 2.0).min(6.0);
        let y = (f64::midpoint(y1, y2) + shift)
            .max(y1 + margin)
            .min(y2 - margin);
        let right = if tx > sx { 1.0 } else { -1.0 };
        return vec![edge_x(s, y, right), y, edge_x(t, y, -right), y];
    }

    let right = if tx > sx { 1.0 } else { -1.0 };
    let down = if ty > sy { 1.0 } else { -1.0 };
    let across = vec![edge_x(s, sy, right), sy, tx, sy, tx, edge_y(t, tx, -down)];
    let along = vec![sx, edge_y(s, sx, down), sx, ty, edge_x(t, ty, -right), ty];
    let wide = (tx - sx).abs() >= (ty - sy).abs();
    let (mut best, other) = if wide {
        (across, along)
    } else {
        (along, across)
    };
    let Some(obstacles) = obstacles else {
        return best;
    };
    let mut least = hits(&best, si, ti, obstacles);
    if least == 0 {
        return best;
    }
    let count = hits(&other, si, ti, obstacles);
    if count < least {
        best = other;
        least = count;
    }
    if least == 0 {
        return best;
    }
    let mut direct = Vec::with_capacity(4);
    border(s, tx, ty, &mut direct);
    border(t, sx, sy, &mut direct);
    if hits(&direct, si, ti, obstacles) < least {
        direct
    } else {
        best
    }
}

/// Appends `x,y`.
fn push_point(out: &mut String, x: f64, y: f64) {
    push_num(out, x);
    out.push(',');
    push_num(out, y);
}

/// Builds the path through two or three points, rounding the bend and leaving room for markers.
fn trace(p: &[f64], mut start_trim: f64, mut end_trim: f64) -> EdgePath {
    let n = p.len();
    let at = |i: usize| p.get(i).copied().unwrap_or(0.0);
    let sx = at(0);
    let sy = at(1);
    let ex = at(n - 2);
    let ey = at(n - 1);
    let mut ax = at(2) - sx;
    let mut ay = at(3) - sy;
    let first = hypot(ax, ay);
    ax /= or_one(first);
    ay /= or_one(first);
    let mut bx = ex - at(n - 4);
    let mut by = ey - at(n - 3);
    let last = hypot(bx, by);
    bx /= or_one(last);
    by /= or_one(last);
    if n == 4 && start_trim + end_trim > first {
        let scale = first / (start_trim + end_trim);
        start_trim *= scale;
        end_trim *= scale;
    }
    start_trim = start_trim.min(first);
    end_trim = end_trim.min(last);
    let mut d = String::from("M");
    push_point(&mut d, sx + ax * start_trim, sy + ay * start_trim);
    if n == 6 {
        let r = CORNER.min(first - start_trim).min(last - end_trim).max(0.0);
        d.push('L');
        push_point(&mut d, at(2) - ax * r, at(3) - ay * r);
        d.push('Q');
        push_point(&mut d, at(2), at(3));
        d.push(' ');
        push_point(&mut d, at(2) + bx * r, at(3) + by * r);
    }
    d.push('L');
    push_point(&mut d, ex - bx * end_trim, ey - by * end_trim);
    EdgePath {
        d,
        edx: bx,
        edy: by,
        ex,
        ey,
        sdx: -ax,
        sdy: -ay,
        sx,
        sy,
    }
}

/// A self-loop is an arc around the top right corner of the block, or around the matching point
/// on a round or diamond outline. Gives the arc's center and radius and the angles where it leaves
/// and returns.
#[expect(clippy::many_single_char_names, reason = "mirrors Pele's loopArc")]
fn loop_arc(v: Shape, k: f64) -> [f64; 5] {
    let hw = v.bw / 2.0;
    let hh = v.bh / 2.0;
    let r = (10.0 + 5.0 * (k - 1.0)).min(hw).min(hh).max(1.0);
    let (x, y) = (v.x, v.y);
    if v.is_round() {
        let px = x + hw * FRAC_1_SQRT_2;
        let py = y - hw * FRAC_1_SQRT_2;
        let half = 2.0 * (r / (2.0 * hw)).min(1.0).asin();
        let angle = |at: f64| (y - hw * at.sin() - py).atan2(x + hw * at.cos() - px);
        return [px, py, r, angle(FRAC_PI_4 + half), angle(FRAC_PI_4 - half)];
    }
    if v.is_diamond() {
        let slope = hh.atan2(hw);
        return [x + hw / 2.0, y - hh / 2.0, r, slope + PI, slope];
    }
    [x + hw, y - hh, r, PI, FRAC_PI_2]
}

fn loop_path(v: Shape, k: f64, start_trim: f64, end_trim: f64) -> EdgePath {
    let [px, py, r, a1, a2] = loop_arc(v, k);
    let from = a1 + start_trim / r;
    let to = a2 - end_trim / r;
    let mut sweep = (to - from) % (2.0 * PI);
    if sweep <= 0.0 {
        sweep += 2.0 * PI;
    }
    let x1 = px + r * from.cos();
    let y1 = py + r * from.sin();
    let x2 = px + r * to.cos();
    let y2 = py + r * to.sin();
    let sx = px + r * a1.cos();
    let sy = py + r * a1.sin();
    let ex = px + r * a2.cos();
    let ey = py + r * a2.sin();
    // Each marker lies along the chord it covers, so it follows the curve instead of the tangent
    // at its tip.
    let sl = hypot(sx - x1, sy - y1);
    let el = hypot(ex - x2, ey - y2);
    let mut d = String::from("M");
    push_point(&mut d, x1, y1);
    d.push('A');
    push_point(&mut d, r, r);
    d.push_str(if sweep > PI { " 0 1 1 " } else { " 0 0 1 " });
    push_point(&mut d, x2, y2);
    EdgePath {
        d,
        edx: if el > 0.0 { (ex - x2) / el } else { -a2.sin() },
        edy: if el > 0.0 { (ey - y2) / el } else { a2.cos() },
        ex,
        ey,
        sdx: if sl > 0.0 { (sx - x1) / sl } else { a1.sin() },
        sdy: if sl > 0.0 { (sy - y1) / sl } else { -a1.cos() },
        sx,
        sy,
    }
}

const SPOTS: [f64; 5] = [0.5, 0.3, 0.7, 0.15, 0.85];

/// Picks the point on a leg where a label of the given size covers no block other than the two it
/// joins.
#[expect(clippy::many_single_char_names, reason = "mirrors Pele's labelSpot")]
fn label_spot(
    p: &[f64],
    at: usize,
    (w, h): (f64, f64),
    s: usize,
    t: usize,
    obstacles: Option<&Obstacles>,
) -> f64 {
    let Some(obstacles) = obstacles else {
        return 0.5;
    };
    let get = |i: usize| p.get(i).copied().unwrap_or(0.0);
    for spot in SPOTS {
        let x = get(at) + (get(at + 2) - get(at)) * spot;
        let y = get(at + 1) + (get(at + 3) - get(at + 1)) * spot;
        let clear = !obstacles.iter().any(|&(i, v)| {
            i != s
                && i != t
                && (x - v.x).abs() < f64::midpoint(w, v.bw)
                && (y - v.y).abs() < f64::midpoint(h, v.bh)
        });
        if clear {
            return spot;
        }
    }
    0.5
}

/// The value under `key` of a YAML mapping.
fn get<'v>(value: &'v Value, key: &str) -> Option<&'v Value> {
    match value {
        Value::Object(entries) => entries.iter().find(|(own, _)| own == key).map(|(_, v)| v),
        _ => None,
    }
}

/// Checks every container's grid width, as Pele's layout does when it places their children.
fn check(model: &BlockDb) -> Result<(), Error> {
    let mut queue = vec![ROOT];
    let mut at = 0;
    while let Some(&index) = queue.get(at) {
        at += 1;
        let Some(block) = model.arena.get(index) else {
            continue;
        };
        if block.r#type != "composite" {
            continue;
        }
        let children = model.children(index);
        if !children.is_empty() {
            check_columns(block.columns.unwrap_or(-1.0))?;
        }
        queue.extend_from_slice(children);
    }
    Ok(())
}

/// Draws a block diagram, tightening its spacing when it does not fit the host's width.
///
/// # Errors
///
/// Returns a semantic error for a container whose grid has 0 columns.
pub(crate) fn render(
    model: &BlockDb,
    config: &Value,
    options: &RenderOptions,
) -> Result<Rendered, Error> {
    check(model)?;
    Ok(tighten(options, |tight| {
        draw(model, config, options, tight)
    }))
}

#[expect(clippy::too_many_lines, reason = "mirrors Pele's draw")]
fn draw(model: &BlockDb, config: &Value, options: &RenderOptions, tight: bool) -> Rendered {
    let size = options.font_size.unwrap_or(16.0);
    let edge_size = js_round(size * 0.875);
    let pad = options.padding.unwrap_or(8.0);
    let configured = match get(config, "block").and_then(|block| get(block, "padding")) {
        Some(Value::Number(value)) if *value >= 0.0 => value.min(200.0),
        _ => GAP,
    };
    let gap = if tight {
        configured.min(6.0)
    } else {
        configured
    };
    let wrap = if tight { 80.0 } else { WRAP };

    let style_of = |block: &Block| {
        let mut declarations: Vec<String> = Vec::new();
        // A style list is split on commas only, so one entry can still hold several declarations.
        let mut add = |styles: &[String]| {
            for style in styles {
                declarations.extend(style.split(';').map(str::to_owned));
            }
        };
        match block
            .classes
            .as_deref()
            .filter(|classes| !classes.is_empty())
        {
            Some(classes) => {
                for name in classes {
                    if let Some(def) = model.class(name) {
                        add(&def.styles);
                    }
                }
            }
            None => {
                if let Some(def) = model.class("default") {
                    add(&def.styles);
                }
            }
        }
        if let Some(styles) = &block.styles {
            add(styles);
        }
        resolve_style(&declarations)
    };

    let view = |index: usize| -> (Cell, View) {
        let block = model.arena.get(index).cloned().unwrap_or_default();
        let r#type = block.r#type;
        let style = if r#type == "space" {
            ResolvedStyle::default()
        } else {
            style_of(&block)
        };
        let text_style =
            (if style.bold { BOLD } else { 0 }) | (if style.italic { ITALIC } else { 0 });
        let font_size = style.font_size.unwrap_or(size);
        let mut kind = Kind::Leaf;
        let mut shape = "";
        let mut nw = 0.0;
        let mut nh = 0.0;
        let mut head = 0.0;
        let mut mask = 0;
        let mut dx = 0.0;
        let mut dy = 0.0;
        let label = if r#type == "space" {
            kind = Kind::Space;
            Label::default()
        } else if r#type == "composite" {
            kind = Kind::Composite;
            let room = if tight { title_room(options) } else { 4000.0 };
            let label = layout_label(
                block.label.as_deref(),
                false,
                font_size,
                room,
                text_style,
                None,
            );
            head = if label.height > 0.0 {
                label.height + 4.0
            } else {
                0.0
            };
            nw = label.width + 2.0 * gap;
            nh = head + 2.0 * gap;
            label
        } else if r#type == "block_arrow" {
            kind = Kind::Arrow;
            let label = layout_label(
                block.label.as_deref(),
                false,
                font_size,
                wrap,
                text_style,
                None,
            );
            let blank = label
                .lines
                .iter()
                .flatten()
                .all(|part| part.icon.is_none() && trim(&part.text).is_empty());
            mask = arrow_mask(block.directions.as_deref());
            let s = if blank {
                arrow_size(mask, 0.0, 0.0)
            } else {
                arrow_size(mask, label.width, label.height)
            };
            nw = s.w;
            nh = s.h;
            dx = s.dx;
            dy = s.dy;
            label
        } else {
            shape = shape_of(r#type);
            let label = layout_label(
                block.label.as_deref(),
                false,
                font_size,
                wrap,
                text_style,
                None,
            );
            let s = shape_size(shape, label.width, label.height);
            nw = s.w;
            // A cylinder's caps deepen as it widens, so it is sized for the deepest ones.
            nh = if shape == "cyl" {
                s.h.max(label.height + 40.0)
            } else {
                s.h
            };
            dy = s.dy;
            label
        };
        let cell = Cell {
            columns: if kind == Kind::Composite {
                block.columns.unwrap_or(-1.0)
            } else {
                -1.0
            },
            head,
            nh,
            nw,
            space: kind == Kind::Space,
            span: if kind == Kind::Space {
                1.0
            } else {
                block.width_in_columns.unwrap_or(1.0)
            },
            ..Cell::default()
        };
        let view = View {
            bh: 0.0,
            block: index,
            bw: 0.0,
            dx,
            dy,
            kind,
            label,
            mask,
            shape,
            style,
        };
        (cell, view)
    };

    let (mut root, root_view) = view(ROOT);
    root.nw = 0.0;
    root.nh = 0.0;
    let mut cells = vec![root];
    let mut views = vec![root_view];
    let mut by_id: BTreeMap<&str, usize> = BTreeMap::new();
    let mut i = 0;
    while let Some(parent) = views.get(i) {
        if parent.kind == Kind::Composite {
            let block = parent.block;
            for &child in model.children(block) {
                let (cell, v) = view(child);
                let at = cells.len();
                if let Some(parent) = cells.get_mut(i) {
                    parent.children.push(at);
                }
                if v.kind != Kind::Space
                    && let Some(id) = model.arena.get(child).and_then(|b| b.id.as_deref())
                {
                    by_id.insert(id, at);
                }
                cells.push(cell);
                views.push(v);
            }
        }
        i += 1;
    }

    let (content_width, content_height) = layout(&mut cells, gap);

    let mut obstacle_list: Vec<usize> = Vec::new();
    for (at, (v, c)) in views.iter_mut().zip(&cells).enumerate() {
        if v.kind == Kind::Leaf && fixed(v.shape) {
            v.bw = c.nw;
            v.bh = c.nh;
        } else if v.kind == Kind::Arrow {
            v.bw = if c.span > 1.0 { c.w } else { c.nw };
            v.bh = c.nh;
        } else {
            v.bw = c.w;
            v.bh = if v.kind == Kind::Leaf && wide(v.shape) {
                c.nh
            } else {
                c.h
            };
        }
        if v.kind == Kind::Leaf && v.shape == "cyl" {
            v.dy = (v.bw / 14.0).clamp(6.0, 12.0) / 2.0;
        }
        if v.kind == Kind::Leaf || v.kind == Kind::Arrow {
            obstacle_list.push(at);
        }
    }

    // Edges between the same two blocks are spread apart instead of drawn on top of each other.
    let count = cells.len();
    let mut pairs: BTreeMap<usize, f64> = BTreeMap::new();
    let mut labelled: BTreeSet<usize> = BTreeSet::new();
    let mut ends: Vec<(usize, usize)> = Vec::new();
    let mut drawn: Vec<usize> = Vec::new();
    for &edge in &model.edges {
        let Some(block) = model.arena.get(edge) else {
            continue;
        };
        let (Some(&s), Some(&t)) = (
            by_id.get(block.start.as_deref().unwrap_or("")),
            by_id.get(block.end.as_deref().unwrap_or("")),
        ) else {
            continue;
        };
        let key = s.min(t) * count + s.max(t);
        *pairs.entry(key).or_insert(0.0) += 1.0;
        if block
            .label
            .as_deref()
            .is_some_and(|label| !label.is_empty())
        {
            labelled.insert(key);
        }
        ends.push((s, t));
        drawn.push(edge);
    }

    let obstacles: Vec<(usize, Shape)> = obstacle_list
        .iter()
        .map(|&at| (at, shape(&cells, &views, at)))
        .collect();
    let check = (drawn.len() * obstacles.len() <= OBSTACLE_BUDGET).then_some(obstacles.as_slice());
    let mut seen: BTreeMap<usize, f64> = BTreeMap::new();
    let mut routes: Vec<Route> = Vec::new();
    let mut min_x: f64 = 0.0;
    let mut min_y: f64 = 0.0;
    let mut max_x = content_width;
    let mut max_y = content_height;
    let mut grow = |x: f64, y: f64, w: f64, h: f64| {
        if x - w / 2.0 < min_x {
            min_x = x - w / 2.0;
        }
        if x + w / 2.0 > max_x {
            max_x = x + w / 2.0;
        }
        if y - h / 2.0 < min_y {
            min_y = y - h / 2.0;
        }
        if y + h / 2.0 > max_y {
            max_y = y + h / 2.0;
        }
    };
    for (&edge, &(s, t)) in drawn.iter().zip(&ends) {
        let key = s.min(t) * count + s.max(t);
        let total = pairs.get(&key).copied().unwrap_or(1.0);
        let k = seen.get(&key).copied().unwrap_or(0.0);
        seen.insert(key, k + 1.0);
        let block = model.arena.get(edge);
        let label = layout_label(
            block.and_then(|b| b.label.as_deref()),
            false,
            edge_size,
            WRAP,
            0,
            None,
        );
        let (points, lx, ly) = if s == t {
            let [px, py, r, _, _] = loop_arc(shape(&cells, &views, s), k + 1.0);
            grow(px, py, 2.0 * r + 2.0, 2.0 * r + 2.0);
            (Vec::new(), px + r + 6.0 + label.width / 2.0, py - r / 2.0)
        } else {
            let step = if labelled.contains(&key) {
                js_round(edge_size * 1.5) + 2.0
            } else {
                14.0
            };
            let shift = (k - (total - 1.0) / 2.0) * step;
            let points = route_edge(
                (s, shape(&cells, &views, s)),
                (t, shape(&cells, &views, t)),
                shift,
                check,
            );
            let n = points.len();
            let p = |i: usize| points.get(i).copied().unwrap_or(0.0);
            // The label goes on the longer leg.
            let first_leg = (p(2) - p(0)).abs() + (p(3) - p(1)).abs();
            let last_leg = (p(n - 2) - p(n - 4)).abs() + (p(n - 1) - p(n - 3)).abs();
            let at = if n == 6 && last_leg > first_leg { 2 } else { 0 };
            let spot = if label.width > 0.0 {
                label_spot(&points, at, (label.width + 8.0, label.height), s, t, check)
            } else {
                0.5
            };
            let lx = p(at) + (p(at + 2) - p(at)) * spot;
            let ly = p(at + 1) + (p(at + 3) - p(at + 1)) * spot;
            (points, lx, ly)
        };
        if label.width > 0.0 {
            grow(lx, ly, label.width + 8.0, label.height);
        }
        routes.push(Route {
            edge,
            label,
            loop_: if s == t { k + 1.0 } else { 0.0 },
            lx,
            ly,
            points,
        });
    }

    let title_width = if tight { title_room(options) } else { 4000.0 };
    let title = layout_label(model.title.as_deref(), false, size, title_width, BOLD, None);
    let title_height = if title.height > 0.0 {
        title.height + 12.0
    } else {
        0.0
    };
    let inner = (max_x - min_x).max(title.width);
    let ox = pad - min_x + (inner - (max_x - min_x)) / 2.0;
    let oy = pad + title_height - min_y;
    for cell in &mut cells {
        cell.x += ox;
        cell.y += oy;
    }
    let width = (inner + 2.0 * pad).ceil();
    let height = (max_y - min_y + title_height + 2.0 * pad).ceil();

    let mut clusters = String::new();
    let mut nodes = String::new();
    for (at, v) in views.iter().enumerate().skip(1) {
        if v.kind == Kind::Space {
            continue;
        }
        let Some(c) = cells.get(at) else {
            continue;
        };
        let block = model.arena.get(v.block).cloned().unwrap_or_default();
        let classes = class_names(&block.classes.as_deref().unwrap_or(&[]).join(" "));
        let id = esc_text(block.id.as_deref().unwrap_or(""));
        if v.kind == Kind::Composite {
            let top = c.y - c.h / 2.0;
            clusters.push_str("<g class=\"ondine-cluster");
            clusters.push_str(&classes);
            clusters.push_str("\" data-id=\"");
            clusters.push_str(&id);
            clusters.push_str("\"><rect x=\"");
            push_num(&mut clusters, c.x - c.w / 2.0);
            clusters.push_str("\" y=\"");
            push_num(&mut clusters, top);
            clusters.push_str("\" width=\"");
            push_num(&mut clusters, c.w);
            clusters.push_str("\" height=\"");
            push_num(&mut clusters, c.h);
            clusters.push_str("\" rx=\"");
            clusters.push_str(RADIUS);
            clusters.push_str("\" fill=\"var(--_a)\" fill-opacity=\"0.5\" stroke=\"var(--_b)\"");
            clusters.push_str(&v.style.shape);
            clusters.push_str("/>");
            let attrs = [
                " class=\"ondine-cluster-label\" fill=\"var(--_m)\"",
                &v.style.text,
            ]
            .concat();
            clusters.push_str(&label_svg(
                &v.label,
                c.x,
                top + f64::midpoint(c.head, gap) + 2.0,
                &attrs,
                None,
            ));
            clusters.push_str("</g>");
            continue;
        }
        let arrow = v.kind == Kind::Arrow;
        nodes.push_str("<g class=\"ondine-node ondine-shape-");
        nodes.push_str(if arrow { "block-arrow" } else { v.shape });
        nodes.push_str(&classes);
        nodes.push_str("\" data-id=\"");
        nodes.push_str(&id);
        nodes.push_str("\" transform=\"translate(");
        push_point(&mut nodes, c.x, c.y);
        nodes.push_str(")\">");
        let shape_attrs = [SHAPE_ATTRS, &v.style.shape].concat();
        if arrow {
            nodes.push_str(&draw_arrow(v.mask, v.bw, v.bh, &shape_attrs));
        } else {
            let line_attrs = [LINE_ATTRS, &v.style.line].concat();
            nodes.push_str(&draw_shape(v.shape, v.bw, v.bh, &shape_attrs, &line_attrs));
        }
        let attrs = [" class=\"ondine-label\"", &v.style.text].concat();
        nodes.push_str(&label_svg(&v.label, v.dx, v.dy, &attrs, None));
        nodes.push_str("</g>");
    }

    let mut edges = String::new();
    let mut labels = String::new();
    for (route, &(s, _)) in routes.iter_mut().zip(&ends) {
        let block = model.arena.get(route.edge).cloned().unwrap_or_default();
        let start_type = block.arrow_type_start.unwrap_or("");
        let end_type = block.arrow_type_end.unwrap_or("");
        let path = if route.loop_ > 0.0 {
            loop_path(
                shape(&cells, &views, s),
                route.loop_,
                marker_trim(start_type),
                marker_trim(end_type),
            )
        } else {
            for pair in route.points.chunks_exact_mut(2) {
                if let [x, y] = pair {
                    *x += ox;
                    *y += oy;
                }
            }
            trace(
                &route.points,
                marker_trim(start_type),
                marker_trim(end_type),
            )
        };
        let id = block.id.as_deref().unwrap_or("");
        edges.push_str("<g class=\"ondine-edge\" data-id=\"");
        edges.push_str(&esc_text(id));
        edges.push_str("\"><path d=\"");
        edges.push_str(&path.d);
        edges.push('"');
        if block.thickness == Some("thick") {
            edges.push_str(" stroke-width=\"2.5\"");
        }
        if block.pattern == Some("dotted") {
            edges.push_str(" stroke-dasharray=\"3 4\"");
        }
        edges.push_str("/>");
        edges.push_str(&marker(
            start_type,
            path.sx,
            path.sy,
            path.sdx,
            path.sdy,
            "var(--_l)",
        ));
        edges.push_str(&marker(
            end_type,
            path.ex,
            path.ey,
            path.edx,
            path.edy,
            "var(--_l)",
        ));
        edges.push_str("</g>");
        if route.label.width > 0.0 {
            labels.push_str(&edge_label_svg(
                id,
                &[],
                &route.label,
                route.lx + ox,
                route.ly + oy,
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
    if !edges.is_empty() {
        content.push_str(
            "<g class=\"ondine-edges\" fill=\"none\" stroke=\"var(--_l)\" stroke-linecap=\"round\">",
        );
        content.push_str(&edges);
        content.push_str("</g>");
    }
    content.push_str("<g class=\"ondine-nodes\">");
    content.push_str(&nodes);
    content.push_str("</g>");
    if !labels.is_empty() {
        content.push_str("<g class=\"ondine-edge-labels\" font-size=\"");
        push_number(&mut content, edge_size);
        content.push_str("\">");
        content.push_str(&labels);
        content.push_str("</g>");
    }

    let svg = svg_document(
        "block",
        width,
        height,
        size,
        options,
        Accessible::default(),
        &content,
    );
    Rendered {
        height,
        links: Vec::new(),
        svg,
        width,
    }
}
