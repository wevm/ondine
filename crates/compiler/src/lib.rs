//! Compiles Mermaid source to monochrome SVG.
//!
//! The pipeline follows Pele (MIT): preprocessing removes front matter, directives, and comments,
//! detection picks a diagram type, and each type parses into a model, lays it out, and emits SVG.
#![expect(
    dead_code,
    reason = "Shared helpers are ported ahead of the diagram types that use them."
)]

mod detect;
mod error;
#[cfg(feature = "model")]
mod model;
mod preprocess;
mod yaml;

mod layout {
    pub(crate) mod compound;
    pub(crate) mod lanes;
    pub(crate) mod layered;
}

mod svg {
    pub(crate) mod builder;
    pub(crate) mod cluster;
    pub(crate) mod edges;
    #[cfg(feature = "theme")]
    pub(crate) mod resolve;
    pub(crate) mod root;
    pub(crate) mod shapes;
    pub(crate) mod theme;
    pub(crate) mod title;
}

mod text {
    pub(crate) mod entities;
    pub(crate) mod icons;
    pub(crate) mod label;
    pub(crate) mod measure;
}

mod util {
    pub(crate) mod chars;
    pub(crate) mod float;
    pub(crate) mod unicode;
    pub(crate) mod url;
}

mod diagrams;

pub use error::{Code, Error};
#[cfg(feature = "model")]
pub use model::{Model, model};

/// A link drawn in a diagram, as Pele's `LinkInfo` reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// The address.
    pub href: String,
    /// The id of the element that carries the link.
    pub id: String,
    /// Whether the address points inside the page.
    pub internal: bool,
}

/// A rendered diagram.
#[derive(Clone, Debug, PartialEq)]
pub struct Rendered {
    /// Height in pixels.
    pub height: f64,
    /// Links in drawing order.
    pub links: Vec<Link>,
    /// The SVG document.
    pub svg: String,
    /// The detected diagram type, using Pele's names.
    pub r#type: &'static str,
    /// Width in pixels.
    pub width: f64,
}

/// Mermaid's default `maxTextSize`.
const DEFAULT_LIMIT: f64 = 50_000.0;
/// A source within the limit can still describe megabytes of drawing, which would stall a page.
const DEFAULT_OUTPUT_LIMIT: f64 = 4_000_000.0;
/// `A & B --> C & D` multiplies edges, so a few kilobytes of source can ask for millions.
const DEFAULT_MAX_EDGES: f64 = 5_000.0;

/// Renders Mermaid `source` to SVG. `options` is a JSON object with Pele's `RenderOptions`
/// fields, or empty for defaults. `now` is the time a gantt chart schedules against when
/// `options` sets none, in milliseconds since the epoch. A non-finite `now` means the epoch.
///
/// # Errors
///
/// Returns an [`Error`] for unsupported diagram types, syntax errors, and exceeded limits.
pub fn render(source: &str, options: &str, now: f64) -> Result<Rendered, Error> {
    let mut options = options_from_json(options)?;
    if options.now.is_none() && now.is_finite() {
        options.now = Some(now);
    }

    let limit = options.limit.unwrap_or(DEFAULT_LIMIT);
    if exceeds(source, limit) {
        let mut message = String::from("Diagram source is longer than the limit of ");
        yaml::push_number(&mut message, limit);
        message.push_str(" characters.");
        return Err(Error::new(Code::Limit, message));
    }

    let pre = preprocess::preprocess(source)?;
    let Some(r#type) = detect::detect(&pre.text) else {
        return Err(Error::new(
            Code::UnsupportedDiagram,
            "No diagram type detected for the given text.",
        ));
    };
    let config = yaml::Value::Object(pre.config);

    let Some(diagram) = diagrams::find(r#type) else {
        let mut message = String::from("Diagram type \"");
        message.push_str(r#type);
        message.push_str("\" is not supported yet.");
        let mut error = Error::new(Code::UnsupportedDiagram, message);
        error.r#type = Some(r#type);
        return Err(error);
    };

    // A diagram whose config section sets `useMaxWidth: false` keeps its natural size.
    let section = match &config {
        yaml::Value::Object(entries) => entries
            .iter()
            .find(|(key, _)| key == diagram.section.unwrap_or(r#type))
            .map(|(_, value)| value),
        _ => None,
    };
    let fixed = matches!(
        section,
        Some(yaml::Value::Object(entries))
            if entries.iter().any(|(key, value)| key == "useMaxWidth" && *value == yaml::Value::Bool(false))
    );
    options.responsive = Some(options.responsive.unwrap_or(!fixed));

    let mut source = preprocess::encode_entities(if diagram.keep_comments {
        &pre.with_comments
    } else {
        &pre.text
    })
    .into_owned();
    source.push('\n');

    let max_edges = options.max_edges.unwrap_or(DEFAULT_MAX_EDGES);
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a negative or fractional limit saturates, as a count comparison would"
    )]
    let max_edges = max_edges as usize;
    let input = diagrams::Input {
        config: &config,
        line_offset: pre.line_offset,
        max_edges,
        source: &source,
        title: pre.title.as_deref(),
    };

    let link_schemes = schemes(options.link_schemes.as_ref());
    let image_schemes = schemes(options.image_schemes.as_ref());
    let url = util::url::UrlOptions {
        image_schemes: image_schemes.as_deref(),
        images: options.images,
        link_rel: options.link_rel.as_deref(),
        link_schemes: link_schemes.as_deref(),
        links: options.links,
    };

    let drawn = (diagram.render)(&input, &options, &url)?;
    #[cfg(feature = "theme")]
    let document = match &options.theme {
        Some(theme) => svg::resolve::resolve(&drawn.svg, theme)?,
        None => drawn.svg,
    };
    #[cfg(not(feature = "theme"))]
    let document = drawn.svg;
    let mut rendered = Rendered {
        height: drawn.height,
        links: drawn
            .links
            .into_iter()
            .map(|link| Link {
                href: link.href,
                id: link.id,
                internal: link.internal,
            })
            .collect(),
        svg: document,
        r#type,
        width: drawn.width,
    };

    let output_limit = options.output_limit.unwrap_or(DEFAULT_OUTPUT_LIMIT);
    if exceeds(&rendered.svg, output_limit) {
        let mut message = String::from("Diagram output is longer than the limit of ");
        yaml::push_number(&mut message, output_limit);
        message.push_str(" characters.");
        let mut error = Error::new(Code::Limit, message);
        error.r#type = Some(r#type);
        return Err(error);
    }

    // Hosts see link ids as the author meant them, with entity codes turned into characters.
    for link in &mut rendered.links {
        if let std::borrow::Cow::Owned(decoded) = text::entities::decode(&link.id) {
            link.id = decoded;
        }
    }

    Ok(rendered)
}

fn schemes(list: Option<&Vec<String>>) -> Option<Vec<&str>> {
    list.map(|list| list.iter().map(String::as_str).collect())
}

/// Whether `text` is longer than `limit` UTF-16 units.
#[expect(
    clippy::cast_precision_loss,
    reason = "source lengths stay far below 2^52"
)]
fn exceeds(text: &str, limit: f64) -> bool {
    // No scalar takes more UTF-16 units than UTF-8 bytes, so text within the limit in bytes is
    // within it in units without a count.
    if text.len() as f64 <= limit {
        return false;
    }

    // Each scalar starts with one non-continuation byte, and only four-byte scalars need a second
    // UTF-16 unit. Counting bytes avoids decoding the whole SVG.
    let units = text
        .bytes()
        .map(|byte| usize::from(byte & 0xc0 != 0x80) + usize::from(byte >= 0xf0))
        .sum::<usize>();
    units as f64 > limit
}

/// Reads host options from JSON. Unknown fields are ignored, as in Pele.
fn options_from_json(json: &str) -> Result<svg::root::RenderOptions, Error> {
    let mut options = svg::root::RenderOptions::default();
    if json.trim().is_empty() {
        return Ok(options);
    }

    let value = yaml::parse(json).map_err(|message| Error::new(Code::Limit, message))?;
    let yaml::Value::Object(entries) = value else {
        return Ok(options);
    };

    let number = |value: &yaml::Value| match value {
        yaml::Value::Number(n) if n.is_finite() => Some(*n),
        _ => None,
    };
    let boolean = |value: &yaml::Value| match value {
        yaml::Value::Bool(b) => Some(*b),
        _ => None,
    };
    let string = |value: &yaml::Value| match value {
        yaml::Value::String(s) => Some(s.clone()),
        _ => None,
    };
    let strings = |value: &yaml::Value| match value {
        yaml::Value::Array(items) => Some(
            items
                .iter()
                .filter_map(|item| match item {
                    yaml::Value::String(s) => Some(s.clone()),
                    _ => None,
                })
                .collect(),
        ),
        _ => None,
    };

    for (key, value) in &entries {
        match key.as_str() {
            "autoDirection" => options.auto_direction = boolean(value),
            "directionBreakpoint" => options.direction_breakpoint = number(value),
            "fontSize" => options.font_size = number(value),
            "idPrefix" => options.id_prefix = string(value),
            "imageSchemes" => options.image_schemes = strings(value),
            "images" => options.images = boolean(value),
            "limit" => options.limit = number(value),
            "linkRel" => options.link_rel = string(value),
            "linkSchemes" => options.link_schemes = strings(value),
            "links" => options.links = boolean(value),
            "maxEdges" => options.max_edges = number(value),
            "maxWidth" => options.max_width = number(value),
            "now" => options.now = number(value),
            "outputLimit" => options.output_limit = number(value),
            "padding" => options.padding = number(value),
            "responsive" => options.responsive = boolean(value),
            #[cfg(feature = "theme")]
            "theme" => options.theme = svg::resolve::properties(value),
            #[cfg(not(feature = "theme"))]
            "theme" => {
                return Err(Error::new(
                    Code::Unsupported,
                    "This module does not support the theme option. Use ondine/compiler.",
                ));
            }
            _ => {}
        }
    }

    Ok(options)
}
