//! Draws an XY chart. Port of Pele's `src/diagrams/xychart/render.ts`.

use std::collections::BTreeMap;

use super::db::{XAxis, XyChartDb};
use super::ticks::{tick_index, tick_step, tick_value, ticks_between};
use crate::diagrams::Rendered;
use crate::diagrams::common::fit_width::cap_width;
use crate::error::{Code, Error};
use crate::svg::builder::{esc, esc_text, js_round, label_svg, push_num};
use crate::svg::root::{Accessible, RenderOptions, svg_document};
use crate::svg::theme::series_color;
use crate::text::label::{Label, layout_label, trim};
use crate::text::measure::{self, BOLD};
use crate::yaml::{Value, push_number};

const TICK: f64 = 4.0;
const SWATCH: f64 = 12.0;
const ROW: f64 = 26.0;
const LEGEND_GAP: f64 = 24.0;
const LIMIT: f64 = 1e300;

/// A labelled place along an axis, as a fraction of its length.
struct Mark {
    at: f64,
    label: Label,
}

/// JavaScript's `Math.min`, which is `NaN` when either value is.
fn min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.min(b)
    }
}

/// JavaScript's `Math.max`, which is `NaN` when either value is.
fn max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        f64::NAN
    } else {
        a.max(b)
    }
}

/// A property of a YAML mapping, as JavaScript's `value.key` reads it.
fn property<'v>(value: &'v Value, key: &str) -> Option<&'v Value> {
    let Value::Object(entries) = value else {
        return None;
    };
    entries
        .iter()
        .find(|(own, _)| own == key)
        .map(|(_, value)| value)
}

/// A mapping, or nothing for any other value.
fn record(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| matches!(value, Value::Object(_)))
}

/// A boolean setting, or `fallback`.
fn on(config: Option<&Value>, key: &str, fallback: bool) -> bool {
    match config.and_then(|config| property(config, key)) {
        Some(Value::Bool(value)) => *value,
        _ => fallback,
    }
}

fn dimension(value: Option<&Value>, fallback: f64) -> f64 {
    match value {
        Some(Value::Number(value)) if *value > 0.0 => js_round(value.clamp(100.0, 4000.0)),
        _ => fallback,
    }
}

fn finite(value: Option<f64>) -> Option<f64> {
    value
        .filter(|value| value.is_finite())
        .map(|value| value.clamp(-LIMIT, LIMIT))
}

fn widest(marks: &[Mark]) -> f64 {
    marks
        .iter()
        .fold(0.0, |width, mark| max(width, mark.label.width))
}

fn number_text(value: f64) -> String {
    let mut out = String::new();
    push_number(&mut out, value);
    out
}

/// Where a label is anchored.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Anchor {
    End,
    Middle,
    Start,
}

impl Anchor {
    const fn name(self) -> &'static str {
        match self {
            Self::End => "end",
            Self::Middle => "middle",
            Self::Start => "start",
        }
    }
}

/// Writes a label with its left edge, center or right edge at `x`. A plain one-line label keeps
/// its place when the text is wider or narrower than measured.
fn aligned(label: &Label, x: f64, y: f64, anchor: Anchor) -> String {
    if let [line] = label.lines.as_slice()
        && let [span] = line.as_slice()
        && span.icon.is_none()
        && span.style == 0
    {
        let mut out = String::from("<text x=\"");
        push_num(&mut out, x);
        out.push_str("\" y=\"");
        push_num(&mut out, y + label.size * 0.35);
        out.push_str("\" text-anchor=\"");
        out.push_str(anchor.name());
        out.push_str("\">");
        out.push_str(&esc(&span.text));
        out.push_str("</text>");
        return out;
    }
    let dx = match anchor {
        Anchor::Start => label.width / 2.0,
        Anchor::End => -label.width / 2.0,
        Anchor::Middle => 0.0,
    };
    label_svg(label, x + dx, y, "", None)
}

fn text(raw: Option<&str>, font_size: f64, max: f64) -> Label {
    layout_label(raw, false, font_size, max, 0, None)
}

/// Where a stack of bars stands: a category by its index, or a place on a numeric x axis by the
/// value written there. Two categories with one name keep separate stacks.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Stack {
    At(String),
    Index(usize),
}

impl Stack {
    fn of(x: &XAxis, index: usize, at: &str) -> Self {
        match x {
            XAxis::Band { .. } => Self::Index(index),
            XAxis::Linear(_) => Self::At(at.to_owned()),
        }
    }
}

/// The sum of the negative values and the sum of the other values of every bar series at each
/// stack, over the first `slots` values of each series. Series on the right axis, as `right` marks
/// them, stack apart from the others.
fn stack_totals(
    model: &XyChartDb,
    right: &[bool],
    slots: usize,
) -> BTreeMap<(bool, Stack), (f64, f64)> {
    let mut totals = BTreeMap::new();
    for (p, plot) in model.plots.iter().enumerate().filter(|(_, plot)| plot.bar) {
        let side = right.get(p).copied().unwrap_or(false);
        for (index, (at, raw)) in plot.data.iter().enumerate().take(slots) {
            let Some(value) = finite(*raw) else {
                continue;
            };
            let total = totals
                .entry((side, Stack::of(&model.x_axis, index, at)))
                .or_insert((0.0, 0.0));
            if value < 0.0 {
                total.0 = (total.0 + value).max(-LIMIT);
            } else {
                total.1 = (total.1 + value).min(LIMIT);
            }
        }
    }
    totals
}

/// `value` to 12 significant digits, which drops the error that adding decimals leaves.
fn tidy(value: f64) -> f64 {
    format!("{value:.11e}").parse().unwrap_or(value)
}

/// The color of the series at `index` of `count`, as the `--ondine-series-*` variable of its
/// place. With `spread`, its default shade runs evenly from the foreground down to 35% of it
/// across the series of the chart, so that segments of a stack and bars of two axes stay apart.
/// Otherwise it is the shared series color, which keeps every shade above half the foreground.
fn chart_color(index: usize, count: usize, spread: bool) -> String {
    if !spread {
        return series_color(i64::try_from(index).unwrap_or(i64::MAX));
    }
    #[expect(
        clippy::cast_precision_loss,
        reason = "series counts stay far below 2^52"
    )]
    let share = if count > 1 {
        100.0 - 65.0 * index as f64 / (count - 1) as f64
    } else {
        100.0
    };
    let mut out = String::from("var(--ondine-series-");
    out.push(char::from(b'1' + u8::try_from(index % 8).unwrap_or(0)));
    out.push_str(",color-mix(in srgb,var(--_fg) ");
    push_number(&mut out, js_round(share));
    out.push_str("%,var(--_bg)))");
    out
}

/// The value axis: its range, which the marks round out to whole steps unless it was written.
struct Values {
    exact: bool,
    hi: f64,
    lo: f64,
}

impl Values {
    fn marks(&mut self, length: f64, spacing: f64, small: f64) -> Vec<Mark> {
        let count = 2.0_f64.max(10.0_f64.min((length / spacing).floor()));
        let step = tick_step((self.hi - self.lo).abs(), count);
        if !self.exact {
            self.lo = tick_value(step, (tick_index(step, self.lo) + 1e-9).floor());
            self.hi = tick_value(step, (tick_index(step, self.hi) - 1e-9).ceil());
        }
        let (lo, hi) = (self.lo, self.hi);
        ticks_between(min(lo, hi), max(lo, hi), step)
            .into_iter()
            .map(|value| Mark {
                at: (value - lo) / (hi - lo),
                label: text(Some(&number_text(value)), small, 4000.0),
            })
            .collect()
    }
}

/// Draws an XY chart at its configured size, or narrower to fit the host's width.
///
/// # Errors
///
/// Returns a semantic error for a chart without a series.
#[expect(
    clippy::too_many_lines,
    clippy::many_single_char_names,
    reason = "mirrors Pele's renderXyChart"
)]
pub(crate) fn render(
    model: &XyChartDb,
    config: &Value,
    options: &RenderOptions,
) -> Result<Rendered, Error> {
    if model.plots.is_empty() {
        let mut error = Error::new(
            Code::Semantic,
            "No Plot to render, please provide a plot with some data",
        );
        error.r#type = Some("xychart");
        return Err(error);
    }
    let chart = record(property(config, "xyChart"));
    let setting = |key: &str| chart.and_then(|chart| property(chart, key));
    let size = options.font_size.unwrap_or(16.0);
    let small = js_round(size * 0.875);
    let tiny = js_round(size * 0.75);
    let pad = options.padding.unwrap_or(8.0);
    let natural = dimension(setting("width"), 700.0);
    let width = cap_width(options, natural, 240.0, 0.0);
    // A narrower chart is not as much shorter, so that the plot keeps room for its marks.
    let height = js_round(dimension(setting("height"), 500.0) * max(0.6, width / natural));
    let horizontal = match model.horizontal {
        Some(horizontal) => horizontal,
        None => matches!(setting("chartOrientation"), Some(Value::String(o)) if o == "horizontal"),
    };
    // The value of a bar is always written past its end, which is what showDataLabelOutsideBar
    // asks for in Mermaid. Inside, whether it fits changes from bar to bar and with the width of
    // the chart.
    let show_values = on(chart, "showDataLabel", false);
    // Stacked bar series share one bar per category, with positive values above the baseline and
    // negative values below it. The value of each stack is its total.
    let stacked = on(chart, "stacked", false);
    let x = &model.x_axis;
    let plots = &model.plots;
    let count_slots = match x {
        XAxis::Band { categories, .. } => categories.len().max(1),
        XAxis::Linear(_) => usize::MAX,
    };
    // Series named in rightAxis.series are measured against a second value axis, drawn on the
    // right of the plot, or along its top when the chart runs horizontal.
    let right_conf = record(setting("rightAxis"));
    let right_names: Vec<&str> = match right_conf.and_then(|conf| property(conf, "series")) {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| match item {
                Value::String(name) => Some(trim(name)),
                _ => None,
            })
            .collect(),
        Some(Value::String(name)) => vec![trim(name)],
        _ => Vec::new(),
    };
    let on_right: Vec<bool> = plots
        .iter()
        .map(|plot| !plot.title.is_empty() && right_names.contains(&plot.title.as_str()))
        .collect();
    let dual = on_right.contains(&true);
    let side_of = |index: usize| on_right.get(index).copied().unwrap_or(false);
    let has_bars = |side: bool| {
        plots
            .iter()
            .enumerate()
            .any(|(index, plot)| plot.bar && side_of(index) == side)
    };
    let totals = if stacked {
        stack_totals(model, &on_right, count_slots)
    } else {
        BTreeMap::new()
    };
    let line_height = text(Some("0"), small, 4000.0).height;
    let tiny_height = text(Some("0"), tiny, 4000.0).height;

    let title = layout_label(
        if on(chart, "showTitle", true) {
            model.title.as_deref()
        } else {
            None
        },
        false,
        size,
        width - 2.0 * pad,
        BOLD,
        None,
    );
    let title_space = if title.height > 0.0 {
        title.height + 12.0
    } else {
        0.0
    };

    let mut legend: Vec<(usize, Label)> = Vec::new();
    let mut legend_width = 0.0;
    if on(chart, "showLegend", true) {
        for (index, plot) in plots.iter().enumerate() {
            if plot.title.is_empty() {
                continue;
            }
            let label = text(Some(&plot.title), small, width * 0.3);
            legend_width = max(legend_width, label.width + SWATCH + 8.0);
            legend.push((index, label));
        }
    }
    let legend_space = if legend.is_empty() {
        0.0
    } else {
        legend_width + LEGEND_GAP
    };

    // The value range: as written, or taken from the data, where bars stand on zero and the
    // range grows to whole steps.
    let bars = plots.iter().filter(|plot| plot.bar).count();
    // Stacks of the two axes stand side by side.
    let slots_per_category = if stacked {
        usize::from(has_bars(false)) + usize::from(has_bars(true))
    } else {
        bars
    };
    #[expect(
        clippy::cast_precision_loss,
        reason = "series counts stay far below 2^52"
    )]
    let bars_f = slots_per_category as f64;
    let range = |side: bool, written: Option<(f64, f64)>| -> Values {
        if let Some((lo, hi)) = written {
            return Values {
                exact: true,
                hi: finite(Some(hi)).unwrap_or(f64::NAN),
                lo: finite(Some(lo)).unwrap_or(f64::NAN),
            };
        }
        let mut lo = f64::INFINITY;
        let mut hi = f64::NEG_INFINITY;
        for (index, plot) in plots.iter().enumerate() {
            if side_of(index) != side || (stacked && plot.bar) {
                continue;
            }
            for &(_, raw) in &plot.data {
                let Some(value) = finite(raw) else {
                    continue;
                };
                if value < lo {
                    lo = value;
                }
                if value > hi {
                    hi = value;
                }
            }
        }
        for (&(own, _), &(negative, positive)) in &totals {
            if own == side {
                lo = min(lo, negative);
                hi = max(hi, positive);
            }
        }
        if lo > hi {
            lo = 0.0;
            hi = 0.0;
        }
        if has_bars(side) {
            lo = min(lo, 0.0);
            hi = max(hi, 0.0);
        }
        #[expect(clippy::float_cmp, reason = "mirrors Pele's exact comparison")]
        if lo == hi {
            if lo == 0.0 {
                hi = 1.0;
            } else {
                lo -= lo.abs() / 2.0;
                hi += hi.abs() / 2.0;
            }
        }
        Values {
            exact: false,
            hi,
            lo,
        }
    };
    #[expect(clippy::float_cmp, reason = "mirrors Pele's exact comparison")]
    let written =
        |lo: f64, hi: f64| (lo.is_finite() && hi.is_finite() && lo != hi).then_some((lo, hi));
    let mut values = range(
        false,
        if model.has_set_y_axis {
            written(model.y_axis.min, model.y_axis.max)
        } else {
            None
        },
    );
    let right_number = |key: &str| match right_conf.and_then(|conf| property(conf, key)) {
        Some(Value::Number(value)) => *value,
        _ => f64::NAN,
    };
    let mut right_values = range(true, written(right_number("min"), right_number("max")));

    // Places along the category axis are fractions of its length. Categories and bars sit in the
    // middle of equal slots. The points of a line on a numeric axis run from end to end.
    let mut count = 1.0_f64;
    match x {
        XAxis::Band { categories, .. } => {
            #[expect(clippy::cast_precision_loss, reason = "counts stay far below 2^52")]
            let n = categories.len() as f64;
            count = max(n, 1.0);
        }
        XAxis::Linear(_) => {
            for plot in plots {
                #[expect(clippy::cast_precision_loss, reason = "counts stay far below 2^52")]
                let n = plot.data.len() as f64;
                count = max(count, n);
            }
        }
    }
    let band = matches!(x, XAxis::Band { .. });
    let slot = 1.0 / count;
    let inset = if band || bars > 0 { slot / 2.0 } else { 0.0 };
    let (x0, x1) = match x {
        XAxis::Linear(axis) => (finite(Some(axis.min)), finite(Some(axis.max))),
        XAxis::Band { .. } => (None, None),
    };
    #[expect(clippy::float_cmp, reason = "mirrors Pele's exact comparison")]
    let ranged = matches!((x0, x1), (Some(a), Some(b)) if a != b);
    let place = |i: usize, n: usize| {
        #[expect(clippy::cast_precision_loss, reason = "counts stay far below 2^52")]
        let (i, n) = (i as f64, n as f64);
        if band {
            (i + 0.5) * slot
        } else {
            let along = if n > 1.0 {
                i / (n - 1.0)
            } else if ranged {
                0.0
            } else {
                0.5
            };
            inset + along * (1.0 - 2.0 * inset)
        }
    };

    let category_conf = record(chart.and_then(|chart| property(chart, "xAxis")));
    let value_conf = record(chart.and_then(|chart| property(chart, "yAxis")));
    let (left_conf, bottom_conf) = if horizontal {
        (category_conf, value_conf)
    } else {
        (value_conf, category_conf)
    };
    let left_title = text(
        if on(left_conf, "showTitle", true) {
            Some(if horizontal {
                x.title()
            } else {
                model.y_axis.title.as_str()
            })
        } else {
            None
        },
        small,
        height * 0.8,
    );
    let bottom_title = text(
        if on(bottom_conf, "showTitle", true) {
            Some(if horizontal {
                model.y_axis.title.as_str()
            } else {
                x.title()
            })
        } else {
            None
        },
        small,
        width * 0.8,
    );
    let left_tick = if on(left_conf, "showTick", true) {
        TICK
    } else {
        0.0
    };
    let bottom_tick = if on(bottom_conf, "showTick", true) {
        TICK
    } else {
        0.0
    };
    let left_labels = on(left_conf, "showLabel", true);
    let bottom_labels = on(bottom_conf, "showLabel", true);

    let right_title = text(
        match right_conf.and_then(|conf| property(conf, "title")) {
            Some(Value::String(title)) if dual && on(right_conf, "showTitle", true) => {
                Some(trim(title))
            }
            _ => None,
        },
        small,
        if horizontal {
            width * 0.8
        } else {
            height * 0.8
        },
    );
    let right_tick = if dual && on(right_conf, "showTick", true) {
        TICK
    } else {
        0.0
    };
    let right_labels = dual && on(right_conf, "showLabel", true);
    let above = if dual && horizontal {
        right_tick
            + if right_labels { line_height + 4.0 } else { 0.0 }
            + if right_title.height > 0.0 {
                right_title.height + 4.0
            } else {
                0.0
            }
    } else {
        0.0
    };

    let wrap_at = if horizontal {
        width * 0.3
    } else {
        max(48.0, (width - 2.0 * pad - 64.0 - legend_space) * slot - 8.0)
    };
    let categories: Vec<Label> = match x {
        XAxis::Band { categories, .. } if on(category_conf, "showLabel", true) => categories
            .iter()
            .map(|category| text(Some(category), small, wrap_at))
            .collect(),
        _ => Vec::new(),
    };
    let mut category_width = 0.0;
    let mut category_height = 0.0;
    for label in &categories {
        category_width = max(category_width, label.width);
        category_height = max(category_height, label.height);
    }

    // The labelled places of the category axis. Labels that would run into each other are
    // thinned out.
    let category_marks = |length: f64| -> Vec<Mark> {
        let mut marks = Vec::new();
        match x {
            XAxis::Band {
                categories: names, ..
            } => {
                let need = if categories.is_empty() {
                    8.0
                } else if horizontal {
                    category_height
                } else {
                    category_width + 12.0
                };
                let every = max(1.0, (need / (length * slot)).ceil());
                // A step too large to count in is as good as one past the end.
                #[expect(
                    clippy::cast_possible_truncation,
                    clippy::cast_sign_loss,
                    reason = "a positive whole number, saturated"
                )]
                let every = (every as usize).max(1);
                let mut i = 0;
                while i < names.len() {
                    #[expect(clippy::cast_precision_loss, reason = "counts stay far below 2^52")]
                    let at = (i as f64 + 0.5) * slot;
                    marks.push(Mark {
                        at,
                        label: categories
                            .get(i)
                            .cloned()
                            .unwrap_or_else(|| text(None, small, 0.0)),
                    });
                    i = i.saturating_add(every);
                }
            }
            XAxis::Linear(_) => {
                if let (true, Some(x0), Some(x1)) = (ranged, x0, x1) {
                    let count = 2.0_f64
                        .max(10.0_f64.min((length / if horizontal { 32.0 } else { 80.0 }).floor()));
                    let step = tick_step((x1 - x0).abs(), count);
                    for value in ticks_between(min(x0, x1), max(x0, x1), step) {
                        marks.push(Mark {
                            at: inset + ((value - x0) / (x1 - x0)) * (1.0 - 2.0 * inset),
                            label: text(Some(&number_text(value)), small, 4000.0),
                        });
                    }
                } else if let Some(x0) = x0 {
                    marks.push(Mark {
                        at: 0.5,
                        label: text(Some(&number_text(x0)), small, 4000.0),
                    });
                }
            }
        }
        marks
    };

    // Room past the end of the value axis for what is written beside a point or a bar.
    let mut point_label_width = 0.0;
    let mut value_width = 0.0;
    for plot in plots {
        for label in plot.point_labels.iter().flatten() {
            point_label_width = max(point_label_width, measure::width(label, tiny, 0));
        }
        if show_values && horizontal && plot.bar && !stacked {
            for &(_, value) in &plot.data {
                let written = finite(value).map(number_text).unwrap_or_default();
                value_width = max(value_width, measure::width(&written, tiny, 0));
            }
        }
    }

    if show_values && horizontal {
        for &(negative, positive) in totals.values() {
            for total in [negative, positive] {
                value_width = max(
                    value_width,
                    measure::width(&number_text(tidy(total)), tiny, 0),
                );
            }
        }
    }

    let bottom_label_height = if !bottom_labels {
        0.0
    } else if horizontal || !band {
        line_height
    } else {
        category_height
    };
    let bottom_space = bottom_tick
        + if bottom_label_height > 0.0 {
            bottom_label_height + 4.0
        } else {
            0.0
        }
        + if bottom_title.height > 0.0 {
            bottom_title.height + 4.0
        } else {
            0.0
        };
    let headroom = if horizontal {
        0.0
    } else if point_label_width > 0.0 || show_values {
        tiny_height + 4.0
    } else if left_labels {
        line_height / 2.0
    } else {
        0.0
    };
    let top = pad + title_space + headroom + above;
    let plot_height = max(1.0, height - pad - bottom_space - top);
    let bottom = top + plot_height;

    let left_marks = if horizontal {
        category_marks(plot_height)
    } else {
        values.marks(plot_height, 56.0, small)
    };
    let left_label_width = if left_labels {
        widest(&left_marks)
    } else {
        0.0
    };
    let left =
        pad + if left_title.height > 0.0 {
            left_title.height + 6.0
        } else {
            0.0
        } + if left_label_width > 0.0 {
            left_label_width + 6.0
        } else {
            0.0
        } + left_tick;
    let beside = max(
        if point_label_width > 0.0 {
            point_label_width + 12.0
        } else {
            0.0
        },
        if value_width > 0.0 {
            value_width + 8.0
        } else {
            0.0
        },
    );
    let reach = if horizontal {
        min(width * 0.25, beside)
    } else {
        0.0
    };
    let right_marks = if dual && !horizontal {
        right_values.marks(plot_height, 56.0, small)
    } else {
        Vec::new()
    };
    let right_label_width = if right_labels {
        widest(&right_marks)
    } else {
        0.0
    };
    let beyond = if dual && !horizontal {
        right_tick
            + if right_label_width > 0.0 {
                right_label_width + 6.0
            } else {
                0.0
            }
            + if right_title.height > 0.0 {
                right_title.height + 6.0
            } else {
                0.0
            }
    } else {
        0.0
    };
    let mut plot_width = max(1.0, width - pad - left - legend_space - reach - beyond);
    let bottom_marks = if horizontal {
        values.marks(plot_width, 80.0, small)
    } else {
        category_marks(plot_width)
    };
    if bottom_labels && legend_space + reach + beyond == 0.0 {
        let mut over = 0.0;
        for mark in &bottom_marks {
            over = max(
                over,
                mark.at * plot_width + mark.label.width / 2.0 - plot_width,
            );
        }
        plot_width = max(1.0, plot_width - over);
    }
    let right = left + plot_width;
    let right_marks = if dual && horizontal {
        right_values.marks(plot_width, 80.0, small)
    } else {
        right_marks
    };
    let (lo, hi) = (values.lo, values.hi);
    let (right_lo, right_hi) = (right_values.lo, right_values.hi);

    let px = |c: f64, v: f64| {
        if horizontal {
            left + v * plot_width
        } else {
            left + c * plot_width
        }
    };
    let py = |c: f64, v: f64| {
        if horizontal {
            top + c * plot_height
        } else {
            bottom - v * plot_height
        }
    };
    let scale = |value: f64| (value - lo) / (hi - lo);
    let scale_of = |side: bool, value: f64| {
        if side {
            (value - right_lo) / (right_hi - right_lo)
        } else {
            scale(value)
        }
    };
    let clamp = |v: f64| min(max(v, 0.0), 1.0);

    // A line across the plot at a place on the value axis.
    let rule = |out: &mut String, v: f64| {
        out.push('M');
        if horizontal {
            push_num(out, px(0.0, v));
            out.push(',');
            push_num(out, top);
            out.push('V');
            push_num(out, bottom);
        } else {
            push_num(out, left);
            out.push(',');
            push_num(out, py(0.0, v));
            out.push('H');
            push_num(out, right);
        }
    };

    let mut grid = String::new();
    for mark in if horizontal {
        &bottom_marks
    } else {
        &left_marks
    } {
        rule(&mut grid, mark.at);
    }

    let mut axis = String::new();
    let mut labels = String::new();
    if on(left_conf, "showAxisLine", true) {
        axis.push('M');
        push_num(&mut axis, left);
        axis.push(',');
        push_num(&mut axis, top);
        axis.push('V');
        push_num(&mut axis, bottom);
    }
    if on(bottom_conf, "showAxisLine", true) {
        axis.push('M');
        push_num(&mut axis, left);
        axis.push(',');
        push_num(&mut axis, bottom);
        axis.push('H');
        push_num(&mut axis, right);
    }
    for mark in &left_marks {
        let y = if horizontal {
            top + mark.at * plot_height
        } else {
            bottom - mark.at * plot_height
        };
        if left_tick != 0.0 {
            axis.push('M');
            push_num(&mut axis, left);
            axis.push(',');
            push_num(&mut axis, y);
            axis.push('h');
            push_number(&mut axis, -left_tick);
        }
        if left_labels {
            labels.push_str(&aligned(
                &mark.label,
                left - left_tick - 6.0,
                y,
                Anchor::End,
            ));
        }
    }
    for mark in &bottom_marks {
        let at = left + mark.at * plot_width;
        if bottom_tick != 0.0 {
            axis.push('M');
            push_num(&mut axis, at);
            axis.push(',');
            push_num(&mut axis, bottom);
            axis.push('v');
            push_number(&mut axis, bottom_tick);
        }
        if bottom_labels {
            labels.push_str(&aligned(
                &mark.label,
                at,
                bottom + bottom_tick + 4.0 + mark.label.height / 2.0,
                Anchor::Middle,
            ));
        }
    }

    if dual && on(right_conf, "showAxisLine", true) {
        axis.push('M');
        if horizontal {
            push_num(&mut axis, left);
            axis.push(',');
            push_num(&mut axis, top);
            axis.push('H');
            push_num(&mut axis, right);
        } else {
            push_num(&mut axis, right);
            axis.push(',');
            push_num(&mut axis, top);
            axis.push('V');
            push_num(&mut axis, bottom);
        }
    }
    for mark in &right_marks {
        let (at, label) = if horizontal {
            let at = left + mark.at * plot_width;
            (
                at,
                aligned(
                    &mark.label,
                    at,
                    top - right_tick - 4.0 - mark.label.height / 2.0,
                    Anchor::Middle,
                ),
            )
        } else {
            let at = bottom - mark.at * plot_height;
            (
                at,
                aligned(&mark.label, right + right_tick + 6.0, at, Anchor::Start),
            )
        };
        if right_tick != 0.0 {
            axis.push('M');
            if horizontal {
                push_num(&mut axis, at);
                axis.push(',');
                push_num(&mut axis, top);
                axis.push('v');
                push_number(&mut axis, -right_tick);
            } else {
                push_num(&mut axis, right);
                axis.push(',');
                push_num(&mut axis, at);
                axis.push('h');
                push_number(&mut axis, right_tick);
            }
        }
        if right_labels {
            labels.push_str(&label);
        }
    }

    let span = (if horizontal { plot_height } else { plot_width }) * slot;
    let group = span * 0.7;
    let thickness = group / max(bars_f, 1.0);
    let gap = if slots_per_category > 1 {
        min(2.0, thickness * 0.2)
    } else {
        0.0
    };
    let base = clamp(scale(0.0));
    let right_base = clamp(scale_of(true, 0.0));
    // Bars that go both ways get a line to stand on.
    if has_bars(false) && base > 0.0 && base < 1.0 {
        rule(&mut axis, base);
    }
    if has_bars(true) && right_base > 0.0 && right_base < 1.0 && (right_base - base).abs() > 1e-9 {
        rule(&mut axis, right_base);
    }
    // Lines are drawn over the bars wherever they were declared, as a bar would hide part of one
    // under it. A series keeps the color of its place in the source.
    let mut series = String::new();
    let mut lines = String::new();
    let mut outside = String::new();
    let mut point_labels = String::new();
    let mut bar = 0.0;
    // How far each stack reaches below and above the baseline, and where it stands on the
    // category axis.
    let mut reached: BTreeMap<(bool, Stack), (f64, f64)> = BTreeMap::new();
    let mut stands: BTreeMap<(bool, Stack), f64> = BTreeMap::new();
    for (index, plot) in plots.iter().enumerate() {
        let color = chart_color(index, plots.len(), stacked || dual);
        let mut id = String::new();
        if !plot.title.is_empty() {
            id.push_str(" data-id=\"");
            id.push_str(&esc_text(&plot.title));
            id.push('"');
        }
        // A series declared before the categories can hold more values than there are categories.
        let n = plot.data.len().min(count_slots);
        let side = side_of(index);
        if plot.bar {
            let at = if stacked {
                if side && has_bars(false) { 1.0 } else { 0.0 }
            } else {
                bar += 1.0;
                bar - 1.0
            };
            let offset = (at - (bars_f - 1.0) / 2.0) * thickness - (thickness - gap) / 2.0;
            let base = if side { right_base } else { base };
            let mut rects = String::new();
            for i in 0..n {
                let Some(value) = finite(plot.data.get(i).and_then(|d| d.1)) else {
                    continue;
                };
                let c = place(i, n);
                // A stacked bar starts where the bars before it in its stack end.
                let start = if stacked {
                    let key = (
                        side,
                        Stack::of(x, i, plot.data.get(i).map_or("", |d| d.0.as_str())),
                    );
                    let along = if horizontal { py(c, 0.0) } else { px(c, 0.0) };
                    stands.insert(key.clone(), along + offset + (thickness - gap) / 2.0);
                    if value == 0.0 {
                        continue;
                    }
                    let reach = reached.entry(key).or_insert((0.0, 0.0));
                    let side = if value < 0.0 {
                        &mut reach.0
                    } else {
                        &mut reach.1
                    };
                    let start = *side;
                    *side = (start + value).clamp(-LIMIT, LIMIT);
                    Some((start, *side))
                } else {
                    None
                };
                let (a, v) = match start {
                    Some((start, end)) => {
                        (clamp(scale_of(side, start)), clamp(scale_of(side, end)))
                    }
                    None => (base, clamp(scale_of(side, value))),
                };
                let from = min(a, v);
                let to = max(a, v);
                let bx = if horizontal {
                    px(c, from)
                } else {
                    px(c, 0.0) + offset
                };
                let by = if horizontal {
                    py(c, 0.0) + offset
                } else {
                    py(c, to)
                };
                let bw = if horizontal {
                    (to - from) * plot_width
                } else {
                    thickness - gap
                };
                let bh = if horizontal {
                    thickness - gap
                } else {
                    (to - from) * plot_height
                };
                // A bar stacked on another leaves a line of the background between them.
                let (bx, by, bw, bh) = match start {
                    Some((start, _)) if start != 0.0 => {
                        let cut = 1.0_f64;
                        match (horizontal, value < 0.0) {
                            (false, false) => (bx, by, bw, max(0.0, bh - cut)),
                            (false, true) => (bx, by + cut.min(bh), bw, max(0.0, bh - cut)),
                            (true, false) => (bx + cut.min(bw), by, max(0.0, bw - cut), bh),
                            (true, true) => (bx, by, max(0.0, bw - cut), bh),
                        }
                    }
                    _ => (bx, by, bw, bh),
                };
                rects.push_str("<rect x=\"");
                push_num(&mut rects, bx);
                rects.push_str("\" y=\"");
                push_num(&mut rects, by);
                rects.push_str("\" width=\"");
                push_num(&mut rects, bw);
                rects.push_str("\" height=\"");
                push_num(&mut rects, bh);
                rects.push_str("\"/>");
                if !show_values || stacked {
                    continue;
                }
                let label = number_text(value);
                let w = measure::width(&label, tiny, 0);
                // Towards the end of the bar: 1 when it runs up or right from the baseline, -1
                // the other way.
                let out = if v >= base { 1.0 } else { -1.0 };
                let mut tx = bx + bw / 2.0;
                let mut ty = by + bh / 2.0;
                let mut anchor = "middle";
                if horizontal {
                    if bh < tiny {
                        continue;
                    }
                    tx = (if out > 0.0 { bx + bw } else { bx }) + out * 6.0;
                    anchor = if out > 0.0 { "start" } else { "end" };
                } else {
                    if w + 2.0 > span / bars_f {
                        continue;
                    }
                    ty = (if out > 0.0 { by } else { by + bh }) - out * (tiny_height / 2.0 + 2.0);
                }
                outside.push_str("<text x=\"");
                push_num(&mut outside, tx);
                outside.push_str("\" y=\"");
                push_num(&mut outside, ty + tiny * 0.35);
                outside.push_str("\" text-anchor=\"");
                outside.push_str(anchor);
                outside.push_str("\">");
                outside.push_str(&label);
                outside.push_str("</text>");
            }
            series.push_str("<g class=\"ondine-series ondine-bars\"");
            series.push_str(&id);
            series.push_str(" fill=\"");
            series.push_str(&color);
            series.push_str("\">");
            series.push_str(&rects);
            series.push_str("</g>");
            continue;
        }

        // A line is cut where it leaves the value range, so it stays inside the plot.
        let mut d = String::new();
        let mut dots = String::new();
        let mut pen = false;
        let mut pc = 0.0;
        let mut pv = f64::NAN;
        let point = |out: &mut String, letter: char, c: f64, v: f64| {
            out.push(letter);
            push_num(out, px(c, v));
            out.push(',');
            push_num(out, py(c, v));
        };
        for i in 0..n {
            let Some(value) = finite(plot.data.get(i).and_then(|d| d.1)) else {
                pv = f64::NAN;
                continue;
            };
            let c = place(i, n);
            let v = scale_of(side, value);
            let visible = (0.0..=1.0).contains(&v);
            if pv.is_nan() {
                if visible {
                    point(&mut d, 'M', c, v);
                }
                pen = visible;
            } else {
                let dv = v - pv;
                let mut t0 = 0.0;
                let mut t1 = 1.0;
                if dv != 0.0 {
                    let a = -pv / dv;
                    let b = (1.0 - pv) / dv;
                    t0 = max(t0, min(a, b));
                    t1 = min(t1, max(a, b));
                } else if !visible {
                    t0 = 2.0;
                }
                if t0 <= t1 {
                    let c0 = pc + (c - pc) * t0;
                    let v0 = pv + dv * t0;
                    let c1 = pc + (c - pc) * t1;
                    let v1 = pv + dv * t1;
                    if !pen || t0 > 0.0 {
                        point(&mut d, 'M', c0, v0);
                    }
                    point(&mut d, 'L', c1, v1);
                }
                pen = visible;
            }
            pc = c;
            pv = v;
            if !visible {
                continue;
            }
            let cx = px(c, v);
            let cy = py(c, v);
            if span >= 12.0 {
                dots.push_str("<circle cx=\"");
                push_num(&mut dots, cx);
                dots.push_str("\" cy=\"");
                push_num(&mut dots, cy);
                dots.push_str("\" r=\"3\"/>");
            }
            let label = text(
                plot.point_labels
                    .as_ref()
                    .and_then(|labels| labels.get(i))
                    .map(String::as_str),
                tiny,
                4000.0,
            );
            if label.height > 0.0 {
                let half = label.width / 2.0;
                point_labels.push_str(&if horizontal {
                    aligned(&label, cx + 10.0, cy, Anchor::Start)
                } else {
                    aligned(
                        &label,
                        max(min(cx, right - half), left + half),
                        cy - 6.0 - label.height / 2.0,
                        Anchor::Middle,
                    )
                });
            }
        }
        lines.push_str("<g class=\"ondine-series ondine-line\"");
        lines.push_str(&id);
        lines.push_str(" fill=\"");
        lines.push_str(&color);
        lines.push_str("\">");
        if !d.is_empty() {
            lines.push_str("<path d=\"");
            lines.push_str(&d);
            lines.push_str("\" fill=\"none\" stroke=\"");
            lines.push_str(&color);
            lines.push_str(
                "\" stroke-width=\"2\" stroke-linecap=\"round\" stroke-linejoin=\"round\"/>",
            );
        }
        lines.push_str(&dots);
        lines.push_str("</g>");
    }

    // Each stack is labelled with its total past its end, and a stack that goes both ways with
    // both totals.
    if show_values {
        for (key, &(negative, positive)) in &totals {
            let Some(&along) = stands.get(key) else {
                continue;
            };
            // A stack of zeros is labelled 0 above the baseline.
            let sides = [
                (negative, negative < 0.0),
                (positive, positive > 0.0 || negative == 0.0),
            ];
            for (total, shown) in sides {
                if !shown {
                    continue;
                }
                let label = number_text(tidy(total));
                let v = clamp(scale_of(key.0, total));
                let out = if total < 0.0 { -1.0 } else { 1.0 };
                let (tx, ty, anchor) = if horizontal {
                    if thickness < tiny {
                        continue;
                    }
                    (
                        px(0.0, v) + out * 6.0,
                        along,
                        if out > 0.0 { "start" } else { "end" },
                    )
                } else {
                    if measure::width(&label, tiny, 0) + 2.0 > span / bars_f {
                        continue;
                    }
                    (
                        along,
                        py(0.0, v) - out * (tiny_height / 2.0 + 2.0),
                        "middle",
                    )
                };
                outside.push_str("<text x=\"");
                push_num(&mut outside, tx);
                outside.push_str("\" y=\"");
                push_num(&mut outside, ty + tiny * 0.35);
                outside.push_str("\" text-anchor=\"");
                outside.push_str(anchor);
                outside.push_str("\">");
                outside.push_str(&label);
                outside.push_str("</text>");
            }
        }
    }

    let mut titles = String::new();
    if left_title.height > 0.0 {
        titles.push_str("<g transform=\"translate(");
        push_num(&mut titles, pad + left_title.height / 2.0);
        titles.push(',');
        push_num(&mut titles, top + plot_height / 2.0);
        titles.push_str(") rotate(-90)\">");
        titles.push_str(&label_svg(&left_title, 0.0, 0.0, "", None));
        titles.push_str("</g>");
    }
    if right_title.height > 0.0 {
        if horizontal {
            titles.push_str(&label_svg(
                &right_title,
                left + plot_width / 2.0,
                top - above + right_title.height / 2.0,
                "",
                None,
            ));
        } else {
            titles.push_str("<g transform=\"translate(");
            push_num(&mut titles, right + beyond - right_title.height / 2.0);
            titles.push(',');
            push_num(&mut titles, top + plot_height / 2.0);
            titles.push_str(") rotate(90)\">");
            titles.push_str(&label_svg(&right_title, 0.0, 0.0, "", None));
            titles.push_str("</g>");
        }
    }
    titles.push_str(&label_svg(
        &bottom_title,
        left + plot_width / 2.0,
        height - pad - bottom_title.height / 2.0,
        "",
        None,
    ));

    let mut legend_out = String::new();
    let mut legend_height = 0.0;
    for (_, label) in &legend {
        legend_height += max(ROW, label.height + 6.0);
    }
    let mut ly = top + max(0.0, (plot_height - legend_height) / 2.0);
    let lx = right + reach + beyond + LEGEND_GAP;
    for (index, label) in &legend {
        let row = max(ROW, label.height + 6.0);
        if ly + row > height {
            break;
        }
        let y = ly + row / 2.0;
        let color = chart_color(*index, plots.len(), stacked || dual);
        let Some(plot) = plots.get(*index) else {
            continue;
        };
        legend_out.push_str("<g class=\"ondine-legend-item\" data-id=\"");
        legend_out.push_str(&esc_text(&plot.title));
        legend_out.push_str("\">");
        if plot.bar {
            legend_out.push_str("<rect x=\"");
            push_num(&mut legend_out, lx);
            legend_out.push_str("\" y=\"");
            push_num(&mut legend_out, y - SWATCH / 2.0);
            legend_out.push_str("\" width=\"12\" height=\"12\" rx=\"2\" fill=\"");
            legend_out.push_str(&color);
            legend_out.push_str("\"/>");
        } else {
            legend_out.push_str("<path d=\"M");
            push_num(&mut legend_out, lx);
            legend_out.push(',');
            push_num(&mut legend_out, y);
            legend_out.push_str("h12\" stroke=\"");
            legend_out.push_str(&color);
            legend_out.push_str("\" stroke-width=\"2\" stroke-linecap=\"round\"/>");
        }
        legend_out.push_str(&label_svg(
            label,
            lx + SWATCH + 8.0 + label.width / 2.0,
            y,
            "",
            None,
        ));
        legend_out.push_str("</g>");
        ly += row;
    }

    let mut content = label_svg(
        &title,
        width / 2.0,
        pad + title.height / 2.0,
        " class=\"ondine-title\" font-weight=\"var(--_tw)\"",
        None,
    );
    if !grid.is_empty() {
        content.push_str("<path class=\"ondine-grid\" d=\"");
        content.push_str(&grid);
        content.push_str("\" stroke=\"var(--_a)\"/>");
    }
    content.push_str("<g class=\"ondine-plot\">");
    content.push_str(&series);
    content.push_str(&lines);
    content.push_str("</g>");
    if !axis.is_empty() {
        content.push_str("<path class=\"ondine-axis\" d=\"");
        content.push_str(&axis);
        content.push_str("\" fill=\"none\" stroke=\"var(--_b)\"/>");
    }
    let group = |content: &mut String, class: &str, font: f64, extra: &str, body: &str| {
        if body.is_empty() {
            return;
        }
        content.push_str("<g class=\"");
        content.push_str(class);
        content.push_str("\" font-size=\"");
        push_number(content, font);
        content.push('"');
        content.push_str(extra);
        content.push('>');
        content.push_str(body);
        content.push_str("</g>");
    };
    group(
        &mut content,
        "ondine-axis-labels",
        small,
        " fill=\"var(--_m)\"",
        &labels,
    );
    group(&mut content, "ondine-axis-titles", small, "", &titles);
    group(&mut content, "ondine-data-labels", tiny, "", &outside);
    group(&mut content, "ondine-point-labels", tiny, "", &point_labels);
    group(&mut content, "ondine-legend", small, "", &legend_out);

    Ok(Rendered {
        height,
        links: Vec::new(),
        svg: svg_document(
            "xychart",
            width,
            height,
            size,
            options,
            Accessible {
                acc_descr: model.acc_descr.as_deref(),
                acc_title: model.acc_title.as_deref(),
            },
            &content,
        ),
        width,
    })
}
