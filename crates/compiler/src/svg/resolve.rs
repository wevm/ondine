//! Writes the theme into an SVG document as literal values, for renderers without CSS custom
//! properties or `color-mix()`, such as server-side rasterizers, native SVG parsers, and email.
//!
//! Each `var()` in an attribute takes the host's value for its `--ondine-*` property, the root's
//! declaration of a private alias, or its own fallback. Each `color-mix()` of two parsed colors then
//! becomes one hex color. Declarations of custom properties are dropped, since nothing reads them.
//! Text content is copied as it is, so a label that spells out `var(` keeps its text.

use crate::util::float;

/// How deep values may nest. Ondine's aliases go three levels deep. The limit stops a host value
/// that refers to itself.
const MAX_DEPTH: u8 = 8;

/// A parsed color: red, green, and blue from 0 to 255, and alpha from 0 to 1.
type Color = [f64; 4];

/// Where `var()` references read their values.
struct Lookup<'a> {
    /// The custom properties the root element declares.
    declared: Vec<(&'a str, &'a str)>,
    /// The host's values by full property name, such as `--ondine-bg`.
    theme: &'a [(String, String)],
}

impl Lookup<'_> {
    /// The value of the custom property `name`, when the host or the root sets one.
    fn get(&self, name: &str) -> Option<&str> {
        if name.starts_with("--ondine-") {
            return self
                .theme
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str());
        }
        self.declared
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    }
}

/// `svg` with every themed attribute written out with `theme`, a list of full
/// `--ondine-*` property names and their values.
pub(crate) fn resolve(svg: &str, theme: &[(String, String)]) -> String {
    let lookup = Lookup {
        declared: root_declarations(svg),
        theme,
    };

    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(open) = rest.find('<') {
        out.push_str(rest.get(..open).unwrap_or(""));
        rest = push_tag(&mut out, rest.get(open..).unwrap_or(""), &lookup);
    }
    out.push_str(rest);
    out
}

/// The custom properties declared in the `style` of the first `<svg>` tag.
fn root_declarations(svg: &str) -> Vec<(&str, &str)> {
    let mut declared = Vec::new();
    let Some(tag) = svg.find("<svg").and_then(|at| svg.get(at..)) else {
        return declared;
    };
    let tag = tag.get(..tag.find('>').unwrap_or(tag.len())).unwrap_or("");
    let Some(style) = tag.find(" style=\"").and_then(|at| tag.get(at + 8..)) else {
        return declared;
    };

    let mut rest = style
        .get(..style.find('"').unwrap_or(style.len()))
        .unwrap_or("");
    loop {
        let (part, next) = split_once_top(rest, b';');
        if let Some(colon) = part.find(':') {
            let name = part.get(..colon).unwrap_or("").trim();
            if name.starts_with("--") {
                declared.push((name, part.get(colon + 1..).unwrap_or("").trim()));
            }
        }
        let Some(next) = next else {
            return declared;
        };
        rest = next;
    }
}

/// Appends the tag at the start of `text` with its themed attributes written out, and returns
/// the text after it. Attributes are escaped, so a `>` outside quotes ends the tag.
fn push_tag<'a>(out: &mut String, text: &'a str, lookup: &Lookup<'_>) -> &'a str {
    let bytes = text.as_bytes();
    let mut copied = 0;
    let mut name = "";
    let mut at = 1;
    while let Some(&c) = bytes.get(at) {
        match c {
            b'>' => {
                at += 1;
                break;
            }
            b'=' => name = attribute_name(text, at),
            b'"' | b'\'' => {
                let start = at + 1;
                let end = text
                    .get(start..)
                    .and_then(|rest| rest.find(char::from(c)))
                    .map_or(text.len(), |length| start + length);
                let value = text.get(start..end).unwrap_or("");
                if themed(name) && value.contains('(') {
                    out.push_str(text.get(copied..start).unwrap_or(""));
                    if name == "style" {
                        push_style(out, value, lookup);
                    } else {
                        push_value(out, value, lookup);
                    }
                    copied = end;
                }
                name = "";
                at = end;
            }
            _ => {}
        }
        at += 1;
    }

    let end = at.min(text.len());
    out.push_str(text.get(copied..end).unwrap_or(""));
    text.get(end..).unwrap_or("")
}

/// The name of the attribute whose `=` is at `eq`.
fn attribute_name(text: &str, eq: usize) -> &str {
    let head = text.get(..eq).unwrap_or("");
    let start = head
        .bytes()
        .rposition(|c| !(c.is_ascii_alphanumeric() || matches!(c, b'-' | b':' | b'_')))
        .map_or(0, |at| at + 1);
    head.get(start..).unwrap_or("")
}

/// Whether the attribute `name` may hold a themed value. Names, links, and accessible text
/// hold authored text, which is copied as it is.
fn themed(name: &str) -> bool {
    !(matches!(name, "class" | "href" | "id" | "xlink:href")
        || name.starts_with("aria-")
        || name.starts_with("data-"))
}

/// Appends a `style` value without its custom properties and with the rest written out.
fn push_style(out: &mut String, style: &str, lookup: &Lookup<'_>) {
    let mut first = true;
    let mut rest = style;
    loop {
        let (part, next) = split_once_top(rest, b';');
        if !part.trim_start().starts_with("--") {
            if !first {
                out.push(';');
            }
            push_value(out, part, lookup);
            first = false;
        }
        let Some(next) = next else {
            return;
        };
        rest = next;
    }
}

/// Appends `value` with its `var()` references and color mixes written out.
fn push_value(out: &mut String, value: &str, lookup: &Lookup<'_>) {
    if !value.contains('(') {
        out.push_str(value);
        return;
    }

    let mut vars = String::with_capacity(value.len() + 16);
    push_vars(&mut vars, value, lookup, 0);
    push_mixes(out, &vars, 0);
}

/// Appends `text` with each `var(--name, fallback)` replaced by the value of `--name`, or by
/// the fallback, written out in turn. A reference with neither is kept.
fn push_vars(out: &mut String, text: &str, lookup: &Lookup<'_>, depth: u8) {
    let mut rest = text;
    while let Some(start) = rest.find("var(") {
        let Some(end) = close(rest, start + 3) else {
            break;
        };
        out.push_str(rest.get(..start).unwrap_or(""));

        let inner = rest.get(start + 4..end).unwrap_or("");
        let (name, fallback) = match inner.find(',') {
            Some(comma) => (
                inner.get(..comma).unwrap_or(""),
                inner.get(comma + 1..).map(str::trim),
            ),
            None => (inner, None),
        };
        match lookup.get(name.trim()).or(fallback) {
            Some(value) if depth < MAX_DEPTH => push_vars(out, value, lookup, depth + 1),
            _ => out.push_str(rest.get(start..=end).unwrap_or("")),
        }
        rest = rest.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
}

/// Appends `text` with each `color-mix()` evaluated, innermost first. A mix whose colors do not
/// parse becomes its first color, which every renderer draws.
fn push_mixes(out: &mut String, text: &str, depth: u8) {
    const CALL: &str = "color-mix(";

    let mut rest = text;
    while let Some(start) = rest.find(CALL) {
        let Some(end) = close(rest, start + CALL.len() - 1) else {
            break;
        };
        out.push_str(rest.get(..start).unwrap_or(""));

        let raw = rest.get(start + CALL.len()..end).unwrap_or("");
        if depth < MAX_DEPTH {
            let mut inner = String::with_capacity(raw.len());
            push_mixes(&mut inner, raw, depth + 1);
            match mix(&inner) {
                Some(color) => push_hex(out, color),
                None => out.push_str(first_color(&inner)),
            }
        } else {
            out.push_str(rest.get(start..=end).unwrap_or(""));
        }
        rest = rest.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
}

/// The index of the parenthesis that closes the one at `open`.
fn close(text: &str, open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (at, c) in text.bytes().enumerate().skip(open) {
        match c {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

/// `text` before the first `separator` outside parentheses, and the text after it, if any.
fn split_once_top(text: &str, separator: u8) -> (&str, Option<&str>) {
    let mut depth = 0_usize;
    for (at, c) in text.bytes().enumerate() {
        match c {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            c if c == separator && depth == 0 => {
                return (text.get(..at).unwrap_or(""), text.get(at + 1..));
            }
            _ => {}
        }
    }
    (text, None)
}

/// The arguments of a `color-mix()`: the color space and two colors with optional percentages.
fn arguments(inner: &str) -> Option<[&str; 3]> {
    let (space, rest) = split_once_top(inner, b',');
    let (first, second) = split_once_top(rest?, b',');
    let second = second?;
    if split_once_top(second, b',').1.is_some() {
        return None;
    }
    Some([space, first, second])
}

/// The first color of a mix, without its percentage, or nothing when the mix has none.
fn first_color(inner: &str) -> &str {
    let (_, rest) = split_once_top(inner, b',');
    let first = split_once_top(rest.unwrap_or(""), b',').0;
    percentage(first).map_or(first.trim(), |(color, _)| color)
}

/// `color-mix(in srgb, A p%, B q%)` as CSS Color 5 mixes it: percentages default to the rest
/// of 100 and scale to sum to 100, a sum under 100 lowers the alpha, and channels mix premultiplied.
fn mix(inner: &str) -> Option<Color> {
    let [space, first, second] = arguments(inner)?;
    if !space.trim().eq_ignore_ascii_case("in srgb") {
        return None;
    }

    let (first, p1) = percentage(first)?;
    let (second, p2) = percentage(second)?;
    let a = parse_color(first)?;
    let b = parse_color(second)?;
    let (p1, p2) = match (p1, p2) {
        (None, None) => (50.0, 50.0),
        (Some(p1), None) => (p1, 100.0 - p1),
        (None, Some(p2)) => (100.0 - p2, p2),
        (Some(p1), Some(p2)) => (p1, p2),
    };
    let sum = p1 + p2;
    if !(0.0..=100.0).contains(&p1) || !(0.0..=100.0).contains(&p2) || sum <= 0.0 {
        return None;
    }

    let scale = if sum < 100.0 { sum / 100.0 } else { 1.0 };
    let (p1, p2) = (p1 / sum, p2 / sum);
    let [r1, g1, b1, a1] = a;
    let [r2, g2, b2, a2] = b;
    let alpha = a1 * p1 + a2 * p2;
    let channel = |x: f64, y: f64| {
        if alpha == 0.0 {
            return 0.0;
        }
        (x * a1 * p1 + y * a2 * p2) / alpha
    };
    Some([
        channel(r1, r2),
        channel(g1, g2),
        channel(b1, b2),
        alpha * scale,
    ])
}

/// A mix argument split into its color and its percentage, if it has one. `None` when the
/// percentage is not a number.
fn percentage(argument: &str) -> Option<(&str, Option<f64>)> {
    let argument = argument.trim();
    let Some(number) = argument.strip_suffix('%') else {
        return Some((argument, None));
    };
    let space = number.rfind(|c: char| c.is_ascii_whitespace())?;
    let value = float::parse(number.get(space + 1..)?).filter(|value| value.is_finite())?;
    Some((number.get(..space)?.trim(), Some(value)))
}

/// A hex color or `transparent`. Other colors are left to the fallback, which keeps them as written.
fn parse_color(text: &str) -> Option<Color> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("transparent") {
        return Some([0.0; 4]);
    }
    parse_hex(text.strip_prefix('#')?.as_bytes())
}

/// Hex digits of a color: `rgb`, `rgba`, `rrggbb`, or `rrggbbaa`.
fn parse_hex(digits: &[u8]) -> Option<Color> {
    let width = match digits.len() {
        3 | 4 => 1,
        6 | 8 => 2,
        _ => return None,
    };

    let mut color = [255.0; 4];
    for (slot, chunk) in color.iter_mut().zip(digits.chunks(width)) {
        let mut value = 0;
        for &c in chunk {
            value = value * 16 + char::from(c).to_digit(16)?;
        }
        // A short digit stands for itself repeated, so `f` is `ff`.
        *slot = f64::from(if width == 1 { value * 17 } else { value });
    }
    let [red, green, blue, alpha] = color;
    Some([red, green, blue, alpha / 255.0])
}

/// Appends `color` as `#rrggbb`, or `#rrggbbaa` when it is not opaque.
fn push_hex(out: &mut String, color: Color) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";

    let [red, green, blue, alpha] = color;
    let alpha = byte(alpha * 255.0);
    out.push('#');
    for value in [byte(red), byte(green), byte(blue)]
        .into_iter()
        .chain((alpha < 255).then_some(alpha))
    {
        for nibble in [value >> 4, value & 15] {
            out.push(char::from(
                DIGITS.get(usize::from(nibble)).copied().unwrap_or(b'0'),
            ));
        }
    }
}

/// `value` rounded to the nearest byte, halves up.
fn byte(value: f64) -> u8 {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "the value is clamped to a byte's range first"
    )]
    let byte = (value + 0.5).floor().clamp(0.0, 255.0) as u8;
    byte
}
