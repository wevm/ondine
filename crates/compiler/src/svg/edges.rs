//! Edge paths, end markers, edge labels, and self-loops. Port of Pele's `src/svg/edges.ts`.
//!
//! Marker types are Mermaid's: `arrow_point`, `arrow_circle`, and `arrow_cross`. Any other type
//! draws nothing.

use std::collections::BTreeMap;

use crate::layout::compound::{CEdge, CNode};
use crate::layout::layered::ease;
use crate::svg::builder::{esc_text, label_svg, push_num};
use crate::text::icons::IconResolver;
use crate::text::label::Label;

/// The length of an arrowhead.
pub(crate) const ARROW: f64 = 8.0;

/// A drawn edge: its path, and where and which way it leaves and arrives, for its markers.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct EdgePath {
    /// The path data.
    pub(crate) d: String,
    /// The x of the way the edge arrives.
    pub(crate) edx: f64,
    /// The y of the way the edge arrives.
    pub(crate) edy: f64,
    /// The x of the end, before trimming.
    pub(crate) ex: f64,
    /// The y of the end, before trimming.
    pub(crate) ey: f64,
    /// The x of the way back out of the start, for a marker there.
    pub(crate) sdx: f64,
    /// The y of the way back out of the start, for a marker there.
    pub(crate) sdy: f64,
    /// The x of the start, before trimming.
    pub(crate) sx: f64,
    /// The y of the start, before trimming.
    pub(crate) sy: f64,
}

/// The most a corner is rounded by in a stepped route.
const CORNER: f64 = 12.0;
/// How far the handles of a wide turn reach along its two legs. A little more than a circle's,
/// which makes the turn fuller.
const HANDLE: f64 = 0.62;
/// How far an edge runs straight before a marker at its end, where it has the room.
pub(crate) const LEAD: f64 = 6.0;

/// JavaScript's `Math.sign`: zero for zero and `NaN` for `NaN`.
fn js_sign(value: f64) -> f64 {
    if value == 0.0 || value.is_nan() {
        value
    } else {
        value.signum()
    }
}

/// `Math.sign(value) || 1`: the sign, or 1 where there is none.
fn sign_or_one(value: f64) -> f64 {
    if value == 0.0 || value.is_nan() {
        1.0
    } else {
        value.signum()
    }
}

/// How far from each side of each node the edges that meet it start to turn: as far as the
/// largest marker there needs, for every edge on the side, so none turns sooner than its
/// neighbor and crosses it.
#[derive(Clone, Debug, Default)]
pub(crate) struct SideReach {
    /// The reach by `node * 4 + side`.
    reach: BTreeMap<usize, f64>,
}

impl SideReach {
    /// Records an end with a marker of `type` on `side` of `node`. A side below 0 is none.
    pub(crate) fn add(&mut self, node: usize, side: i32, r#type: &str) {
        let trim = marker_trim(r#type);
        let Ok(side) = usize::try_from(side) else {
            return;
        };
        let key = node * 4 + side;
        if trim > 0.0 && trim + LEAD > self.reach.get(&key).copied().unwrap_or(0.0) {
            self.reach.insert(key, trim + LEAD);
        }
    }

    /// How far from `side` of `node` its edges start to turn.
    pub(crate) fn of(&self, node: usize, side: i32) -> f64 {
        usize::try_from(side).map_or(0.0, |side| {
            self.reach.get(&(node * 4 + side)).copied().unwrap_or(0.0)
        })
    }
}

/// A route's points as [`route_path`] reshapes them.
struct Points {
    /// The axis each point is entered along: 0 vertically, 1 horizontally.
    axes: Vec<f64>,
    /// Whether the route is drawn as straight lines.
    linear: bool,
    /// The x of each point.
    xs: Vec<f64>,
    /// The y of each point.
    ys: Vec<f64>,
}

impl Points {
    /// The x of point `i`, or `NaN` past the ends, as JavaScript reads an absent element.
    fn x(&self, i: usize) -> f64 {
        self.xs.get(i).copied().unwrap_or(f64::NAN)
    }

    /// The y of point `i`, or `NaN` past the ends.
    fn y(&self, i: usize) -> f64 {
        self.ys.get(i).copied().unwrap_or(f64::NAN)
    }

    /// The axis of point `i`, or `NaN` past the ends.
    fn axis(&self, i: usize) -> f64 {
        self.axes.get(i).copied().unwrap_or(f64::NAN)
    }

    /// Whether points `i - 1` and `i` are in line.
    fn aligned(&self, i: usize) -> bool {
        let j = i.wrapping_sub(1);
        (self.x(i) - self.x(j)).abs() < 0.01 || (self.y(i) - self.y(j)).abs() < 0.01
    }

    /// Whether the route runs straight into point `i`.
    fn straight(&self, i: usize) -> bool {
        self.linear || self.aligned(i)
    }

    /// The way the route heads from point `i` to point `j`: along the flow where an S joins them.
    fn unit(&self, i: usize, j: usize, axis: f64) -> (f64, f64) {
        if !self.straight(i.max(j)) {
            return if axis == 0.0 {
                (0.0, sign_or_one(self.y(j) - self.y(i)))
            } else {
                (sign_or_one(self.x(j) - self.x(i)), 0.0)
            };
        }
        let dx = self.x(j) - self.x(i);
        let dy = self.y(j) - self.y(i);
        let len = dx.hypot(dy);
        let len = if len == 0.0 || len.is_nan() { 1.0 } else { len };
        (dx / len, dy / len)
    }

    /// The length of the leg into point `i`, along both axes.
    fn length(&self, i: usize) -> f64 {
        let j = i.wrapping_sub(1);
        (self.x(i) - self.x(j)).abs() + (self.y(i) - self.y(j)).abs()
    }

    /// Whether the route turns a corner at point `i`, from running one way to running the other.
    fn turns(&self, i: usize, last: usize) -> bool {
        let j = i.wrapping_sub(1);
        !self.linear
            && i > 0
            && i < last
            && self.aligned(i)
            && self.aligned(i + 1)
            && self.length(i) >= 0.01
            && self.length(i + 1) >= 0.01
            && ((self.x(i) - self.x(j)).abs() < 0.01) != ((self.x(i + 1) - self.x(i)).abs() < 0.01)
    }

    /// How far the leg into point `i` runs along (`ux`, `uy`).
    fn along(&self, i: usize, ux: f64, uy: f64) -> f64 {
        let j = i.wrapping_sub(1);
        (self.x(i) - self.x(j)).abs() * ux.abs() + (self.y(i) - self.y(j)).abs() * uy.abs()
    }
}

/// Whether two points are within a hundredth of each other on both axes.
fn near(ax: f64, ay: f64, bx: f64, by: f64) -> bool {
    (ax - bx).abs() < 0.01 && (ay - by).abs() < 0.01
}

/// Of the way from the end of a run to where it turns, the part that makes up for a larger marker
/// beside this one is kept where it can be, and the rest is at most a quarter of what is left, so
/// ends on one side of a node turn equally far from it.
fn lead(reach: f64, trim: f64, run: f64) -> f64 {
    let room = js_min(js_max(0.0, reach - LEAD - trim), run / 4.0);
    room + js_min(reach - trim - room, (run - room) / 4.0)
}

/// JavaScript's two-argument `Math.min`, which passes `NaN` on.
fn js_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// JavaScript's two-argument `Math.max`, which passes `NaN` on.
fn js_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// Appends `x,y`.
fn push_point(out: &mut String, x: f64, y: f64) {
    push_num(out, x);
    out.push(',');
    push_num(out, y);
}

/// Builds the path for a route of `x, y, axis` triples, trimming both ends to leave room for
/// markers.
///
/// Between two points that are not in line, a route is one S that leaves and arrives along the
/// flow, or a step in a stepped route. Where the route itself turns a corner, a stepped route
/// rounds it a little, and any other but a linear one turns as widely as it can: the turn takes
/// the whole of a leg that ends at a node and half of a leg it shares with the next turn.
/// `start_reach` and `end_reach` say how far from its nodes the edge starts to turn, where the
/// other edges on the same side of a node need it to. By default, far enough to clear its own
/// marker.
#[expect(
    clippy::many_single_char_names,
    reason = "mirrors Pele's geometry names"
)]
pub(crate) fn route_path(
    route: &[f64],
    curve: Option<&str>,
    start_trim: f64,
    end_trim: f64,
    start_reach: Option<f64>,
    end_reach: Option<f64>,
) -> EdgePath {
    let start_reach = start_reach.unwrap_or(if start_trim > 0.0 {
        start_trim + LEAD
    } else {
        0.0
    });
    let end_reach = end_reach.unwrap_or(if end_trim > 0.0 { end_trim + LEAD } else { 0.0 });
    let linear = curve == Some("linear");
    let stepped = matches!(curve, Some("step" | "stepBefore" | "stepAfter"));
    let mut p = Points {
        axes: Vec::new(),
        linear,
        xs: Vec::new(),
        ys: Vec::new(),
    };
    let at = |i: usize| route.get(i).copied().unwrap_or(f64::NAN);
    for i in (0..route.len()).step_by(3) {
        let x = at(i);
        let y = at(i + 1);
        let axis = at(i + 2);
        // A stepped route turns at right angles where a smooth one would curve.
        if stepped
            && let (Some(&px), Some(&py)) = (p.xs.last(), p.ys.last())
            && (x - px).abs() >= 0.01
            && (y - py).abs() >= 0.01
        {
            let (mx, my) = match curve {
                Some("stepBefore") => (px, py),
                Some("stepAfter") => (x, y),
                _ => (f64::midpoint(px, x), f64::midpoint(py, y)),
            };
            let turns = if axis == 0.0 {
                [px, my, x, my]
            } else {
                [mx, py, mx, y]
            };
            for [tx, ty] in [[turns[0], turns[1]], [turns[2], turns[3]]] {
                if near(tx, ty, px, py) || near(tx, ty, x, y) {
                    continue;
                }
                p.xs.push(tx);
                p.ys.push(ty);
                p.axes.push(axis);
            }
        }
        p.xs.push(x);
        p.ys.push(y);
        p.axes.push(axis);
    }

    // An empty route has no path. Pele would write `NaN` coordinates.
    let Some(mut last) = p.xs.len().checked_sub(1) else {
        return EdgePath::default();
    };
    let (sdx, sdy) = p.unit(0, 1, p.axis(1));
    let (edx, edy) = p.unit(last.wrapping_sub(1), last, p.axis(last));
    let sx = p.x(0);
    let sy = p.y(0);
    let ex = p.x(last);
    let ey = p.y(last);
    if let (Some(x), Some(y)) = (p.xs.first_mut(), p.ys.first_mut()) {
        *x += sdx * start_trim;
        *y += sdy * start_trim;
    }
    if let (Some(x), Some(y)) = (p.xs.get_mut(last), p.ys.get_mut(last)) {
        *x -= edx * end_trim;
        *y -= edy * end_trim;
    }

    // An edge that curves into a marker runs straight for a little way first, where it has the
    // room, so the marker does not sit on the bend.
    let arrives =
        if end_reach > end_trim && (!p.straight(last) || p.turns(last.wrapping_sub(1), last)) {
            lead(end_reach, end_trim, p.along(last, edx, edy))
        } else {
            0.0
        };
    let leaves = if start_reach > start_trim && (!p.straight(1) || p.turns(1, last)) {
        lead(start_reach, start_trim, p.along(1, sdx, sdy))
    } else {
        0.0
    };
    if arrives >= 1.0 {
        let (x, y, axis) = (
            p.x(last) - edx * arrives,
            p.y(last) - edy * arrives,
            p.axis(last),
        );
        p.xs.insert(last, x);
        p.ys.insert(last, y);
        p.axes.insert(last, axis);
        last += 1;
    }
    if leaves >= 1.0 {
        let (x, y, axis) = (p.x(0) + sdx * leaves, p.y(0) + sdy * leaves, p.axis(1));
        p.xs.insert(1, x);
        p.ys.insert(1, y);
        p.axes.insert(1, axis);
        last += 1;
    }

    let mut d = String::from("M");
    push_point(&mut d, p.x(0), p.y(0));
    let mut at_x = p.x(0);
    let mut at_y = p.y(0);
    let line_to = |d: &mut String, at_x: &mut f64, at_y: &mut f64, x: f64, y: f64| {
        if near(x, y, *at_x, *at_y) {
            return;
        }
        d.push('L');
        push_point(d, x, y);
        *at_x = x;
        *at_y = y;
    };
    for i in 1..=last {
        let x = p.x(i);
        let y = p.y(i);
        if !p.straight(i) {
            let down = p.axis(i) == 0.0;
            let along = if down { y - at_y } else { x - at_x };
            let k = along.abs() * ease(if down { x - at_x } else { y - at_y }, along);
            let ux = if down { 0.0 } else { js_sign(along) * k };
            let uy = if down { js_sign(along) * k } else { 0.0 };
            d.push('C');
            push_point(&mut d, at_x + ux, at_y + uy);
            d.push(' ');
            push_point(&mut d, x - ux, y - uy);
            d.push(' ');
            push_point(&mut d, x, y);
            at_x = x;
            at_y = y;
            continue;
        }
        if !p.turns(i, last) {
            // A point part of the way along one straight run adds nothing to the path.
            if i < last
                && p.aligned(i)
                && p.aligned(i + 1)
                && (x - p.x(i - 1)) * (p.x(i + 1) - x) + (y - p.y(i - 1)) * (p.y(i + 1) - y) > 0.0
            {
                continue;
            }
            line_to(&mut d, &mut at_x, &mut at_y, x, y);
            continue;
        }

        // How much of the leg before the turn and of the leg after it the turn takes: all of one
        // that ends at a node, short of the straight run into a marker, and half of one it shares.
        let mut before = if p.turns(i - 1, last) {
            p.length(i) / 2.0
        } else {
            p.length(i)
        };
        let mut after = if p.turns(i + 1, last) {
            p.length(i + 1) / 2.0
        } else {
            p.length(i + 1)
        };
        if stepped {
            before = CORNER.min(before).min(after);
            after = before;
        }
        let (ux, uy) = p.unit(i - 1, i, p.axis(i));
        let (vx, vy) = p.unit(i, i + 1, p.axis(i + 1));
        line_to(
            &mut d,
            &mut at_x,
            &mut at_y,
            x - ux * before,
            y - uy * before,
        );
        at_x = x + vx * after;
        at_y = y + vy * after;
        if stepped {
            d.push('Q');
            push_point(&mut d, x, y);
            d.push(' ');
            push_point(&mut d, at_x, at_y);
        } else {
            d.push('C');
            push_point(
                &mut d,
                x - ux * before * (1.0 - HANDLE),
                y - uy * before * (1.0 - HANDLE),
            );
            d.push(' ');
            push_point(
                &mut d,
                x + vx * after * (1.0 - HANDLE),
                y + vy * after * (1.0 - HANDLE),
            );
            d.push(' ');
            push_point(&mut d, at_x, at_y);
        }
    }
    EdgePath {
        d,
        edx,
        edy,
        ex,
        ey,
        sdx: -sdx,
        sdy: -sdy,
        sx,
        sy,
    }
}

/// How far a marker of `type` trims the end of its edge.
pub(crate) fn marker_trim(r#type: &str) -> f64 {
    match r#type {
        "arrow_point" => ARROW - 1.0,
        "arrow_circle" => ARROW,
        "arrow_cross" => 4.0,
        _ => 0.0,
    }
}

/// Draws a marker of `type` whose tip is at (`x`, `y`), pointing along (`dx`, `dy`), filled with
/// `color` as written.
pub(crate) fn marker(r#type: &str, x: f64, y: f64, dx: f64, dy: f64, color: &str) -> String {
    let nx = -dy;
    let ny = dx;
    let mut out = String::new();
    match r#type {
        "arrow_point" => {
            let bx = x - dx * ARROW;
            let by = y - dy * ARROW;
            let half = ARROW * 0.42;
            out.push_str("<path class=\"ondine-marker\" d=\"M");
            push_point(&mut out, x, y);
            out.push('L');
            push_point(&mut out, bx + nx * half, by + ny * half);
            out.push('L');
            push_point(&mut out, bx - nx * half, by - ny * half);
            out.push_str("Z\" fill=\"");
            out.push_str(color);
            out.push_str("\" stroke=\"none\"/>");
        }
        "arrow_circle" => {
            out.push_str("<circle class=\"ondine-marker\" cx=\"");
            push_num(&mut out, x - dx * 4.0);
            out.push_str("\" cy=\"");
            push_num(&mut out, y - dy * 4.0);
            out.push_str("\" r=\"3.5\" fill=\"");
            out.push_str(color);
            out.push_str("\" stroke=\"none\"/>");
        }
        "arrow_cross" => {
            let cx = x - dx * 4.0;
            let cy = y - dy * 4.0;
            let a = 3.5;
            let ux = (dx + nx) * a;
            let uy = (dy + ny) * a;
            let vx = (dx - nx) * a;
            let vy = (dy - ny) * a;
            out.push_str("<path class=\"ondine-marker\" d=\"M");
            push_point(&mut out, cx - ux, cy - uy);
            out.push('L');
            push_point(&mut out, cx + ux, cy + uy);
            out.push('M');
            push_point(&mut out, cx - vx, cy - vy);
            out.push('L');
            push_point(&mut out, cx + vx, cy + vy);
            out.push_str("\"/>");
        }
        _ => {}
    }
    out
}

/// An edge label centered on (`x`, `y`) on a patch of the background, so the line does not run
/// through the text. `lines` are the edge's authored source lines, if any.
pub(crate) fn edge_label_svg(
    id: &str,
    lines: &[u32],
    label: &Label,
    x: f64,
    y: f64,
    attrs: &str,
    icons: Option<IconResolver<'_>>,
) -> String {
    let w = label.width + 8.0;
    let h = label.height;
    let mut out = String::from("<g class=\"ondine-edge-label\" data-id=\"");
    out.push_str(&esc_text(id));
    out.push('"');
    crate::lines::push_attribute(&mut out, lines);
    out.push_str("><rect x=\"");
    push_num(&mut out, x - w / 2.0);
    out.push_str("\" y=\"");
    push_num(&mut out, y - h / 2.0);
    out.push_str("\" width=\"");
    push_num(&mut out, w);
    out.push_str("\" height=\"");
    push_num(&mut out, h);
    out.push_str("\" rx=\"3\" fill=\"var(--_bg)\"/>");
    out.push_str(&label_svg(label, x, y, attrs, icons));
    out.push_str("</g>");
    out
}

/// A self-loop beside node `c` of drawn size `w` by `h`: out of the side that lies across the
/// flow and back in. `k` counts the loops already on the node, so each one reaches a little
/// further than `base`. `trim` leaves room for the end marker. Also places the edge's label
/// beyond the loop.
#[expect(
    clippy::too_many_arguments,
    clippy::many_single_char_names,
    reason = "mirrors Pele's loopPath"
)]
pub(crate) fn loop_path(
    c: &CNode,
    w: f64,
    h: f64,
    k: f64,
    sideways: bool,
    base: f64,
    trim: f64,
    e: &mut CEdge,
) -> EdgePath {
    let half = (if sideways { w } else { h }) / 2.0;
    let spread = (half - 4.0).min(8.0 + k * 6.0);
    let reach = base + k * 8.0;
    let len = reach.hypot(spread);
    let len = if len == 0.0 || len.is_nan() { 1.0 } else { len };
    // Local frame: `out` points away from the node, `along` runs along its side.
    let at = |d: &mut String, out: f64, along: f64| {
        if sideways {
            push_point(d, c.x + along, c.y + h / 2.0 + out);
        } else {
            push_point(d, c.x + w / 2.0 + out, c.y + along);
        }
    };
    let tx = (reach / len) * trim;
    let ty = (spread / len) * trim;
    if sideways {
        e.label_x = c.x;
        e.label_y = c.y + h / 2.0 + reach + 4.0 + e.label_h / 2.0;
    } else {
        e.label_x = c.x + w / 2.0 + reach + 4.0 + e.label_w / 2.0;
        e.label_y = c.y;
    }

    let mut d = String::from("M");
    at(&mut d, 0.0, -spread);
    d.push('C');
    at(&mut d, reach, -spread * 2.0);
    d.push(' ');
    at(&mut d, reach, spread * 2.0);
    d.push(' ');
    at(&mut d, tx, spread + ty);
    EdgePath {
        d,
        edx: if sideways {
            -spread / len
        } else {
            -reach / len
        },
        edy: if sideways {
            -reach / len
        } else {
            -spread / len
        },
        ex: if sideways {
            c.x + spread
        } else {
            c.x + w / 2.0
        },
        ey: if sideways {
            c.y + h / 2.0
        } else {
            c.y + spread
        },
        sdx: if sideways { 0.0 } else { -1.0 },
        sdy: if sideways { -1.0 } else { 0.0 },
        sx: if sideways {
            c.x - spread
        } else {
            c.x + w / 2.0
        },
        sy: if sideways {
            c.y + h / 2.0
        } else {
            c.y - spread
        },
    }
}

#[cfg(test)]
mod tests {
    //! Expectations come from Pele's `routePath`, `marker`, `edgeLabelSvg`, `loopPath`, and
    //! `sideReach`.

    use super::{EdgePath, SideReach, edge_label_svg, loop_path, marker, marker_trim, route_path};
    use crate::layout::compound::{CEdge, cnode};
    use crate::svg::builder::num;
    use crate::text::label::layout_label;

    fn repr(p: &EdgePath) -> String {
        let nums: Vec<String> = [p.sx, p.sy, p.sdx, p.sdy, p.ex, p.ey, p.edx, p.edy]
            .iter()
            .map(|&v| num(v))
            .collect();
        format!("{}|{}", p.d, nums.join("|"))
    }

    #[expect(clippy::type_complexity, reason = "a table of cases")]
    const ROUTES: &[(
        &[f64],
        Option<&str>,
        f64,
        f64,
        Option<f64>,
        Option<f64>,
        &str,
    )] = &[
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            None,
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            None,
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            None,
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,92|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,92|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,92|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,92|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,92|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,92|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,93|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 0.0, 100.0, 0.0],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,100|0|0|0|-1|0|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            None,
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,50 40,50 40,100|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            None,
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,13C0,50 40,50 40,87L40,93|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            None,
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,43 40,43 40,86L40,92|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,20C0,45 40,45 40,70L40,93|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,47 40,47 40,87L40,100|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L40,100|0|0|-0.37|-0.93|40|100|0.37|0.93",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "M2.6,6.5L37.4,93.5|0|0|-0.37|-0.93|40|100|0.37|0.93",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L37.03,92.57|0|0|-0.37|-0.93|40|100|0.37|0.93",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M1.49,3.71L37.4,93.5|0|0|-0.37|-0.93|40|100|0.37|0.93",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M2.6,6.5L40,100|0|0|-0.37|-0.93|40|100|0.37|0.93",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,38Q0,50 12,50L28,50Q40,50 40,62L40,100|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,38Q0,50 12,50L28,50Q40,50 40,62L40,93|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,38Q0,50 12,50L28,50Q40,50 40,62L40,92|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,38Q0,50 12,50L28,50Q40,50 40,62L40,93|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,38Q0,50 12,50L28,50Q40,50 40,62L40,100|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L28,0Q40,0 40,12L40,100|0|0|-1|0|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M7,0L28,0Q40,0 40,12L40,93|0|0|-1|0|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L28,0Q40,0 40,12L40,92|0|0|-1|0|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L28,0Q40,0 40,12L40,93|0|0|-1|0|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0L28,0Q40,0 40,12L40,100|0|0|-1|0|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,88Q0,100 12,100L40,100|0|0|0|-1|40|100|1|0",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,88Q0,100 12,100L33,100|0|0|0|-1|40|100|1|0",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,88Q0,100 12,100L32,100|0|0|0|-1|40|100|1|0",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,88Q0,100 12,100L33,100|0|0|0|-1|40|100|1|0",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,88Q0,100 12,100L40,100|0|0|0|-1|40|100|1|0",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,50 40,50 40,100|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,13C0,50 40,50 40,87L40,93|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,43 40,43 40,86L40,92|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,20C0,45 40,45 40,70L40,93|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 40.0, 100.0, 0.0],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,47 40,47 40,87L40,100|0|0|0|-1|40|100|0|1",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            None,
            0.0,
            0.0,
            None,
            None,
            "M0,0C50,0 50,30 100,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            None,
            7.0,
            7.0,
            None,
            None,
            "M7,0L13,0C50,0 50,30 87,30L93,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            None,
            0.0,
            8.0,
            None,
            None,
            "M0,0C43,0 43,30 86,30L92,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L20,0C45,0 45,30 70,30L93,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0C47,0 47,30 87,30L100,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L100,30|0|0|-0.96|-0.29|100|30|0.96|0.29",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "M6.7,2.01L93.3,27.99|0|0|-0.96|-0.29|100|30|0.96|0.29",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L92.34,27.7|0|0|-0.96|-0.29|100|30|0.96|0.29",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M3.83,1.15L93.3,27.99|0|0|-0.96|-0.29|100|30|0.96|0.29",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M6.7,2.01L100,30|0|0|-0.96|-0.29|100|30|0.96|0.29",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L38,0Q50,0 50,12L50,18Q50,30 62,30L100,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M7,0L38,0Q50,0 50,12L50,18Q50,30 62,30L93,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L38,0Q50,0 50,12L50,18Q50,30 62,30L92,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L38,0Q50,0 50,12L50,18Q50,30 62,30L93,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0L38,0Q50,0 50,12L50,18Q50,30 62,30L100,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,18Q0,30 12,30L100,30|0|0|0|-1|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,18Q0,30 12,30L93,30|0|0|0|-1|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,18Q0,30 12,30L92,30|0|0|0|-1|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,18Q0,30 12,30L93,30|0|0|0|-1|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,18Q0,30 12,30L100,30|0|0|0|-1|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L88,0Q100,0 100,12L100,30|0|0|-1|0|100|30|0|1",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M7,0L88,0Q100,0 100,12L100,23|0|0|-1|0|100|30|0|1",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L88,0Q100,0 100,12L100,22|0|0|-1|0|100|30|0|1",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L88,0Q100,0 100,12L100,23|0|0|-1|0|100|30|0|1",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0L88,0Q100,0 100,12L100,30|0|0|-1|0|100|30|0|1",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M0,0C50,0 50,30 100,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M7,0L13,0C50,0 50,30 87,30L93,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M0,0C43,0 43,30 86,30L92,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L20,0C45,0 45,30 70,30L93,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[0.0, 0.0, 1.0, 100.0, 30.0, 1.0],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0C47,0 47,30 87,30L100,30|0|0|-1|0|100|30|1|0",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            None,
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,31 15.2,50 40,50C64.8,50 80,76.6 80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            None,
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,13C0,35.94 15.2,50 40,50C64.8,50 80,71.66 80,107L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            None,
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,31 15.2,50 40,50C64.8,50 80,71.28 80,106L80,112|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,20C0,38.6 15.2,50 40,50C64.8,50 80,65.2 80,90L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,33.66 15.2,50 40,50C64.8,50 80,71.66 80,107L80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,50L80,50L80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,50L80,50L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,50L80,50L80,112|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,50L80,50L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,50L80,50L80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,38Q0,50 12,50L68,50Q80,50 80,62L80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,38Q0,50 12,50L68,50Q80,50 80,62L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,38Q0,50 12,50L68,50Q80,50 80,62L80,112|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,38Q0,50 12,50L68,50Q80,50 80,62L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,38Q0,50 12,50L68,50Q80,50 80,62L80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,38Q0,50 12,50L68,50Q80,50 80,62L80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,38Q0,50 12,50L68,50Q80,50 80,62L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,38Q0,50 12,50L68,50Q80,50 80,62L80,112|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,38Q0,50 12,50L68,50Q80,50 80,62L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,38Q0,50 12,50L68,50Q80,50 80,62L80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,38Q0,50 12,50L68,50Q80,50 80,62L80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,38Q0,50 12,50L68,50Q80,50 80,62L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,38Q0,50 12,50L68,50Q80,50 80,62L80,112|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,38Q0,50 12,50L68,50Q80,50 80,62L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,38Q0,50 12,50L68,50Q80,50 80,62L80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,31 15.2,50 40,50C64.8,50 80,76.6 80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,13C0,35.94 15.2,50 40,50C64.8,50 80,71.66 80,107L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,31 15.2,50 40,50C64.8,50 80,71.28 80,106L80,112|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,20C0,38.6 15.2,50 40,50C64.8,50 80,65.2 80,90L80,113|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 50.0, 0.0, 80.0, 50.0, 1.0, 80.0, 120.0, 0.0,
            ],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,33.66 15.2,50 40,50C64.8,50 80,71.66 80,107L80,120|0|0|0|-1|80|120|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            None,
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,24.8 11.4,40 30,40C48.6,40 60,47.6 60,60C60,72.4 71.4,80 90,80C108.6,80 120,102.8 120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            None,
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,13C0,29.74 11.4,40 30,40C48.6,40 60,47.6 60,60C60,72.4 71.4,80 90,80C108.6,80 120,97.86 120,127L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            None,
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,24.8 11.4,40 30,40C48.6,40 60,47.6 60,60C60,72.4 71.4,80 90,80C108.6,80 120,97.48 120,126L120,132|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,19.75C0,32.31 11.4,40 30,40C48.6,40 60,47.6 60,60C60,72.4 71.4,80 90,80C108.6,80 120,91.4 120,110L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,27.46 11.4,40 30,40C48.6,40 60,47.6 60,60C60,72.4 71.4,80 90,80C108.6,80 120,97.86 120,127L120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,40L60,40L60,80L120,80L120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,40L60,40L60,80L120,80L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,40L60,40L60,80L120,80L120,132|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,40L60,40L60,80L120,80L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,40L60,40L60,80L120,80L120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,132|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,132|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,132|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,28Q0,40 12,40L48,40Q60,40 60,52L60,68Q60,80 72,80L108,80Q120,80 120,92L120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,24.8 11.4,40 30,40C48.6,40 60,47.6 60,60C60,72.4 71.4,80 90,80C108.6,80 120,102.8 120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,13C0,29.74 11.4,40 30,40C48.6,40 60,47.6 60,60C60,72.4 71.4,80 90,80C108.6,80 120,97.86 120,127L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,24.8 11.4,40 30,40C48.6,40 60,47.6 60,60C60,72.4 71.4,80 90,80C108.6,80 120,97.48 120,126L120,132|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,19.75C0,32.31 11.4,40 30,40C48.6,40 60,47.6 60,60C60,72.4 71.4,80 90,80C108.6,80 120,91.4 120,110L120,133|0|0|0|-1|120|140|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 40.0, 0.0, 60.0, 40.0, 1.0, 60.0, 80.0, 0.0, 120.0, 80.0, 1.0,
                120.0, 140.0, 0.0,
            ],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,27.46 11.4,40 30,40C48.6,40 60,47.6 60,60C60,72.4 71.4,80 90,80C108.6,80 120,97.86 120,127L120,140|0|0|0|-1|120|140|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            None,
            0.0,
            0.0,
            None,
            None,
            "M10,10L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            None,
            7.0,
            7.0,
            None,
            None,
            "M10,17L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            None,
            0.0,
            8.0,
            None,
            None,
            "M10,10L10,102|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M10,14L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M10,17L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "M10,10L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "M10,17L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "M10,10L10,102|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M10,14L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M10,17L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M10,10L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M10,17L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M10,10L10,102|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M10,14L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M10,17L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M10,10L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M10,17L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M10,10L10,102|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M10,14L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M10,17L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M10,10L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M10,17L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M10,10L10,102|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M10,14L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M10,17L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M10,10L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M10,17L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M10,10L10,102|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M10,14L10,103|10|10|0|-1|10|110|0|1",
        ),
        (
            &[10.0, 10.0, 0.0, 10.0, 60.0, 0.0, 10.0, 110.0, 0.0],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M10,17L10,110|10|10|0|-1|10|110|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            None,
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,30 30,30 30,60L30,120C30,160 -20,160 -20,200|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            None,
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,13C0,36.5 30,36.5 30,60L30,120C30,153.5 -20,153.5 -20,187L-20,193|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            None,
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,30 30,30 30,60L30,120C30,153 -20,153 -20,186L-20,192|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,20C0,40 30,40 30,60L30,120C30,145 -20,145 -20,170L-20,193|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,33.5 30,33.5 30,60L30,120C30,153.5 -20,153.5 -20,187L-20,200|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L30,60L30,120L-20,200|0|0|-0.45|-0.89|-20|200|-0.53|0.85",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "M3.13,6.26L30,60L30,120L-16.29,194.06|0|0|-0.45|-0.89|-20|200|-0.53|0.85",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L30,60L30,120L-15.76,193.22|0|0|-0.45|-0.89|-20|200|-0.53|0.85",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M1.79,3.58L30,60L30,120L-16.29,194.06|0|0|-0.45|-0.89|-20|200|-0.53|0.85",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M3.13,6.26L30,60L30,120L-20,200|0|0|-0.45|-0.89|-20|200|-0.53|0.85",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,18Q0,30 12,30L18,30Q30,30 30,42L30,148Q30,160 18,160L-8,160Q-20,160 -20,172L-20,200|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,18Q0,30 12,30L18,30Q30,30 30,42L30,148Q30,160 18,160L-8,160Q-20,160 -20,172L-20,193|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,18Q0,30 12,30L18,30Q30,30 30,42L30,148Q30,160 18,160L-8,160Q-20,160 -20,172L-20,192|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,18Q0,30 12,30L18,30Q30,30 30,42L30,148Q30,160 18,160L-8,160Q-20,160 -20,172L-20,193|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,18Q0,30 12,30L18,30Q30,30 30,42L30,148Q30,160 18,160L-8,160Q-20,160 -20,172L-20,200|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L18,0Q30,0 30,12L30,108Q30,120 18,120L-8,120Q-20,120 -20,132L-20,200|0|0|-1|0|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M7,0L18,0Q30,0 30,12L30,108Q30,120 18,120L-8,120Q-20,120 -20,132L-20,193|0|0|-1|0|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L18,0Q30,0 30,12L30,108Q30,120 18,120L-8,120Q-20,120 -20,132L-20,192|0|0|-1|0|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L18,0Q30,0 30,12L30,108Q30,120 18,120L-8,120Q-20,120 -20,132L-20,193|0|0|-1|0|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0L18,0Q30,0 30,12L30,108Q30,120 18,120L-8,120Q-20,120 -20,132L-20,200|0|0|-1|0|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,48Q0,60 12,60L18,60Q30,60 30,72L30,188Q30,200 18,200L-20,200|0|0|0|-1|-20|200|-1|0",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,48Q0,60 12,60L18,60Q30,60 30,72L30,188Q30,200 18,200L-13,200|0|0|0|-1|-20|200|-1|0",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,48Q0,60 12,60L18,60Q30,60 30,72L30,188Q30,200 18,200L-12,200|0|0|0|-1|-20|200|-1|0",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,48Q0,60 12,60L18,60Q30,60 30,72L30,188Q30,200 18,200L-13,200|0|0|0|-1|-20|200|-1|0",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,48Q0,60 12,60L18,60Q30,60 30,72L30,188Q30,200 18,200L-20,200|0|0|0|-1|-20|200|-1|0",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,30 30,30 30,60L30,120C30,160 -20,160 -20,200|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,13C0,36.5 30,36.5 30,60L30,120C30,153.5 -20,153.5 -20,187L-20,193|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,30 30,30 30,60L30,120C30,153 -20,153 -20,186L-20,192|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,20C0,40 30,40 30,60L30,120C30,145 -20,145 -20,170L-20,193|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 30.0, 60.0, 0.0, 30.0, 120.0, 0.0, -20.0, 200.0, 0.0,
            ],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,33.5 30,33.5 30,60L30,120C30,153.5 -20,153.5 -20,187L-20,200|0|0|0|-1|-20|200|0|1",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            None,
            0.0,
            0.0,
            None,
            None,
            "M0,0L50,0C75,0 75,40 100,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            None,
            7.0,
            7.0,
            None,
            None,
            "M7,0L50,0C68.5,0 68.5,40 87,40L93,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            None,
            0.0,
            8.0,
            None,
            None,
            "M0,0L50,0C68,0 68,40 86,40L92,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L50,0C62.09,0 62.09,40 74.19,40L93,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0L50,0C68.5,0 68.5,40 87,40L100,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L50,0L100,40|0|0|-1|0|100|40|0.78|0.62",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "M7,0L50,0L94.53,35.63|0|0|-1|0|100|40|0.78|0.62",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L50,0L93.75,35|0|0|-1|0|100|40|0.78|0.62",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L50,0L94.53,35.63|0|0|-1|0|100|40|0.78|0.62",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0L50,0L100,40|0|0|-1|0|100|40|0.78|0.62",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L50,0L63,0.01Q75,0.01 75,12.01L75,28Q75,40 87,40L100,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M7,0L50,0L63,0.01Q75,0.01 75,12.01L75,28Q75,40 87,40L93,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L50,0L63,0.01Q75,0.01 75,12.01L75,28Q75,40 87,40L92,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L50,0L63,0.01Q75,0.01 75,12.01L75,29.88Q75,40 85.13,40L93,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0L50,0L63,0.01Q75,0.01 75,12.01L75,28Q75,40 87,40L100,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L50,0L50,28Q50,40 62,40L100,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M7,0L50,0L50,28Q50,40 62,40L93,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L50,0L50,28Q50,40 62,40L92,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L50,0L50,28Q50,40 62,40L93,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0L50,0L50,28Q50,40 62,40L100,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L50,0L88,0.01Q100,0.01 100,12.01L100,40|0|0|-1|0|100|40|0|1",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M7,0L50,0L88,0.01Q100,0.01 100,12.01L100,33|0|0|-1|0|100|40|0|1",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L50,0L88,0.01Q100,0.01 100,12.01L100,32|0|0|-1|0|100|40|0|1",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L50,0L88,0.01Q100,0.01 100,12.01L100,33|0|0|-1|0|100|40|0|1",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0L50,0L88,0.01Q100,0.01 100,12.01L100,40|0|0|-1|0|100|40|0|1",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L50,0C75,0 75,40 100,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M7,0L50,0C68.5,0 68.5,40 87,40L93,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L50,0C68,0 68,40 86,40L92,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0L50,0C62.09,0 62.09,40 74.19,40L93,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[
                0.0, 0.0, 1.0, 50.0, 0.0, 1.0, 50.0, 0.005, 0.0, 100.0, 40.0, 1.0,
            ],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0L50,0C68.5,0 68.5,40 87,40L100,40|0|0|-1|0|100|40|1|0",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            None,
            0.0,
            0.0,
            None,
            None,
            "M5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            None,
            7.0,
            7.0,
            None,
            None,
            "M5,12L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            None,
            0.0,
            8.0,
            None,
            None,
            "M5,5L5,82|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M5,9L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M5,12L5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "M5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "M5,12L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "M5,5L5,82|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M5,9L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M5,12L5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M5,12L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M5,5L5,82|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M5,9L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M5,12L5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M5,12L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M5,5L5,82|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M5,9L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M5,12L5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M5,12L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M5,5L5,82|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M5,9L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M5,12L5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M5,12L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M5,5L5,82|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M5,9L5,5L5,83|5|5|0|-1|5|90|0|1",
        ),
        (
            &[5.0, 5.0, 0.0, 5.0, 5.004, 0.0, 5.0, 90.0, 0.0],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M5,12L5,5L5,90|5|5|0|-1|5|90|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            None,
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,1 3,1 3,2|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            None,
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,10C0,1 3,1 3,-8L3,-5|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            None,
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,-3.75 3,-3.75 3,-7.5L3,-6|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,7.94C0,-0.5 3,-0.5 3,-8.94L3,-5|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,3.41 3,3.41 3,-0.19L3,2|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L3,2|0|0|-0.83|-0.55|3|2|0.83|0.55",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "M5.82,3.88L-2.82,-1.88|0|0|-0.83|-0.55|3|2|0.83|0.55",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L-3.66,-2.44|0|0|-0.83|-0.55|3|2|0.83|0.55",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M3.33,2.22L-2.82,-1.88|0|0|-0.83|-0.55|3|2|0.83|0.55",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M5.82,3.88L3,2|0|0|-0.83|-0.55|3|2|0.83|0.55",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M0,0Q0,1 1,1L2,1Q3,1 3,2|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,8.5L0,2.5Q0,1 1.5,1Q3,1 3,-0.5L3,-6.5L3,-5|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M0,0Q0,1 1,1L1.5,1Q3,1 3,-0.5L3,-7.75L3,-6|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,5.31L0,2.5Q0,1 1.5,1Q3,1 3,-0.5L3,-7.62L3,-5|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,2.5Q0,1 1.5,1L2,1Q3,1 3,2|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L1,0Q3,0 3,2|0|0|-1|0|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M7,0L8,0Q3,0 3,-5L3,-6.25L3,-5|0|0|-1|0|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M0,0Q3,0 3,-3L3,-7.5L3,-6|0|0|-1|0|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,0Q3,0 3,-1L3,-7.19L3,-5|0|0|-1|0|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0L5,0Q3,0 3,2|0|0|-1|0|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M0,0Q0,2 2,2L3,2|0|0|0|-1|3|2|1|0",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,8.25L0,7Q0,2 -5,2L-4,2|0|0|0|-1|3|2|1|0",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M0,0Q0,2 -2,2L-6.25,2L-5,2|0|0|0|-1|3|2|1|0",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4Q0,2 -2,2L-5.75,2L-4,2|0|0|0|-1|3|2|1|0",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,3.69Q0,2 1.69,2L3,2|0|0|0|-1|3|2|1|0",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,1 3,1 3,2|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,10C0,1 3,1 3,-8L3,-5|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,-3.75 3,-3.75 3,-7.5L3,-6|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,7.94C0,-0.5 3,-0.5 3,-8.94L3,-5|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0, 3.0, 2.0, 0.0],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,3.41 3,3.41 3,-0.19L3,2|0|0|0|-1|3|2|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            None,
            0.0,
            0.0,
            None,
            None,
            "M0,0|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            None,
            7.0,
            7.0,
            None,
            None,
            "M7,-7|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            None,
            0.0,
            8.0,
            None,
            None,
            "M0,-8|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,-7|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "MNaN,NaN|0|0|NaN|NaN|0|0|NaN|NaN",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "MNaN,NaN|0|0|NaN|NaN|0|0|NaN|NaN",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "MNaN,NaN|0|0|NaN|NaN|0|0|NaN|NaN",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "MNaN,NaN|0|0|NaN|NaN|0|0|NaN|NaN",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "MNaN,NaN|0|0|NaN|NaN|0|0|NaN|NaN",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M0,0|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M7,-7|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M0,-8|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,-7|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M0,0|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M7,-7|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M0,-8|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,-7|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M0,0|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M7,-7|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M0,-8|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,-7|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M0,0|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M7,-7|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M0,-8|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M4,-7|0|0|-1|0|0|0|0|1",
        ),
        (
            &[0.0, 0.0, 0.0],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M7,0|0|0|-1|0|0|0|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            None,
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,18.6 19,30 50,30C81,30 100,41.4 100,60|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            None,
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,12.75C0,23.45 19,30 50,30C81,30 100,36.56 100,47.25L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            None,
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,18.6 19,30 50,30C81,30 100,36.27 100,46.5L100,52|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            None,
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,15.38C0,24.44 19,30 50,30C81,30 100,34.92 100,42.94L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            None,
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,21.26 19,30 50,30C81,30 100,36.56 100,47.25L100,60|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("linear"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,30L100,30L100,60|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("linear"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,30L100,30L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("linear"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,30L100,30L100,52|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("linear"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,30L100,30L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("linear"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,30L100,30L100,60|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("step"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,18Q0,30 12,30L88,30Q100,30 100,42L100,60|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("step"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,18Q0,30 12,30L88,30Q100,30 100,42L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("step"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,18Q0,30 12,30L88,30Q100,30 100,42L100,52|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("step"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,18Q0,30 12,30L88,30Q100,30 100,42L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("step"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,18Q0,30 12,30L88,30Q100,30 100,42L100,60|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("stepBefore"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,18Q0,30 12,30L88,30Q100,30 100,42L100,60|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("stepBefore"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,18Q0,30 12,30L88,30Q100,30 100,42L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("stepBefore"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,18Q0,30 12,30L88,30Q100,30 100,42L100,52|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("stepBefore"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,18Q0,30 12,30L88,30Q100,30 100,42L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("stepBefore"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,18Q0,30 12,30L88,30Q100,30 100,42L100,60|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("stepAfter"),
            0.0,
            0.0,
            None,
            None,
            "M0,0L0,18Q0,30 12,30L88,30Q100,30 100,42L100,60|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("stepAfter"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,18Q0,30 12,30L88,30Q100,30 100,42L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("stepAfter"),
            0.0,
            8.0,
            None,
            None,
            "M0,0L0,18Q0,30 12,30L88,30Q100,30 100,42L100,52|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("stepAfter"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,18Q0,30 12,30L88,30Q100,30 100,42L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("stepAfter"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7L0,18Q0,30 12,30L88,30Q100,30 100,42L100,60|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("basis"),
            0.0,
            0.0,
            None,
            None,
            "M0,0C0,18.6 19,30 50,30C81,30 100,41.4 100,60|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("basis"),
            7.0,
            7.0,
            None,
            None,
            "M0,7L0,12.75C0,23.45 19,30 50,30C81,30 100,36.56 100,47.25L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("basis"),
            0.0,
            8.0,
            None,
            None,
            "M0,0C0,18.6 19,30 50,30C81,30 100,36.27 100,46.5L100,52|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("basis"),
            4.0,
            7.0,
            Some(20.0),
            Some(30.0),
            "M0,4L0,15.38C0,24.44 19,30 50,30C81,30 100,34.92 100,42.94L100,53|0|0|0|-1|100|60|0|1",
        ),
        (
            &[
                0.0, 0.0, 0.0, 0.0, 30.0, 0.0, 100.0, 30.0, 1.0, 100.0, 60.0, 0.0,
            ],
            Some("basis"),
            7.0,
            0.0,
            Some(0.0),
            Some(13.0),
            "M0,7C0,21.26 19,30 50,30C81,30 100,36.56 100,47.25L100,60|0|0|0|-1|100|60|0|1",
        ),
    ];

    #[test]
    fn route_path_matches_pele() {
        for &(route, curve, start_trim, end_trim, start_reach, end_reach, expected) in ROUTES {
            let path = route_path(route, curve, start_trim, end_trim, start_reach, end_reach);
            assert_eq!(
                repr(&path),
                expected,
                "{route:?} {curve:?} {start_trim} {end_trim}"
            );
        }
    }

    #[test]
    #[expect(clippy::float_cmp, reason = "exact values from Pele")]
    fn marker_matches_pele() {
        for &(r#type, dx, dy, expected, trim) in &[
            (
                "arrow_point",
                0.0,
                1.0,
                "<path class=\"ondine-marker\" d=\"M40.5,12.25L37.14,4.25L43.86,4.25Z\" fill=\"var(--_l)\" stroke=\"none\"/>",
                7.0,
            ),
            (
                "arrow_point",
                1.0,
                0.0,
                "<path class=\"ondine-marker\" d=\"M40.5,12.25L32.5,15.61L32.5,8.89Z\" fill=\"var(--_l)\" stroke=\"none\"/>",
                7.0,
            ),
            (
                "arrow_point",
                -0.6,
                0.8,
                "<path class=\"ondine-marker\" d=\"M40.5,12.25L42.61,3.83L47.99,7.87Z\" fill=\"var(--_l)\" stroke=\"none\"/>",
                7.0,
            ),
            (
                "arrow_point",
                0.0,
                -1.0,
                "<path class=\"ondine-marker\" d=\"M40.5,12.25L43.86,20.25L37.14,20.25Z\" fill=\"var(--_l)\" stroke=\"none\"/>",
                7.0,
            ),
            (
                "arrow_circle",
                0.0,
                1.0,
                "<circle class=\"ondine-marker\" cx=\"40.5\" cy=\"8.25\" r=\"3.5\" fill=\"var(--_l)\" stroke=\"none\"/>",
                8.0,
            ),
            (
                "arrow_circle",
                1.0,
                0.0,
                "<circle class=\"ondine-marker\" cx=\"36.5\" cy=\"12.25\" r=\"3.5\" fill=\"var(--_l)\" stroke=\"none\"/>",
                8.0,
            ),
            (
                "arrow_circle",
                -0.6,
                0.8,
                "<circle class=\"ondine-marker\" cx=\"42.9\" cy=\"9.05\" r=\"3.5\" fill=\"var(--_l)\" stroke=\"none\"/>",
                8.0,
            ),
            (
                "arrow_circle",
                0.0,
                -1.0,
                "<circle class=\"ondine-marker\" cx=\"40.5\" cy=\"16.25\" r=\"3.5\" fill=\"var(--_l)\" stroke=\"none\"/>",
                8.0,
            ),
            (
                "arrow_cross",
                0.0,
                1.0,
                "<path class=\"ondine-marker\" d=\"M44,4.75L37,11.75M37,4.75L44,11.75\"/>",
                4.0,
            ),
            (
                "arrow_cross",
                1.0,
                0.0,
                "<path class=\"ondine-marker\" d=\"M33,8.75L40,15.75M33,15.75L40,8.75\"/>",
                4.0,
            ),
            (
                "arrow_cross",
                -0.6,
                0.8,
                "<path class=\"ondine-marker\" d=\"M47.8,8.35L38,9.75M42.2,4.15L43.6,13.95\"/>",
                4.0,
            ),
            (
                "arrow_cross",
                0.0,
                -1.0,
                "<path class=\"ondine-marker\" d=\"M37,19.75L44,12.75M44,19.75L37,12.75\"/>",
                4.0,
            ),
            ("none", 0.0, 1.0, "", 0.0),
            ("none", 1.0, 0.0, "", 0.0),
            ("none", -0.6, 0.8, "", 0.0),
            ("none", 0.0, -1.0, "", 0.0),
            ("", 0.0, 1.0, "", 0.0),
            ("", 1.0, 0.0, "", 0.0),
            ("", -0.6, 0.8, "", 0.0),
            ("", 0.0, -1.0, "", 0.0),
        ] {
            assert_eq!(marker(r#type, 40.5, 12.25, dx, dy, "var(--_l)"), expected);
            assert_eq!(marker_trim(r#type), trim);
        }
    }

    #[test]
    fn edge_label_svg_matches_pele() {
        let icons = |name: &str| (name == "fa:fa-car").then(|| "<path d=\"M1 1\"/>".to_owned());
        let resolver: &dyn Fn(&str) -> Option<String> = &icons;
        let label = layout_label(
            Some("yes fa:fa-car"),
            false,
            14.0,
            4000.0,
            0,
            Some(resolver),
        );
        assert_eq!(
            edge_label_svg(
                "L_A_B_0",
                &[],
                &label,
                30.0,
                40.0,
                " class=\"t\"",
                Some(resolver)
            ),
            "<g class=\"ondine-edge-label\" data-id=\"L_A_B_0\"><rect x=\"0.45\" y=\"29.5\" width=\"59.1\" height=\"21\" rx=\"3\" fill=\"var(--_bg)\"/><text class=\"t\" text-anchor=\"middle\" xml:space=\"preserve\"><tspan x=\"4.45\" y=\"44.9\" text-anchor=\"start\">yes </tspan></text><svg class=\"ondine-icon\" data-icon=\"fa:fa-car\" x=\"38.05\" y=\"32.3\" width=\"15.4\" height=\"15.4\" viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"><path d=\"M1 1\"/></svg></g>"
        );
        let label = layout_label(
            Some("<b>two</b><br>lines & more"),
            false,
            14.0,
            4000.0,
            0,
            None,
        );
        assert_eq!(
            edge_label_svg("a&amp;\"b\"", &[], &label, -3.333, 7.0, "", None),
            "<g class=\"ondine-edge-label\" data-id=\"a&amp;&quot;b&quot;\"><rect x=\"-57.73\" y=\"-14\" width=\"108.8\" height=\"42\" rx=\"3\" fill=\"var(--_bg)\"/><text text-anchor=\"middle\" xml:space=\"preserve\"><tspan x=\"-3.33\" y=\"1.4\" font-weight=\"var(--_w)\">two</tspan><tspan x=\"-3.33\" y=\"22.4\">lines &amp; more</tspan></text></g>"
        );
    }

    #[test]
    fn loop_path_matches_pele() {
        for &(w, h, k, sideways, base, trim, expected, label) in &[
            (
                60.0,
                40.0,
                0.0,
                false,
                24.0,
                7.0,
                "M130,42C154,34 154,66 136.64,60.21|130|42|-1|0|130|58|-0.95|-0.32",
                "168,50",
            ),
            (
                60.0,
                40.0,
                1.0,
                true,
                24.0,
                0.0,
                "M86,70C72,102 128,102 114,70|86|70|0|-1|114|70|-0.4|-0.92",
                "100,113",
            ),
            (
                10.0,
                6.0,
                2.0,
                false,
                18.0,
                8.0,
                "M105,51C139,52 139,48 113,48.76|105|51|-1|0|105|49|-1|0.03",
                "153,50",
            ),
            (
                80.0,
                30.0,
                3.0,
                true,
                30.0,
                4.0,
                "M74,65C48,119 152,119 127.74,68.6|74|65|0|-1|126|65|-0.43|-0.9",
                "100,130",
            ),
        ] {
            let mut c = cnode(w, h, -1);
            c.x = 100.0;
            c.y = 50.0;
            let mut e = CEdge {
                label_h: 14.0,
                label_w: 20.0,
                ..CEdge::default()
            };
            let path = loop_path(&c, w, h, k, sideways, base, trim, &mut e);
            assert_eq!(repr(&path), expected);
            assert_eq!(format!("{},{}", num(e.label_x), num(e.label_y)), label);
        }
    }

    #[test]
    fn side_reach_matches_pele() {
        let mut reach = SideReach::default();
        reach.add(0, 1, "arrow_point");
        reach.add(0, 1, "arrow_circle");
        reach.add(0, 1, "arrow_point");
        reach.add(2, 0, "arrow_cross");
        reach.add(2, -1, "arrow_circle");
        reach.add(3, 2, "none");
        let got: Vec<String> = [(0, 1), (0, 0), (2, 0), (2, -1), (3, 2)]
            .iter()
            .map(|&(n, s)| num(reach.of(n, s)))
            .collect();
        assert_eq!(got.join(","), "14,0,10,0,0");
    }
}
