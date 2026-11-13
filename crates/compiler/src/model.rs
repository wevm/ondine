//! Parsed models as JSON, for the tests that compare Ondine's parsers with Pele's and Mermaid's.
//! Release modules leave this out with the `model` feature.

use crate::error::{Code, Error};
use crate::{detect, diagrams, options_from_json, preprocess, yaml};

/// A parsed diagram.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Model {
    /// The model as `JSON.stringify` serializes Pele's model for the type.
    pub json: String,
    /// The detected diagram type, using Pele's names.
    pub r#type: &'static str,
}

/// Parses Mermaid `source` without drawing it. Preprocessing, detection, and the options that
/// reach parsing (`limit`, `maxEdges`, and `now`) work as in [`crate::render`].
///
/// # Errors
///
/// Returns an [`Error`] for unsupported diagram types, syntax errors, and exceeded limits. Errors
/// raised while drawing, such as a gantt chart that cannot be scheduled, are not reported.
pub fn model(source: &str, options: &str, now: f64) -> Result<Model, Error> {
    let options = options_from_json(options)?;
    let now = options
        .now
        .unwrap_or(if now.is_finite() { now } else { 0.0 });

    let limit = options.limit.unwrap_or(crate::DEFAULT_LIMIT);
    if crate::exceeds(source, limit) {
        let mut message = String::from("Diagram source is longer than the limit of ");
        yaml::push_number(&mut message, limit);
        message.push_str(" characters.");
        return Err(Error::new(Code::Limit, message));
    }

    let pre = preprocess::preprocess(source, false)?;
    let Some(r#type) = detect::detect(&pre.text) else {
        return Err(Error::new(
            Code::UnsupportedDiagram,
            "No diagram type detected for the given text.",
        ));
    };
    let Some(diagram) = diagrams::find(r#type) else {
        let mut error = Error::new(Code::UnsupportedDiagram, "Diagram type is not supported.");
        error.r#type = Some(r#type);
        return Err(error);
    };

    let config = yaml::Value::Object(pre.config);
    let mut text = preprocess::encode_entities(if diagram.keep_comments {
        &pre.with_comments
    } else {
        &pre.text
    })
    .into_owned();
    text.push('\n');

    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a negative or fractional limit saturates, as a count comparison would"
    )]
    let max_edges = options.max_edges.unwrap_or(crate::DEFAULT_MAX_EDGES) as usize;
    let input = diagrams::Input {
        config: &config,
        line_offset: pre.line_offset,
        lines: None,
        max_edges,
        source: &text,
        title: pre.title.as_deref(),
    };

    let json = to_json(r#type, &input, now).map_err(|mut error| {
        error.r#type = Some(r#type);
        error
    })?;
    Ok(Model { json, r#type })
}

/// Parses `input` as the diagram type `type` and serializes the model, with the arguments each
/// type's `render_input` passes to its parser.
#[cfg_attr(
    not(feature = "gantt"),
    expect(unused_variables, reason = "only gantt charts read the time")
)]
fn to_json(r#type: &str, input: &diagrams::Input<'_>, now: f64) -> Result<String, Error> {
    let source = input.source;
    let title = input.title;

    Ok(match r#type {
        #[cfg(feature = "agentflow")]
        "agentflow" => diagrams::agentflow::to_json(&diagrams::agentflow::parse(
            source,
            input.config,
            title,
            input.line_offset,
            Some(input.max_edges),
        )?),
        #[cfg(feature = "architecture")]
        "architecture" => {
            diagrams::architecture::to_json(&diagrams::architecture::parse(source, title)?)
        }
        #[cfg(feature = "block")]
        "block" => diagrams::block::to_json(&diagrams::block::parse(source, title)?),
        #[cfg(feature = "c4")]
        "c4" => diagrams::c4::to_json(&diagrams::c4::parse(source, title)?),
        #[cfg(feature = "class")]
        "class" => diagrams::class::to_json(&diagrams::class::parse(source, title)?),
        #[cfg(feature = "cynefin")]
        "cynefin" => diagrams::cynefin::to_json(&diagrams::cynefin::parse(source, title)?),
        #[cfg(feature = "er")]
        "er" => diagrams::er::to_json(&diagrams::er::parse(source, title)?),
        #[cfg(feature = "eventmodeling")]
        "eventmodeling" => {
            diagrams::eventmodeling::to_json(&diagrams::eventmodeling::parse(source, title)?)
        }
        // Swimlane diagrams parse as flowcharts.
        #[cfg(any(feature = "flowchart", feature = "swimlane"))]
        "flowchart" | "swimlane" => diagrams::flowchart::to_json(&diagrams::flowchart::parse(
            source,
            input.lines,
            input.config,
            title,
            Some(input.max_edges),
        )?),
        #[cfg(feature = "gantt")]
        "gantt" => {
            diagrams::gantt::to_json(&diagrams::gantt::parse(source, input.config, title)?, now)
        }
        #[cfg(feature = "git")]
        "gitGraph" => diagrams::git::to_json(&diagrams::git::parse(source, input.config, title)?),
        #[cfg(feature = "info")]
        "info" => diagrams::info::to_json(&diagrams::info::parse(source, title)?),
        #[cfg(feature = "ishikawa")]
        "ishikawa" => diagrams::ishikawa::to_json(&diagrams::ishikawa::parse(source, title)?),
        #[cfg(feature = "journey")]
        "journey" => diagrams::journey::to_json(&diagrams::journey::parse(source, title)?),
        #[cfg(feature = "kanban")]
        "kanban" => diagrams::kanban::to_json(&diagrams::kanban::parse(source, title)?),
        #[cfg(feature = "mindmap")]
        "mindmap" => diagrams::mindmap::to_json(&diagrams::mindmap::parse(source, title)?),
        #[cfg(feature = "packet")]
        "packet" => {
            diagrams::packet::to_json(&diagrams::packet::parse(source, input.config, title)?)
        }
        #[cfg(feature = "pie")]
        "pie" => diagrams::pie::to_json(&diagrams::pie::parse(source, title)?),
        #[cfg(feature = "quadrant")]
        "quadrantChart" => diagrams::quadrant::to_json(&diagrams::quadrant::parse(source, title)?),
        #[cfg(feature = "radar")]
        "radar" => diagrams::radar::to_json(&diagrams::radar::parse(source, title)?),
        #[cfg(feature = "railroad")]
        "railroad" => diagrams::railroad::to_json(&diagrams::railroad::parse(source, title)?),
        #[cfg(feature = "requirement")]
        "requirement" => {
            diagrams::requirement::to_json(&diagrams::requirement::parse(source, title)?)
        }
        #[cfg(feature = "sankey")]
        "sankey" => diagrams::sankey::to_json(&diagrams::sankey::parse(source, title)?),
        #[cfg(feature = "sequence")]
        "sequence" => {
            diagrams::sequence::to_json(&diagrams::sequence::parse(source, input.config, title)?)
        }
        #[cfg(feature = "state")]
        "state" => diagrams::state::to_json(&diagrams::state::parse(source, title)?),
        #[cfg(feature = "timeline")]
        "timeline" => diagrams::timeline::to_json(&diagrams::timeline::parse(source, title)?),
        #[cfg(feature = "treemap")]
        "treemap" => diagrams::treemap::to_json(&diagrams::treemap::parse(source, title)?),
        #[cfg(feature = "treeview")]
        "treeView" => diagrams::treeview::to_json(&diagrams::treeview::parse(source, title)?),
        #[cfg(feature = "usecase")]
        "usecase" => diagrams::usecase::to_json(&diagrams::usecase::parse(source, title)?),
        #[cfg(feature = "venn")]
        "venn" => diagrams::venn::to_json(&diagrams::venn::parse(source, title)?),
        #[cfg(feature = "wardley")]
        "wardley" => diagrams::wardley::to_json(&diagrams::wardley::parse(source, title)?),
        #[cfg(feature = "xychart")]
        "xychart" => diagrams::xychart::to_json(&diagrams::xychart::parse(source, title)?),
        _ => {
            return Err(Error::new(
                Code::UnsupportedDiagram,
                "Diagram type is not supported.",
            ));
        }
    })
}
