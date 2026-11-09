//! Writes the theme into an SVG document as literal values, for renderers without CSS custom
//! properties or `color-mix()`, such as server-side rasterizers, native SVG parsers, and email.
//!
//! Each `var()` in an attribute takes the host's value for its `--ondine-*` property, the root's
//! declaration of a private alias, or its own fallback. Each `color-mix()` of two parsed colors then
//! becomes one hex color. Declarations of custom properties are dropped, since nothing reads them.
//! Text content is copied as it is, so a label that spells out `var(` keeps its text.
//!
//! A mix of a host color that does not parse is an error that names its key. A mix that only
//! authored styles write becomes its first color, which every renderer draws.

use crate::error::{Code, Error};
use crate::svg::{builder, theme};
use crate::util::float;
use crate::yaml;

/// How deep values may nest. Ondine's aliases go three levels deep. The limit stops a host value
/// that refers to itself.
const MAX_DEPTH: u8 = 8;

/// The start of a mix.
const MIX: &str = "color-mix(";

/// CSS named colors, sorted by name for binary search.
const NAMED: [(&str, [u8; 3]); 148] = [
    ("aliceblue", [240, 248, 255]),
    ("antiquewhite", [250, 235, 215]),
    ("aqua", [0, 255, 255]),
    ("aquamarine", [127, 255, 212]),
    ("azure", [240, 255, 255]),
    ("beige", [245, 245, 220]),
    ("bisque", [255, 228, 196]),
    ("black", [0, 0, 0]),
    ("blanchedalmond", [255, 235, 205]),
    ("blue", [0, 0, 255]),
    ("blueviolet", [138, 43, 226]),
    ("brown", [165, 42, 42]),
    ("burlywood", [222, 184, 135]),
    ("cadetblue", [95, 158, 160]),
    ("chartreuse", [127, 255, 0]),
    ("chocolate", [210, 105, 30]),
    ("coral", [255, 127, 80]),
    ("cornflowerblue", [100, 149, 237]),
    ("cornsilk", [255, 248, 220]),
    ("crimson", [220, 20, 60]),
    ("cyan", [0, 255, 255]),
    ("darkblue", [0, 0, 139]),
    ("darkcyan", [0, 139, 139]),
    ("darkgoldenrod", [184, 134, 11]),
    ("darkgray", [169, 169, 169]),
    ("darkgreen", [0, 100, 0]),
    ("darkgrey", [169, 169, 169]),
    ("darkkhaki", [189, 183, 107]),
    ("darkmagenta", [139, 0, 139]),
    ("darkolivegreen", [85, 107, 47]),
    ("darkorange", [255, 140, 0]),
    ("darkorchid", [153, 50, 204]),
    ("darkred", [139, 0, 0]),
    ("darksalmon", [233, 150, 122]),
    ("darkseagreen", [143, 188, 143]),
    ("darkslateblue", [72, 61, 139]),
    ("darkslategray", [47, 79, 79]),
    ("darkslategrey", [47, 79, 79]),
    ("darkturquoise", [0, 206, 209]),
    ("darkviolet", [148, 0, 211]),
    ("deeppink", [255, 20, 147]),
    ("deepskyblue", [0, 191, 255]),
    ("dimgray", [105, 105, 105]),
    ("dimgrey", [105, 105, 105]),
    ("dodgerblue", [30, 144, 255]),
    ("firebrick", [178, 34, 34]),
    ("floralwhite", [255, 250, 240]),
    ("forestgreen", [34, 139, 34]),
    ("fuchsia", [255, 0, 255]),
    ("gainsboro", [220, 220, 220]),
    ("ghostwhite", [248, 248, 255]),
    ("gold", [255, 215, 0]),
    ("goldenrod", [218, 165, 32]),
    ("gray", [128, 128, 128]),
    ("green", [0, 128, 0]),
    ("greenyellow", [173, 255, 47]),
    ("grey", [128, 128, 128]),
    ("honeydew", [240, 255, 240]),
    ("hotpink", [255, 105, 180]),
    ("indianred", [205, 92, 92]),
    ("indigo", [75, 0, 130]),
    ("ivory", [255, 255, 240]),
    ("khaki", [240, 230, 140]),
    ("lavender", [230, 230, 250]),
    ("lavenderblush", [255, 240, 245]),
    ("lawngreen", [124, 252, 0]),
    ("lemonchiffon", [255, 250, 205]),
    ("lightblue", [173, 216, 230]),
    ("lightcoral", [240, 128, 128]),
    ("lightcyan", [224, 255, 255]),
    ("lightgoldenrodyellow", [250, 250, 210]),
    ("lightgray", [211, 211, 211]),
    ("lightgreen", [144, 238, 144]),
    ("lightgrey", [211, 211, 211]),
    ("lightpink", [255, 182, 193]),
    ("lightsalmon", [255, 160, 122]),
    ("lightseagreen", [32, 178, 170]),
    ("lightskyblue", [135, 206, 250]),
    ("lightslategray", [119, 136, 153]),
    ("lightslategrey", [119, 136, 153]),
    ("lightsteelblue", [176, 196, 222]),
    ("lightyellow", [255, 255, 224]),
    ("lime", [0, 255, 0]),
    ("limegreen", [50, 205, 50]),
    ("linen", [250, 240, 230]),
    ("magenta", [255, 0, 255]),
    ("maroon", [128, 0, 0]),
    ("mediumaquamarine", [102, 205, 170]),
    ("mediumblue", [0, 0, 205]),
    ("mediumorchid", [186, 85, 211]),
    ("mediumpurple", [147, 112, 219]),
    ("mediumseagreen", [60, 179, 113]),
    ("mediumslateblue", [123, 104, 238]),
    ("mediumspringgreen", [0, 250, 154]),
    ("mediumturquoise", [72, 209, 204]),
    ("mediumvioletred", [199, 21, 133]),
    ("midnightblue", [25, 25, 112]),
    ("mintcream", [245, 255, 250]),
    ("mistyrose", [255, 228, 225]),
    ("moccasin", [255, 228, 181]),
    ("navajowhite", [255, 222, 173]),
    ("navy", [0, 0, 128]),
    ("oldlace", [253, 245, 230]),
    ("olive", [128, 128, 0]),
    ("olivedrab", [107, 142, 35]),
    ("orange", [255, 165, 0]),
    ("orangered", [255, 69, 0]),
    ("orchid", [218, 112, 214]),
    ("palegoldenrod", [238, 232, 170]),
    ("palegreen", [152, 251, 152]),
    ("paleturquoise", [175, 238, 238]),
    ("palevioletred", [219, 112, 147]),
    ("papayawhip", [255, 239, 213]),
    ("peachpuff", [255, 218, 185]),
    ("peru", [205, 133, 63]),
    ("pink", [255, 192, 203]),
    ("plum", [221, 160, 221]),
    ("powderblue", [176, 224, 230]),
    ("purple", [128, 0, 128]),
    ("rebeccapurple", [102, 51, 153]),
    ("red", [255, 0, 0]),
    ("rosybrown", [188, 143, 143]),
    ("royalblue", [65, 105, 225]),
    ("saddlebrown", [139, 69, 19]),
    ("salmon", [250, 128, 114]),
    ("sandybrown", [244, 164, 96]),
    ("seagreen", [46, 139, 87]),
    ("seashell", [255, 245, 238]),
    ("sienna", [160, 82, 45]),
    ("silver", [192, 192, 192]),
    ("skyblue", [135, 206, 235]),
    ("slateblue", [106, 90, 205]),
    ("slategray", [112, 128, 144]),
    ("slategrey", [112, 128, 144]),
    ("snow", [255, 250, 250]),
    ("springgreen", [0, 255, 127]),
    ("steelblue", [70, 130, 180]),
    ("tan", [210, 180, 140]),
    ("teal", [0, 128, 128]),
    ("thistle", [216, 191, 216]),
    ("tomato", [255, 99, 71]),
    ("turquoise", [64, 224, 208]),
    ("violet", [238, 130, 238]),
    ("wheat", [245, 222, 179]),
    ("white", [255, 255, 255]),
    ("whitesmoke", [245, 245, 245]),
    ("yellow", [255, 255, 0]),
    ("yellowgreen", [154, 205, 50]),
];

/// A parsed color: red, green, and blue from 0 to 255, and alpha from 0 to 1.
type Color = [f64; 4];

/// A value of the `theme` option.
#[derive(Clone, Debug)]
pub(crate) struct Property {
    /// The key as the host writes it, such as `surfaceAlt`.
    pub(crate) key: String,
    /// The full property name, such as `--ondine-surface-alt`.
    pub(crate) name: String,
    /// The value, escaped for an attribute.
    pub(crate) value: String,
}

/// Where `var()` references read their values.
struct Lookup<'a> {
    /// The custom properties the root element declares.
    declared: Vec<(&'a str, &'a str)>,
    /// The host's values.
    theme: &'a [Property],
}

impl Lookup<'_> {
    /// The value of the custom property `name`, when the host or the root sets one.
    fn get(&self, name: &str) -> Option<&str> {
        if name.starts_with("--ondine-") {
            return self
                .theme
                .iter()
                .find(|property| property.name == name)
                .map(|property| property.value.as_str());
        }
        self.declared
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| *value)
    }
}

/// Reads the `theme` option: an object of `--ondine-*` property names in camel case, without the
/// prefix, and their values. Keys become full property names, so `surfaceAlt` sets
/// `--ondine-surface-alt` and `series1` sets `--ondine-series-1`. A value that is not a string, or
/// that an author style could not hold, is ignored, so its property keeps its default.
pub(crate) fn properties(value: &yaml::Value) -> Option<Vec<Property>> {
    let yaml::Value::Object(entries) = value else {
        return None;
    };

    let mut properties = Vec::with_capacity(entries.len());
    for (key, value) in entries {
        let yaml::Value::String(value) = value else {
            continue;
        };
        let value = value.trim();
        if value.is_empty() || theme::unsafe_value(value) {
            continue;
        }

        let mut name = String::from("--ondine-");
        let mut digits = false;
        for c in key.chars() {
            if c.is_ascii_uppercase() || (c.is_ascii_digit() && !digits) {
                name.push('-');
            }
            name.push(c.to_ascii_lowercase());
            digits = c.is_ascii_digit();
        }
        properties.push(Property {
            key: key.clone(),
            name,
            value: builder::esc(value).into_owned(),
        });
    }
    Some(properties)
}

/// `svg` with every themed attribute written out with `theme`.
///
/// # Errors
///
/// Returns an [`Error`] when a `color-mix()` reads a host value that is not a color it parses.
pub(crate) fn resolve(svg: &str, theme: &[Property]) -> Result<String, Error> {
    let lookup = Lookup {
        declared: root_declarations(svg),
        theme,
    };

    let mut out = String::with_capacity(svg.len());
    let mut rest = svg;
    while let Some(open) = rest.find('<') {
        out.push_str(rest.get(..open).unwrap_or(""));
        rest = push_tag(&mut out, rest.get(open..).unwrap_or(""), &lookup)?;
    }
    out.push_str(rest);
    Ok(out)
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
fn push_tag<'a>(out: &mut String, text: &'a str, lookup: &Lookup<'_>) -> Result<&'a str, Error> {
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
                        push_style(out, value, lookup)?;
                    } else {
                        push_value(out, value, lookup)?;
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
    Ok(text.get(end..).unwrap_or(""))
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
fn push_style(out: &mut String, style: &str, lookup: &Lookup<'_>) -> Result<(), Error> {
    let mut first = true;
    let mut rest = style;
    loop {
        let (part, next) = split_once_top(rest, b';');
        if !part.trim_start().starts_with("--") {
            if !first {
                out.push(';');
            }
            push_value(out, part, lookup)?;
            first = false;
        }
        let Some(next) = next else {
            return Ok(());
        };
        rest = next;
    }
}

/// Appends `value` with its `var()` references and color mixes written out.
fn push_value(out: &mut String, value: &str, lookup: &Lookup<'_>) -> Result<(), Error> {
    if !value.contains('(') {
        out.push_str(value);
        return Ok(());
    }

    let mut vars = String::with_capacity(value.len() + 16);
    push_vars(&mut vars, value, lookup, 0);
    push_mixes(out, &vars, lookup, 0)
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

/// Appends `text` with each `color-mix()` evaluated to a hex color.
///
/// # Errors
///
/// Returns an [`Error`] naming the first host value in a mix that does not evaluate, if any.
fn push_mixes(out: &mut String, text: &str, lookup: &Lookup<'_>, depth: u8) -> Result<(), Error> {
    let mut rest = text;
    while let Some(start) = rest.find(MIX) {
        let Some(end) = close(rest, start + MIX.len() - 1) else {
            break;
        };
        out.push_str(rest.get(..start).unwrap_or(""));

        let call = rest.get(start..=end).unwrap_or("");
        // The mix holds the host values written out, so a value in it that is not a color
        // is the one that stops the mix.
        if let Some(color) = color(call, depth) {
            push_hex(out, color);
        } else if let Some(property) = lookup
            .theme
            .iter()
            .find(|property| call.contains(&property.value) && color(&property.value, 0).is_none())
        {
            return Err(unmixable(&property.key));
        } else if depth < MAX_DEPTH {
            let inner = call.get(MIX.len()..call.len() - 1).unwrap_or("");
            push_mixes(out, first_color(inner), lookup, depth + 1)?;
        } else {
            out.push_str(call);
        }
        rest = rest.get(end + 1..).unwrap_or("");
    }
    out.push_str(rest);
    Ok(())
}

/// The error for a mix that reads the host value of `key`.
#[cold]
fn unmixable(key: &str) -> Error {
    let mut message = String::from("The theme value for \"");
    message.push_str(key);
    message.push_str("\" cannot be mixed. Use a hex, named, rgb(), or transparent color.");
    Error::new(Code::Unsupported, message)
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

/// `text` as a color: a parsed color, or a `color-mix()` of them evaluated.
fn color(text: &str, depth: u8) -> Option<Color> {
    let text = text.trim();
    if !text.starts_with(MIX) {
        return parse_color(text);
    }
    if depth >= MAX_DEPTH || close(text, MIX.len() - 1)? + 1 != text.len() {
        return None;
    }
    mix(text.get(MIX.len()..text.len() - 1)?, depth + 1)
}

/// `color-mix(in srgb, A p%, B q%)` as CSS Color 5 mixes it: percentages default to the rest
/// of 100 and scale to sum to 100, a sum under 100 lowers the alpha, and channels mix premultiplied.
fn mix(inner: &str, depth: u8) -> Option<Color> {
    let [space, first, second] = arguments(inner)?;
    if !space.trim().eq_ignore_ascii_case("in srgb") {
        return None;
    }

    let (first, p1) = percentage(first)?;
    let (second, p2) = percentage(second)?;
    let a = color(first, depth)?;
    let b = color(second, depth)?;
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

/// A hex color, `rgb()` or `rgba()`, a named color, or `transparent`.
fn parse_color(text: &str) -> Option<Color> {
    if text.eq_ignore_ascii_case("transparent") {
        return Some([0.0; 4]);
    }
    if let Some(digits) = text.strip_prefix('#') {
        return parse_hex(digits.as_bytes());
    }
    if let Some(inner) = function(text, "rgba(").or_else(|| function(text, "rgb(")) {
        return parse_rgb(inner);
    }

    let at = NAMED
        .binary_search_by(|(name, _)| {
            name.bytes()
                .cmp(text.bytes().map(|c| c.to_ascii_lowercase()))
        })
        .ok()?;
    let (_, [red, green, blue]) = NAMED.get(at)?;
    Some([f64::from(*red), f64::from(*green), f64::from(*blue), 1.0])
}

/// The arguments of `text` when it calls the function `open`, such as `rgb(`, in any case.
fn function<'a>(text: &'a str, open: &str) -> Option<&'a str> {
    text.get(..open.len())
        .filter(|head| head.eq_ignore_ascii_case(open))?;
    text.get(open.len()..)?.strip_suffix(')')
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

/// The arguments of `rgb()` or `rgba()`: three channels and an optional alpha, separated by commas
/// (`51, 102, 153, 0.5`) or by spaces with a slash before the alpha (`51 102 153 / 50%`).
fn parse_rgb(inner: &str) -> Option<Color> {
    let (channels, alpha) = if inner.contains(',') {
        let mut parts: Vec<&str> = inner.split(',').map(str::trim).collect();
        let alpha = if parts.len() == 4 { parts.pop() } else { None };
        (parts, alpha)
    } else {
        let (channels, alpha) = match inner.split_once('/') {
            Some((channels, alpha)) => (channels, Some(alpha.trim())),
            None => (inner, None),
        };
        (channels.split_ascii_whitespace().collect(), alpha)
    };
    let &[red, green, blue] = channels.as_slice() else {
        return None;
    };

    let channel = |text: &str| number(text, 2.55).map(|value| value.clamp(0.0, 255.0));
    let alpha = match alpha {
        Some(alpha) => number(alpha, 0.01)?.clamp(0.0, 1.0),
        None => 1.0,
    };
    Some([channel(red)?, channel(green)?, channel(blue)?, alpha])
}

/// A finite number, or a percentage times `scale`.
fn number(text: &str, scale: f64) -> Option<f64> {
    let value = match text.strip_suffix('%') {
        Some(percent) => float::parse(percent)? * scale,
        None => float::parse(text)?,
    };
    value.is_finite().then_some(value)
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
