//! Cluster titles placed clear of the edges that cross a cluster's top border. Port of Pele's
//! `src/svg/cluster.ts`.

use std::collections::BTreeMap;

use crate::layout::compound::CNode;
use crate::svg::builder::{js_round, label_svg, push_num};
use crate::text::icons::IconResolver;
use crate::text::label::Label;

/// The x of every route point, by its rounded y: the edges that cross a cluster's top border.
pub(crate) type Crossings = BTreeMap<i64, Vec<f64>>;

/// A rounded y as a map key.
fn key(y: f64) -> i64 {
    // ponytail: NaN and values past `i64` saturate, where Pele would key them by their own value.
    #[expect(clippy::cast_possible_truncation, reason = "a rounded coordinate")]
    let key = js_round(y) as i64;
    key
}

/// Records the points of a route of `x, y, axis` triples.
pub(crate) fn mark_crossings(crossings: &mut Crossings, route: &[f64]) {
    for point in route.chunks(3) {
        if let [x, y, ..] = *point {
            crossings.entry(key(y)).or_default().push(x);
        }
    }
}

/// Picks where a cluster title of `width` sits along the top edge so that edges entering there do
/// not cross it. `pad` is the cluster's padding.
pub(crate) fn cluster_title_x(c: &CNode, width: f64, crossings: &Crossings, pad: f64) -> f64 {
    if width == 0.0 {
        return c.x;
    }

    let top = c.y - c.h / 2.0;
    let left = c.x - c.w / 2.0;
    let half = width / 2.0;
    let candidates = [left + pad - 6.0 + half, c.x, left + c.w - pad + 6.0 - half];
    let Some(xs) = crossings.get(&key(top)) else {
        return candidates[0];
    };

    let mut best = candidates[0];
    let mut best_gap = -1.0;
    for x in candidates {
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

/// Whether an edge that enters at the top runs through a title of `width` placed at `x`.
pub(crate) fn title_crossed(c: &CNode, width: f64, x: f64, crossings: &Crossings) -> bool {
    if width.is_nan() || width <= 0.0 {
        return false;
    }
    let Some(xs) = crossings.get(&key(c.y - c.h / 2.0)) else {
        return false;
    };

    let left = c.x - c.w / 2.0;
    xs.iter()
        .any(|&cx| cx > left && cx < left + c.w && (cx - x).abs() < width / 2.0 + 4.0)
}

/// A title that an edge runs through, drawn over the edges: the text on an outline in the color
/// of the cluster behind it, so only the line is seen to stop short of the letters. `nodes` and
/// `index` find how many clusters deep the title is, each of which tints the background. `id` is
/// written as given, and `lines` are the cluster's authored source lines, if any.
#[expect(clippy::too_many_arguments, reason = "mirrors Pele's struckTitle")]
pub(crate) fn struck_title(
    label: &Label,
    x: f64,
    y: f64,
    attrs: &str,
    id: &str,
    lines: &[u32],
    nodes: &[CNode],
    index: usize,
    icons: Option<IconResolver<'_>>,
) -> String {
    let mut depth = 1;
    let mut parent = nodes.get(index).map_or(-1, |node| node.parent);
    while let Ok(p) = usize::try_from(parent)
        && depth < 8
    {
        depth += 1;
        parent = nodes.get(p).map_or(-1, |node| node.parent);
    }

    // Each cluster is half its fill over what is behind it.
    let mut halo = String::from(" fill=\"none\" stroke=\"color-mix(in srgb,var(--_a) ");
    push_num(&mut halo, 100.0 * (1.0 - 0.5_f64.powi(depth)));
    halo.push_str("%,var(--_bg))\" stroke-width=\"5\" stroke-linejoin=\"round\"");

    let mut out = String::from("<g class=\"ondine-cluster-title\" data-id=\"");
    out.push_str(id);
    out.push('"');
    crate::lines::push_attribute(&mut out, lines);
    out.push('>');
    out.push_str(&label_svg(label, x, y, &halo, icons));
    out.push_str(&label_svg(label, x, y, attrs, icons));
    out.push_str("</g>");
    out
}

#[cfg(test)]
mod tests {
    //! Expectations come from Pele's `markCrossings`, `clusterTitleX`, `titleCrossed`, and
    //! `struckTitle`.

    use super::{Crossings, cluster_title_x, mark_crossings, struck_title, title_crossed};
    use crate::layout::compound::cnode;
    use crate::svg::builder::num;
    use crate::text::label::layout_label;

    #[test]
    fn title_placement_matches_pele() {
        let mut crossings = Crossings::new();
        mark_crossings(&mut crossings, &[100.0, 49.6, 0.0, 100.0, 120.0, 0.0]);
        mark_crossings(
            &mut crossings,
            &[60.0, 50.4, 0.0, 60.0, 90.0, 0.0, 400.0, 50.0, 0.0],
        );
        for &((x, y, w, h), width, expected, crossed, crossed_middle) in &[
            ((100.0, 100.0, 200.0, 100.0), 0.0, "100", false, false),
            ((100.0, 100.0, 200.0, 100.0), 40.0, "22", false, true),
            ((100.0, 100.0, 200.0, 100.0), 80.0, "158", false, true),
            ((100.0, 100.0, 200.0, 100.0), 150.0, "77", true, true),
            ((100.0, 150.0, 200.0, 100.0), 0.0, "100", false, false),
            ((100.0, 150.0, 200.0, 100.0), 40.0, "22", false, false),
            ((100.0, 150.0, 200.0, 100.0), 80.0, "42", false, false),
            ((100.0, 150.0, 200.0, 100.0), 150.0, "77", false, false),
            ((70.0, 100.0, 140.0, 100.0), 0.0, "70", false, false),
            ((70.0, 100.0, 140.0, 100.0), 40.0, "22", false, true),
            ((70.0, 100.0, 140.0, 100.0), 80.0, "42", true, true),
            ((70.0, 100.0, 140.0, 100.0), 150.0, "77", true, true),
            ((300.0, 100.0, 200.0, 100.0), 0.0, "300", false, false),
            ((300.0, 100.0, 200.0, 100.0), 40.0, "222", false, false),
            ((300.0, 100.0, 200.0, 100.0), 80.0, "242", false, false),
            ((300.0, 100.0, 200.0, 100.0), 150.0, "277", false, false),
        ] {
            let mut c = cnode(w, h, -1);
            c.x = x;
            c.y = y;
            c.is_group = true;
            let tx = cluster_title_x(&c, width, &crossings, 8.0);
            assert_eq!(num(tx), expected, "{x} {y} {width}");
            assert_eq!(
                title_crossed(&c, width, tx, &crossings),
                crossed,
                "{x} {y} {width}"
            );
            assert_eq!(
                title_crossed(&c, width, x, &crossings),
                crossed_middle,
                "{x} {y} {width}"
            );
        }
    }

    #[test]
    fn struck_title_matches_pele() {
        let mut nodes = vec![
            cnode(10.0, 10.0, -1),
            cnode(10.0, 10.0, 0),
            cnode(10.0, 10.0, 1),
        ];
        for node in &mut nodes {
            node.is_group = true;
        }
        let label = layout_label(Some("Cluster *title*"), true, 14.0, 4000.0, 0, None);
        let expected = [
            "<g class=\"ondine-cluster-title\" data-id=\"sg&amp;1\"><text fill=\"none\" stroke=\"color-mix(in srgb,var(--_a) 50%,var(--_bg))\" stroke-width=\"5\" stroke-linejoin=\"round\" text-anchor=\"middle\" xml:space=\"preserve\"><tspan x=\"12\" y=\"39.4\">Cluster </tspan><tspan font-style=\"italic\">title</tspan></text><text class=\"ct\" text-anchor=\"middle\" xml:space=\"preserve\"><tspan x=\"12\" y=\"39.4\">Cluster </tspan><tspan font-style=\"italic\">title</tspan></text></g>",
            "<g class=\"ondine-cluster-title\" data-id=\"sg&amp;1\"><text fill=\"none\" stroke=\"color-mix(in srgb,var(--_a) 75%,var(--_bg))\" stroke-width=\"5\" stroke-linejoin=\"round\" text-anchor=\"middle\" xml:space=\"preserve\"><tspan x=\"12\" y=\"39.4\">Cluster </tspan><tspan font-style=\"italic\">title</tspan></text><text class=\"ct\" text-anchor=\"middle\" xml:space=\"preserve\"><tspan x=\"12\" y=\"39.4\">Cluster </tspan><tspan font-style=\"italic\">title</tspan></text></g>",
            "<g class=\"ondine-cluster-title\" data-id=\"sg&amp;1\"><text fill=\"none\" stroke=\"color-mix(in srgb,var(--_a) 87.5%,var(--_bg))\" stroke-width=\"5\" stroke-linejoin=\"round\" text-anchor=\"middle\" xml:space=\"preserve\"><tspan x=\"12\" y=\"39.4\">Cluster </tspan><tspan font-style=\"italic\">title</tspan></text><text class=\"ct\" text-anchor=\"middle\" xml:space=\"preserve\"><tspan x=\"12\" y=\"39.4\">Cluster </tspan><tspan font-style=\"italic\">title</tspan></text></g>",
        ];
        for (i, expected) in expected.iter().enumerate() {
            assert_eq!(
                struck_title(
                    &label,
                    12.0,
                    34.5,
                    " class=\"ct\"",
                    "sg&amp;1",
                    &[],
                    &nodes,
                    i,
                    None
                ),
                *expected
            );
        }
    }
}
