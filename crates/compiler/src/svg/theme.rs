//! Theme variables, author styles, and class names. Port of Pele's `src/svg/theme.ts`.
//!
//! Ondine's deliberate difference: public `--ondine-*` variables replace Pele's `--pele-*` ones,
//! with monochrome defaults, square corners, a monospace font, and a `--ondine-stroke` width.

use std::borrow::Cow;

use crate::svg::builder::esc;
use crate::text::label::trim;
use crate::util::chars::is_space;
use crate::yaml::push_number;

/// Private aliases set once on the root element, so each use is short and the public `--ondine-*`
/// variables keep their defaults. Node fills default to the background, and lines, borders,
/// and muted text to the foreground, so setting `--ondine-bg` and `--ondine-fg` themes a diagram.
/// The alternate surface is a light tint of the foreground, as diagrams draw grids, graticules,
/// excluded ranges, and finished work in it, which would vanish into the background. The text color is set as
/// well: an icon is drawn in `currentColor`, which would otherwise be whatever color the page
/// gives the diagram's container. Shapes and edges inherit the stroke width, which defaults to the
/// 2 pixels that the diagrams' fixed-width outlines already use.
pub(crate) const ROOT_STYLE: &str = "--_bg:var(--ondine-bg,#fff);--_fg:var(--ondine-fg,#000);\
--_m:var(--ondine-muted,var(--_fg));--_l:var(--ondine-line,var(--_fg));--_s:var(--ondine-surface,var(--_bg));\
--_a:var(--ondine-surface-alt,color-mix(in srgb,var(--_fg) 20%,var(--_bg)));--_b:var(--ondine-border,var(--_fg));--_c:var(--ondine-accent,var(--_fg));\
--_sw:var(--ondine-stroke,2px);stroke-width:var(--_sw);color:var(--_fg)";

/// The monospace font, as set on the root.
pub(crate) const FONT_MONO: &str = "var(--_fm)";
/// Expands to the corner radius of rectangles, for constants built from it.
macro_rules! radius {
    () => {
        "var(--ondine-radius,0)"
    };
}

/// The corner radius of rectangles.
pub(crate) const RADIUS: &str = radius!();
/// The attribute that gives a rectangle the themed corner radius. The root document moves it
/// into the rectangle's style.
pub(crate) const ROUNDED: &str = concat!(" rx=\"", radius!(), "\"");
/// The color of what a diagram marks as critical. It is not one of the series colors, as a theme
/// may put those in any order.
pub(crate) const CRITICAL: &str = "var(--ondine-critical,var(--_fg))";
/// The stroke width of shapes and edges, as set on the root.
pub(crate) const STROKE: &str = "var(--_sw)";

/// The font stack labels are drawn in when the page sets no `--ondine-font`.
const FONT: &str = "ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace";

/// The root's font declarations at `size` pixels. Ondine measures every font as monospace, so the
/// font comes only from `--ondine-font`, for both Pele's normal and monospace stacks. The rest of
/// what text inherits is pinned, as none of it was measured. Kerning and ligatures are off and
/// glyphs are placed at their exact size, so every character keeps the uniform advance that
/// layout assumed.
pub(crate) fn font_style(size: f64) -> String {
    let mut out = String::with_capacity(256);
    out.push_str(";--_fm:var(--ondine-font,");
    out.push_str(FONT);
    out.push_str(");font:");
    push_number(&mut out, size);
    out.push_str("px monospace;letter-spacing:normal;word-spacing:normal;text-transform:none;direction:ltr;font-family:var(--ondine-font,");
    out.push_str(FONT);
    out.push_str(
        ");font-kerning:none;font-variant-ligatures:none;text-rendering:geometricPrecision",
    );
    out
}

/// Properties of an author style that apply to a shape, in no particular order.
const SHAPE_PROPS: [&str; 12] = [
    "fill",
    "fill-opacity",
    "opacity",
    "rx",
    "ry",
    "stroke",
    "stroke-dasharray",
    "stroke-dashoffset",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-opacity",
    "stroke-width",
];

/// Properties of an author style that apply to text, in no particular order.
const TEXT_PROPS: [&str; 9] = [
    "color",
    "font-family",
    "font-size",
    "font-style",
    "font-weight",
    "letter-spacing",
    "opacity",
    "text-decoration",
    "word-spacing",
];

/// Author styles split by what they apply to, as attributes ready to write.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ResolvedStyle {
    /// Whether the text is bold.
    pub(crate) bold: bool,
    /// A font size in pixels, when one in range was given.
    pub(crate) font_size: Option<f64>,
    /// Whether the text is italic.
    pub(crate) italic: bool,
    /// A `style` attribute for lines: the shape's declarations without fills.
    pub(crate) line: String,
    /// A `style` attribute for the shape.
    pub(crate) shape: String,
    /// The stroke color, when one was given.
    pub(crate) stroke: Option<String>,
    /// A `style` attribute for the text.
    pub(crate) text: String,
}

/// Whether `text` at `at` starts with the ASCII `word`, ignoring case.
fn starts_with_ci(text: &[u8], at: usize, word: &str) -> bool {
    text.get(at..at + word.len())
        .is_some_and(|head| head.eq_ignore_ascii_case(word.as_bytes()))
}

/// Whether `word`, then optional whitespace, then `(` starts at `at`.
fn call_at(text: &str, at: usize, word: &str) -> bool {
    starts_with_ci(text.as_bytes(), at, word)
        && text.get(at + word.len()..).is_some_and(|rest| {
            rest.trim_start_matches(|c: char| is_space(u32::from(c)))
                .starts_with('(')
        })
}

/// Pele's `/url\s*\(|expression\s*\(|javascript:|[<>{};\\]|@import|image-set|\/\*/i`.
pub(crate) fn unsafe_value(value: &str) -> bool {
    let bytes = value.as_bytes();
    (0..bytes.len()).any(|at| {
        matches!(
            bytes.get(at),
            Some(b'<' | b'>' | b'{' | b'}' | b';' | b'\\')
        ) || call_at(value, at, "url")
            || call_at(value, at, "expression")
            || starts_with_ci(bytes, at, "javascript:")
            || starts_with_ci(bytes, at, "@import")
            || starts_with_ci(bytes, at, "image-set")
            || starts_with_ci(bytes, at, "/*")
    })
}

/// `text` without any `!important`, in any case.
fn strip_important(text: &str) -> Cow<'_, str> {
    const WORD: &str = "!important";
    let bytes = text.as_bytes();
    if !(0..bytes.len()).any(|at| starts_with_ci(bytes, at, WORD)) {
        return Cow::Borrowed(text);
    }

    let mut out = String::with_capacity(text.len());
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        if starts_with_ci(bytes, at, WORD) {
            out.push_str(text.get(start..at).unwrap_or(""));
            at += WORD.len();
            start = at;
        } else {
            at += 1;
        }
    }
    out.push_str(text.get(start..).unwrap_or(""));
    Cow::Owned(out)
}

/// Sets `key` in an insertion-ordered map, keeping the position of an existing key as a
/// JavaScript `Map` does.
fn set(map: &mut Vec<(String, String)>, key: &str, value: &str) {
    if let Some(entry) = map.iter_mut().find(|(k, _)| k == key) {
        value.clone_into(&mut entry.1);
    } else {
        map.push((key.to_owned(), value.to_owned()));
    }
}

/// JavaScript's `parseFloat` over text of digits and dots: the longest leading decimal number.
fn parse_float(text: &str) -> Option<f64> {
    let bytes = text.as_bytes();
    let int = bytes.iter().take_while(|c| c.is_ascii_digit()).count();
    let mut end = int;
    let mut digits = int;
    if bytes.get(end) == Some(&b'.') {
        let frac = bytes
            .get(end + 1..)
            .unwrap_or(&[])
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .count();
        digits += frac;
        if frac > 0 {
            end += 1 + frac;
        }
    }
    if digits == 0 {
        return None;
    }
    text.get(..end).and_then(crate::util::float::parse)
}

/// JavaScript's `Number(text)`, for the forms a font weight can take.
fn js_number(text: &str) -> f64 {
    let text = trim(text);
    if text.is_empty() {
        return 0.0;
    }

    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(digits) = text.strip_prefix(prefix) {
            return u64::from_str_radix(digits, radix)
                .ok()
                .filter(|_| !digits.starts_with(['+', '-']))
                .map_or(f64::NAN, |value| {
                    #[expect(
                        clippy::cast_precision_loss,
                        reason = "JavaScript numbers are doubles"
                    )]
                    let value = value as f64;
                    value
                });
        }
    }

    match text {
        "Infinity" | "+Infinity" => return f64::INFINITY,
        "-Infinity" => return f64::NEG_INFINITY,
        _ => {}
    }

    // Rust also reads `inf` and `nan`, which JavaScript does not, so only number characters pass.
    if !text
        .bytes()
        .all(|c| c.is_ascii_digit() || matches!(c, b'.' | b'e' | b'E' | b'+' | b'-'))
    {
        return f64::NAN;
    }
    crate::util::float::parse(text).unwrap_or(f64::NAN)
}

/// Lowercases as JavaScript's `toLowerCase` does, for property names.
fn lowercase(text: &str) -> String {
    text.chars().flat_map(char::to_lowercase).collect()
}

/// Splits Mermaid style declarations into what applies to a shape and what applies to its text.
pub(crate) fn resolve_style<S: AsRef<str>>(declarations: &[S]) -> ResolvedStyle {
    if declarations.is_empty() {
        return ResolvedStyle::default();
    }

    let mut shape: Vec<(String, String)> = Vec::new();
    let mut text: Vec<(String, String)> = Vec::new();
    for decl in declarations {
        let decl = decl.as_ref();
        let Some(at) = decl.find(':').filter(|&at| at > 0) else {
            continue;
        };
        let prop = lowercase(trim(decl.get(..at).unwrap_or("")));
        let value = strip_important(decl.get(at + 1..).unwrap_or(""));
        let value = trim(&value);
        if value.is_empty() || unsafe_value(value) {
            continue;
        }
        if SHAPE_PROPS.contains(&prop.as_str()) {
            set(&mut shape, &prop, value);
        }
        if TEXT_PROPS.contains(&prop.as_str()) {
            set(
                &mut text,
                if prop == "color" { "fill" } else { &prop },
                value,
            );
        }
    }
    if shape.is_empty() && text.is_empty() {
        return ResolvedStyle::default();
    }

    let mut shape_css = String::new();
    let mut line_css = String::new();
    for (prop, value) in &shape {
        push_declaration(&mut shape_css, prop, value);
        if prop != "fill" && prop != "fill-opacity" {
            push_declaration(&mut line_css, prop, value);
        }
    }
    let mut text_css = String::new();
    for (prop, value) in &text {
        push_declaration(&mut text_css, prop, value);
    }

    let get = |map: &[(String, String)], key: &str| {
        map.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone())
    };
    let weight = get(&text, "font-weight");
    let px = get(&text, "font-size").and_then(|size| {
        let digits = size.strip_suffix("px").unwrap_or(&size);
        if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit() || c == b'.') {
            return None;
        }
        parse_float(digits)
    });
    ResolvedStyle {
        bold: weight
            .as_deref()
            .is_some_and(|w| w == "bold" || w == "bolder" || js_number(w) >= 600.0),
        font_size: px.filter(|&px| px > 0.0 && px <= 200.0),
        italic: get(&text, "font-style").as_deref() == Some("italic"),
        line: style_attr(&line_css),
        shape: style_attr(&shape_css),
        stroke: get(&shape, "stroke"),
        text: style_attr(&text_css),
    }
}

/// Appends `prop:value;`.
fn push_declaration(out: &mut String, prop: &str, value: &str) {
    out.push_str(prop);
    out.push(':');
    out.push_str(value);
    out.push(';');
}

/// A `style` attribute holding `css`, or nothing when it is empty.
fn style_attr(css: &str) -> String {
    if css.is_empty() {
        return String::new();
    }
    let mut out = String::from(" style=\"");
    out.push_str(&esc(css));
    out.push('"');
    out
}

/// Author class names as a ` class` suffix: every run of characters outside `[\w-]` becomes one
/// space. Empty when nothing is left.
pub(crate) fn class_names(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len() + 1);
    let mut gap = true;
    for c in raw.chars() {
        if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
            if gap {
                out.push(' ');
            }
            out.push(c);
            gap = false;
        } else {
            gap = true;
        }
    }
    out
}

/// The share of the foreground in each series' default shade. Every shade is at least half the
/// foreground, so thin lines, outlines under a wash, and labels drawn in the background color stay
/// readable on it. Neighboring series differ by at least 15 points.
const SERIES_SHADES: [&str; 8] = ["100", "57", "86", "50", "79", "64", "93", "71"];

/// Color for the nth data series in a chart, cycling through eight variables. Each defaults to a
/// shade between the foreground and background, so charts stay monochrome and follow the scheme.
pub(crate) fn series_color(index: i64) -> String {
    let k = index.rem_euclid(8);
    let mut out = String::from("var(--ondine-series-");
    out.push(char::from(b'1' + u8::try_from(k).unwrap_or(0)));
    out.push_str(",color-mix(in srgb,var(--_fg) ");
    out.push_str(
        SERIES_SHADES
            .get(usize::try_from(k).unwrap_or(0))
            .copied()
            .unwrap_or("100"),
    );
    out.push_str("%,var(--_bg)))");
    out
}

#[cfg(test)]
mod tests {
    //! Expectations come from Pele's `resolveStyle`, `classNames`, and `seriesColor`, with
    //! `--pele-*` series colors replaced by Ondine's monochrome ones.

    use super::{class_names, resolve_style, series_color};

    #[test]
    #[expect(clippy::type_complexity, reason = "a table of cases")]
    fn resolve_style_matches_pele() {
        let cases: &[(
            &[&str],
            &str,
            &str,
            &str,
            Option<&str>,
            bool,
            bool,
            Option<f64>,
        )] = &[
            (&[][..], "", "", "", None, false, false, None),
            (
                &["fill:#f9f", "stroke:#333", "stroke-width:4px"][..],
                " style=\"fill:#f9f;stroke:#333;stroke-width:4px;\"",
                " style=\"stroke:#333;stroke-width:4px;\"",
                "",
                Some("#333"),
                false,
                false,
                None,
            ),
            (
                &[
                    "color:red",
                    "font-weight:bold",
                    "font-size:20px",
                    "font-style:italic",
                ][..],
                "",
                "",
                " style=\"fill:red;font-weight:bold;font-size:20px;font-style:italic;\"",
                None,
                true,
                true,
                Some(20.0),
            ),
            (
                &["FILL : blue !important", "fill:green", "opacity:0.5"][..],
                " style=\"fill:green;opacity:0.5;\"",
                " style=\"opacity:0.5;\"",
                " style=\"opacity:0.5;\"",
                None,
                false,
                false,
                None,
            ),
            (
                &[
                    "fill:url(#x)",
                    "stroke:expression (1)",
                    "color:javascript:x",
                    "fill:a;b",
                    "fill:@IMPORT",
                    "fill:image-set(x)",
                    "fill:/*c*/",
                    "stroke:URL  (y)",
                ][..],
                "",
                "",
                "",
                None,
                false,
                false,
                None,
            ),
            (
                &[
                    ":nope",
                    "noval:",
                    "fill:",
                    "unknown:1",
                    "font-weight:600",
                    "font-size:300px",
                ][..],
                "",
                "",
                " style=\"font-weight:600;font-size:300px;\"",
                None,
                true,
                false,
                None,
            ),
            (
                &["font-weight:0x258", "font-size:1.5.5px"][..],
                "",
                "",
                " style=\"font-weight:0x258;font-size:1.5.5px;\"",
                None,
                true,
                false,
                Some(1.5),
            ),
            (
                &["font-weight:599", "font-size:.px"][..],
                "",
                "",
                " style=\"font-weight:599;font-size:.px;\"",
                None,
                false,
                false,
                None,
            ),
            (
                &["font-weight: Infinity ", "font-size:12"][..],
                "",
                "",
                " style=\"font-weight:Infinity;font-size:12;\"",
                None,
                true,
                false,
                Some(12.0),
            ),
            (
                &["font-weight:bolder", "font-size:0"][..],
                "",
                "",
                " style=\"font-weight:bolder;font-size:0;\"",
                None,
                true,
                false,
                None,
            ),
            (
                &["font-weight:7e2", "font-size:12pt"][..],
                "",
                "",
                " style=\"font-weight:7e2;font-size:12pt;\"",
                None,
                true,
                false,
                None,
            ),
            (
                &[
                    "stroke-dasharray:5 5",
                    "rx:10",
                    "ry:5",
                    "stroke-linecap:round",
                    "letter-spacing:2px",
                    "text-decoration:underline",
                    "font-family:Arial",
                ][..],
                " style=\"stroke-dasharray:5 5;rx:10;ry:5;stroke-linecap:round;\"",
                " style=\"stroke-dasharray:5 5;rx:10;ry:5;stroke-linecap:round;\"",
                " style=\"letter-spacing:2px;text-decoration:underline;font-family:Arial;\"",
                None,
                false,
                false,
                None,
            ),
            (
                &["stro\u{212a}e:#000", "fill:\"q\"&<"][..],
                " style=\"stroke:#000;\"",
                " style=\"stroke:#000;\"",
                "",
                Some("#000"),
                false,
                false,
                None,
            ),
            (
                &["fill:a!IMPORTANT!important b"][..],
                " style=\"fill:a b;\"",
                "",
                "",
                None,
                false,
                false,
                None,
            ),
        ];
        for &(decls, shape, line, text, stroke, bold, italic, font_size) in cases {
            let resolved = resolve_style(decls);
            assert_eq!(resolved.shape, shape, "{decls:?}");
            assert_eq!(resolved.line, line, "{decls:?}");
            assert_eq!(resolved.text, text, "{decls:?}");
            assert_eq!(resolved.stroke.as_deref(), stroke, "{decls:?}");
            assert_eq!(resolved.bold, bold, "{decls:?}");
            assert_eq!(resolved.italic, italic, "{decls:?}");
            assert_eq!(resolved.font_size, font_size, "{decls:?}");
        }
    }

    #[test]
    fn class_names_match_pele() {
        for &(raw, expected) in &[
            ("", ""),
            ("a", " a"),
            ("a b", " a b"),
            (" a,,b  c-d_e ", " a b c-d_e"),
            ("\u{e9} x", " x"),
            ("!!!", ""),
            ("x.y:z", " x y z"),
        ] {
            assert_eq!(class_names(raw), expected);
        }
    }

    #[test]
    fn series_color_matches_pele() {
        for &(index, expected) in &[
            (
                -9,
                "var(--ondine-series-8,color-mix(in srgb,var(--_fg) 71%,var(--_bg)))",
            ),
            (
                -1,
                "var(--ondine-series-8,color-mix(in srgb,var(--_fg) 71%,var(--_bg)))",
            ),
            (
                0,
                "var(--ondine-series-1,color-mix(in srgb,var(--_fg) 100%,var(--_bg)))",
            ),
            (
                1,
                "var(--ondine-series-2,color-mix(in srgb,var(--_fg) 57%,var(--_bg)))",
            ),
            (
                7,
                "var(--ondine-series-8,color-mix(in srgb,var(--_fg) 71%,var(--_bg)))",
            ),
            (
                8,
                "var(--ondine-series-1,color-mix(in srgb,var(--_fg) 100%,var(--_bg)))",
            ),
            (
                15,
                "var(--ondine-series-8,color-mix(in srgb,var(--_fg) 71%,var(--_bg)))",
            ),
        ] {
            assert_eq!(series_color(index), expected);
        }
    }
}
