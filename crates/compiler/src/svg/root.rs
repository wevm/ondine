//! The root SVG document around a diagram's content. Port of Pele's `src/svg/root.ts`.

use std::borrow::Cow;

use crate::svg::builder::{esc, esc_text};
use crate::svg::theme::{RADIUS, ROOT_STYLE, ROUNDED, font_style};
use crate::yaml::push_number;

/// The host's options that the shared SVG modules read. A subset of Pele's `RenderOptions`;
/// diagram modules add the fields they read as they are ported. Ondine has no font options, since
/// fonts come from `--ondine-font` and every width is monospace.
#[derive(Clone, Debug, Default)]
pub(crate) struct RenderOptions {
    /// Whether a diagram that runs across may be drawn running down when it does not fit
    /// `max_width`. On unless `Some(false)`.
    pub(crate) auto_direction: Option<bool>,
    /// The width under which a diagram may turn to run down. 640 when absent or not positive.
    pub(crate) direction_breakpoint: Option<f64>,
    /// Font size in pixels. 16 when absent.
    pub(crate) font_size: Option<f64>,
    /// The prefix of element IDs, so several diagrams can share a page. `pele` when absent.
    pub(crate) id_prefix: Option<String>,
    /// Schemes an image may use besides a relative address.
    pub(crate) image_schemes: Option<Vec<String>>,
    /// Whether images are drawn. On unless `Some(false)`.
    pub(crate) images: Option<bool>,
    /// The most UTF-16 units of source to accept. 50,000 when absent.
    pub(crate) limit: Option<f64>,
    /// The `rel` of links. `noopener` when absent; empty omits it.
    pub(crate) link_rel: Option<String>,
    /// Schemes a link may use besides a relative address.
    pub(crate) link_schemes: Option<Vec<String>>,
    /// Whether links are drawn. On unless `Some(false)`.
    pub(crate) links: Option<bool>,
    /// The most edges a flowchart may have. 5,000 when absent.
    pub(crate) max_edges: Option<f64>,
    /// The width the host has for the diagram, in pixels. A type that can draw itself narrower
    /// does, so its text is not shrunk with it.
    pub(crate) max_width: Option<f64>,
    /// The time a gantt chart schedules against, in milliseconds since the epoch. The epoch
    /// when absent, since the compiler reads no clock.
    pub(crate) now: Option<f64>,
    /// The most UTF-16 units of SVG to produce. 4,000,000 when absent.
    pub(crate) output_limit: Option<f64>,
    /// Space around a diagram. 8 when absent.
    pub(crate) padding: Option<f64>,
    /// Whether the document scales down to its container. On unless `Some(false)`.
    pub(crate) responsive: Option<bool>,
    /// When present, the theme is written into the document as literal values: full
    /// `--ondine-*` property names and the host's values for them. The rest keep their defaults.
    pub(crate) theme: Option<Vec<(String, String)>>,
}

/// A diagram's accessible title and description.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Accessible<'a> {
    /// The accessible description.
    pub(crate) acc_descr: Option<&'a str>,
    /// The accessible title.
    pub(crate) acc_title: Option<&'a str>,
}

/// Moves the themed corner radius of each `<rect>` into its style. Browsers only lately take a
/// variable in the `rx` attribute, and all of them take one in a style. It goes first there, so a
/// radius the diagram's author set comes after it and wins. Also returns whether any moved.
fn rounded(content: &str) -> (Cow<'_, str>, bool) {
    let mut moved = false;
    let bytes = content.as_bytes();
    let mut out = String::with_capacity(content.len() + 64);
    let mut start = 0;
    let mut at = 0;
    while let Some(found) = content.get(at..).and_then(|rest| rest.find("<rect")) {
        let tag_start = at + found;
        let after = tag_start + 5;
        // `\b` after `rect`: the next character is not a word character.
        let bounded = !bytes
            .get(after)
            .is_some_and(|c| c.is_ascii_alphanumeric() || *c == b'_');
        let close = content.get(after..).and_then(|rest| rest.find('>'));
        let (true, Some(close)) = (bounded, close) else {
            at = tag_start + 1;
            continue;
        };
        let tag_end = after + close + 1;
        let tag = content.get(tag_start..tag_end).unwrap_or("");
        out.push_str(content.get(start..tag_start).unwrap_or(""));
        moved |= push_rounded(&mut out, tag);
        start = tag_end;
        at = tag_end;
    }
    out.push_str(content.get(start..).unwrap_or(""));
    (Cow::Owned(out), moved)
}

/// Appends one `<rect>` tag with its themed radius moved into its style, and returns whether it
/// had one.
fn push_rounded(out: &mut String, tag: &str) -> bool {
    let Some(at) = tag.find(ROUNDED) else {
        out.push_str(tag);
        return false;
    };

    let mut rest = String::with_capacity(tag.len());
    rest.push_str(tag.get(..at).unwrap_or(""));
    rest.push_str(tag.get(at + ROUNDED.len()..).unwrap_or(""));
    if let Some(style) = rest.find(" style=\"") {
        out.push_str(rest.get(..style + 8).unwrap_or(""));
        out.push_str("rx:var(--_r);");
        out.push_str(rest.get(style + 8..).unwrap_or(""));
        return true;
    }

    let end = if rest.ends_with("/>") {
        rest.len() - 2
    } else {
        rest.len().saturating_sub(1)
    };
    out.push_str(rest.get(..end).unwrap_or(""));
    out.push_str(" style=\"rx:var(--_r)\"");
    out.push_str(rest.get(end..).unwrap_or(""));
    true
}

/// The weights of bold text, each set only in a diagram that has such text: the alias and the
/// name in its public variable.
const WEIGHTS: [(&str, &str); 3] = [("--_tw", "title"), ("--_hw", "heading"), ("--_w", "bold")];

/// Wraps a diagram's content in the root `<svg>` document of the given size, with its theme,
/// font at `font_size`, accessible title and description, and diagram `type` class.
pub(crate) fn svg_document(
    r#type: &str,
    width: f64,
    height: f64,
    font_size: f64,
    options: &RenderOptions,
    acc: Accessible<'_>,
    content: &str,
) -> String {
    let prefix = esc(options.id_prefix.as_deref().unwrap_or("ondine"));
    let mut head = String::new();
    let mut aria = String::new();
    if let Some(title) = acc.acc_title.filter(|title| !title.is_empty()) {
        head.push_str("<title id=\"");
        head.push_str(&prefix);
        head.push_str("-title\">");
        head.push_str(&esc_text(title));
        head.push_str("</title>");
        aria.push_str(" aria-labelledby=\"");
        aria.push_str(&prefix);
        aria.push_str("-title\"");
    }
    if let Some(descr) = acc.acc_descr.filter(|descr| !descr.is_empty()) {
        head.push_str("<desc id=\"");
        head.push_str(&prefix);
        head.push_str("-desc\">");
        head.push_str(&esc_text(descr));
        head.push_str("</desc>");
        aria.push_str(" aria-describedby=\"");
        aria.push_str(&prefix);
        aria.push_str("-desc\"");
    }

    // One scan finds the themed radius and every alias the content reads.
    let mut radius = false;
    let mut themed = false;
    let mut used = [false; WEIGHTS.len()];
    for (at, _) in content.match_indices("var(--") {
        let rest = content.get(at + 6..).unwrap_or("");
        let Some(name) = rest.strip_prefix("_") else {
            themed |= content
                .get(at.saturating_sub(ROUNDED.len() - RADIUS.len() - 1)..)
                .is_some_and(|tail| tail.starts_with(ROUNDED));
            continue;
        };
        radius |= name.starts_with("r)");
        for (used, (alias, _)) in used.iter_mut().zip(&WEIGHTS) {
            *used |= name
                .strip_prefix(alias.get(3..).unwrap_or(""))
                .is_some_and(|tail| tail.starts_with(')'));
        }
    }

    // A written-out radius stays in the `rx` attribute, which renderers without CSS geometry read.
    let content = if themed && options.theme.is_none() {
        let (content, moved) = rounded(content);
        radius |= moved;
        content
    } else {
        Cow::Borrowed(content)
    };

    let mut weights = String::new();
    if radius {
        weights.push_str(";--_r:");
        weights.push_str(RADIUS);
    }
    for ((alias, name), used) in WEIGHTS.iter().zip(used) {
        if used {
            weights.push(';');
            weights.push_str(alias);
            weights.push_str(":var(--ondine-");
            weights.push_str(name);
            weights.push_str("-weight,bold)");
        }
    }

    let mut out = String::with_capacity(content.len() + 1024);
    out.push_str("<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"ondine ondine-");
    out.push_str(r#type);
    out.push_str("\" viewBox=\"0 0 ");
    push_number(&mut out, width);
    out.push(' ');
    push_number(&mut out, height);
    out.push_str("\" width=\"");
    push_number(&mut out, width);
    out.push_str("\" height=\"");
    push_number(&mut out, height);
    out.push_str("\" style=\"");
    if options.responsive != Some(false) {
        out.push_str("max-width:100%;height:auto;");
    }
    out.push_str(ROOT_STYLE);
    out.push_str(&weights);
    out.push_str(&font_style(font_size));
    out.push_str(
        "\" fill=\"var(--_fg)\" role=\"graphics-document document\" aria-roledescription=\"",
    );
    out.push_str(r#type);
    out.push('"');
    out.push_str(&aria);
    out.push('>');
    out.push_str(&head);
    out.push_str(&content);
    out.push_str("</svg>");
    out
}

#[cfg(test)]
mod tests {
    //! Expectations come from Pele's `svgDocument`, after the theme substitution: Pele's root
    //! style, font declarations, and `--pele-*` variables replaced by Ondine's.

    use super::{Accessible, RenderOptions, svg_document};

    #[expect(clippy::type_complexity, reason = "a table of cases")]
    const CASES: &[(
        &str,
        f64,
        f64,
        f64,
        Option<&str>,
        Option<bool>,
        Option<&str>,
        Option<&str>,
        &str,
        &str,
    )] = &[
        (
            "flowchart",
            120.0,
            80.0,
            16.0,
            None,
            None,
            None,
            None,
            "<rect x=\"0\" y=\"0\" width=\"5\" height=\"5\" rx=\"var(--ondine-radius,0)\" fill=\"red\"/>",
            "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"ondine ondine-flowchart\" viewBox=\"0 0 120 80\" width=\"120\" height=\"80\" style=\"max-width:100%;height:auto;--_bg:var(--ondine-bg,#fff);--_fg:var(--ondine-fg,#000);--_m:var(--ondine-muted,var(--_fg));--_l:var(--ondine-line,var(--_fg));--_s:var(--ondine-surface,var(--_bg));--_a:var(--ondine-surface-alt,color-mix(in srgb,var(--_fg) 20%,var(--_bg)));--_b:var(--ondine-border,var(--_fg));--_c:var(--ondine-accent,var(--_fg));--_sw:var(--ondine-stroke,2px);stroke-width:var(--_sw);color:var(--_fg);--_r:var(--ondine-radius,0);--_fm:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font:16px monospace;letter-spacing:normal;word-spacing:normal;text-transform:none;direction:ltr;font-family:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font-kerning:none;font-variant-ligatures:none;text-rendering:geometricPrecision\" fill=\"var(--_fg)\" role=\"graphics-document document\" aria-roledescription=\"flowchart\"><rect x=\"0\" y=\"0\" width=\"5\" height=\"5\" fill=\"red\" style=\"rx:var(--_r)\"/></svg>",
        ),
        (
            "flowchart",
            120.5,
            80.0,
            14.0,
            Some("my\"id"),
            Some(false),
            Some("Title &amp; <x>"),
            Some("Desc"),
            "<rect rx=\"var(--ondine-radius,0)\" style=\"fill:red\"/><rectangle rx=\"var(--ondine-radius,0)\"/><text font-weight=\"var(--_w)\">b</text>",
            "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"ondine ondine-flowchart\" viewBox=\"0 0 120.5 80\" width=\"120.5\" height=\"80\" style=\"--_bg:var(--ondine-bg,#fff);--_fg:var(--ondine-fg,#000);--_m:var(--ondine-muted,var(--_fg));--_l:var(--ondine-line,var(--_fg));--_s:var(--ondine-surface,var(--_bg));--_a:var(--ondine-surface-alt,color-mix(in srgb,var(--_fg) 20%,var(--_bg)));--_b:var(--ondine-border,var(--_fg));--_c:var(--ondine-accent,var(--_fg));--_sw:var(--ondine-stroke,2px);stroke-width:var(--_sw);color:var(--_fg);--_r:var(--ondine-radius,0);--_w:var(--ondine-bold-weight,bold);--_fm:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font:14px monospace;letter-spacing:normal;word-spacing:normal;text-transform:none;direction:ltr;font-family:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font-kerning:none;font-variant-ligatures:none;text-rendering:geometricPrecision\" fill=\"var(--_fg)\" role=\"graphics-document document\" aria-roledescription=\"flowchart\" aria-labelledby=\"my&quot;id-title\" aria-describedby=\"my&quot;id-desc\"><title id=\"my&quot;id-title\">Title &amp; &lt;x&gt;</title><desc id=\"my&quot;id-desc\">Desc</desc><rect style=\"rx:var(--_r);fill:red\"/><rectangle rx=\"var(--ondine-radius,0)\"/><text font-weight=\"var(--_w)\">b</text></svg>",
        ),
        (
            "pie",
            10.0,
            10.0,
            16.0,
            None,
            None,
            Some(""),
            Some("only desc"),
            "<g><text font-weight=\"var(--_tw)\">t</text><text font-weight=\"var(--_hw)\">h</text></g><rect width=\"1\"/>",
            "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"ondine ondine-pie\" viewBox=\"0 0 10 10\" width=\"10\" height=\"10\" style=\"max-width:100%;height:auto;--_bg:var(--ondine-bg,#fff);--_fg:var(--ondine-fg,#000);--_m:var(--ondine-muted,var(--_fg));--_l:var(--ondine-line,var(--_fg));--_s:var(--ondine-surface,var(--_bg));--_a:var(--ondine-surface-alt,color-mix(in srgb,var(--_fg) 20%,var(--_bg)));--_b:var(--ondine-border,var(--_fg));--_c:var(--ondine-accent,var(--_fg));--_sw:var(--ondine-stroke,2px);stroke-width:var(--_sw);color:var(--_fg);--_tw:var(--ondine-title-weight,bold);--_hw:var(--ondine-heading-weight,bold);--_fm:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font:16px monospace;letter-spacing:normal;word-spacing:normal;text-transform:none;direction:ltr;font-family:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font-kerning:none;font-variant-ligatures:none;text-rendering:geometricPrecision\" fill=\"var(--_fg)\" role=\"graphics-document document\" aria-roledescription=\"pie\" aria-describedby=\"ondine-desc\"><desc id=\"ondine-desc\">only desc</desc><g><text font-weight=\"var(--_tw)\">t</text><text font-weight=\"var(--_hw)\">h</text></g><rect width=\"1\"/></svg>",
        ),
        (
            "sequence",
            0.0,
            0.0,
            12.5,
            Some(""),
            None,
            None,
            None,
            "",
            "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"ondine ondine-sequence\" viewBox=\"0 0 0 0\" width=\"0\" height=\"0\" style=\"max-width:100%;height:auto;--_bg:var(--ondine-bg,#fff);--_fg:var(--ondine-fg,#000);--_m:var(--ondine-muted,var(--_fg));--_l:var(--ondine-line,var(--_fg));--_s:var(--ondine-surface,var(--_bg));--_a:var(--ondine-surface-alt,color-mix(in srgb,var(--_fg) 20%,var(--_bg)));--_b:var(--ondine-border,var(--_fg));--_c:var(--ondine-accent,var(--_fg));--_sw:var(--ondine-stroke,2px);stroke-width:var(--_sw);color:var(--_fg);--_fm:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font:12.5px monospace;letter-spacing:normal;word-spacing:normal;text-transform:none;direction:ltr;font-family:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font-kerning:none;font-variant-ligatures:none;text-rendering:geometricPrecision\" fill=\"var(--_fg)\" role=\"graphics-document document\" aria-roledescription=\"sequence\"></svg>",
        ),
        (
            "x",
            1.0,
            2.0,
            16.0,
            None,
            None,
            None,
            None,
            "<rect rx=\"var(--ondine-radius,0)\">",
            "<svg xmlns=\"http://www.w3.org/2000/svg\" class=\"ondine ondine-x\" viewBox=\"0 0 1 2\" width=\"1\" height=\"2\" style=\"max-width:100%;height:auto;--_bg:var(--ondine-bg,#fff);--_fg:var(--ondine-fg,#000);--_m:var(--ondine-muted,var(--_fg));--_l:var(--ondine-line,var(--_fg));--_s:var(--ondine-surface,var(--_bg));--_a:var(--ondine-surface-alt,color-mix(in srgb,var(--_fg) 20%,var(--_bg)));--_b:var(--ondine-border,var(--_fg));--_c:var(--ondine-accent,var(--_fg));--_sw:var(--ondine-stroke,2px);stroke-width:var(--_sw);color:var(--_fg);--_r:var(--ondine-radius,0);--_fm:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font:16px monospace;letter-spacing:normal;word-spacing:normal;text-transform:none;direction:ltr;font-family:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font-kerning:none;font-variant-ligatures:none;text-rendering:geometricPrecision\" fill=\"var(--_fg)\" role=\"graphics-document document\" aria-roledescription=\"x\"><rect style=\"rx:var(--_r)\"></svg>",
        ),
    ];

    #[test]
    fn svg_document_matches_pele() {
        for &(
            r#type,
            width,
            height,
            size,
            id_prefix,
            responsive,
            acc_title,
            acc_descr,
            content,
            expected,
        ) in CASES
        {
            let options = RenderOptions {
                id_prefix: id_prefix.map(str::to_owned),
                responsive,
                ..RenderOptions::default()
            };
            let acc = Accessible {
                acc_descr,
                acc_title,
            };
            assert_eq!(
                svg_document(r#type, width, height, size, &options, acc, content),
                expected
            );
        }
    }
}
