//! Diagram types and the contract each one implements.
//!
//! Every type is a Cargo feature, so each WebAssembly module contains only the types it renders.

pub(crate) mod common {
    pub(crate) mod acc_descr;
    pub(crate) mod caseless;
    pub(crate) mod fit;
    pub(crate) mod fit_text;
    pub(crate) mod fit_width;
    pub(crate) mod generics;
    pub(crate) mod icon;
    pub(crate) mod literal_label;
    pub(crate) mod outline;
    pub(crate) mod scan;
    pub(crate) mod tokens;
}

#[cfg(feature = "agentflow")]
pub(crate) mod agentflow;
#[cfg(feature = "architecture")]
pub(crate) mod architecture;
#[cfg(feature = "block")]
pub(crate) mod block;
#[cfg(feature = "c4")]
pub(crate) mod c4;
#[cfg(feature = "class")]
pub(crate) mod class;
#[cfg(feature = "cynefin")]
pub(crate) mod cynefin;
#[cfg(feature = "er")]
pub(crate) mod er;
#[cfg(feature = "eventmodeling")]
pub(crate) mod eventmodeling;
/// Swimlane diagrams draw through the flowchart, and agentflow reads its shapes.
#[cfg(any(feature = "agentflow", feature = "flowchart", feature = "swimlane"))]
pub(crate) mod flowchart;
#[cfg(feature = "gantt")]
pub(crate) mod gantt;
#[cfg(feature = "git")]
pub(crate) mod git;
#[cfg(feature = "info")]
pub(crate) mod info;
#[cfg(feature = "ishikawa")]
pub(crate) mod ishikawa;
#[cfg(feature = "journey")]
pub(crate) mod journey;
#[cfg(feature = "kanban")]
pub(crate) mod kanban;
#[cfg(feature = "mindmap")]
pub(crate) mod mindmap;
#[cfg(feature = "packet")]
pub(crate) mod packet;
#[cfg(feature = "pie")]
pub(crate) mod pie;
#[cfg(feature = "quadrant")]
pub(crate) mod quadrant;
#[cfg(feature = "radar")]
pub(crate) mod radar;
#[cfg(feature = "railroad")]
pub(crate) mod railroad;
#[cfg(feature = "requirement")]
pub(crate) mod requirement;
#[cfg(feature = "sankey")]
pub(crate) mod sankey;
#[cfg(feature = "sequence")]
pub(crate) mod sequence;
#[cfg(feature = "state")]
pub(crate) mod state;
#[cfg(feature = "swimlane")]
pub(crate) mod swimlane;
#[cfg(feature = "timeline")]
pub(crate) mod timeline;
#[cfg(feature = "treemap")]
pub(crate) mod treemap;
#[cfg(feature = "treeview")]
pub(crate) mod treeview;
#[cfg(feature = "usecase")]
pub(crate) mod usecase;
#[cfg(feature = "venn")]
pub(crate) mod venn;
#[cfg(feature = "wardley")]
pub(crate) mod wardley;
#[cfg(feature = "xychart")]
pub(crate) mod xychart;

use crate::error::Error;
use crate::lines::Lines;
use crate::svg::root::RenderOptions;
use crate::util::url::{LinkInfo, UrlOptions};
use crate::yaml::Value;

/// What a diagram type receives, as Pele's `Diagram.parse` does.
pub(crate) struct Input<'a> {
    /// The diagram configuration from front matter and directives, as a YAML mapping.
    pub(crate) config: &'a Value,
    /// Front matter lines removed before `source`, for types that report source positions.
    pub(crate) line_offset: u32,
    /// The authored lines of `source`, when the host asks for `data-lines`.
    pub(crate) lines: Option<&'a Lines>,
    /// The most edges a flowchart-like diagram may have.
    pub(crate) max_edges: usize,
    /// Preprocessed, entity-encoded source ending in a newline. It keeps `%%` comment lines
    /// when the type sets `keep_comments`.
    pub(crate) source: &'a str,
    /// The front matter title.
    pub(crate) title: Option<&'a str>,
}

/// A drawn diagram.
#[derive(Clone, Debug)]
pub(crate) struct Rendered {
    /// Height in pixels.
    pub(crate) height: f64,
    /// Links in drawing order.
    pub(crate) links: Vec<LinkInfo>,
    /// The SVG document.
    pub(crate) svg: String,
    /// Width in pixels.
    pub(crate) width: f64,
}

impl common::fit_width::Drawn for Rendered {
    fn width(&self) -> f64 {
        self.width
    }
}

/// A diagram type, as Pele's `Diagram` describes one.
#[derive(Clone, Copy)]
pub(crate) struct Diagram {
    /// Whether the type parses source with its `%%` comment lines kept.
    pub(crate) keep_comments: bool,
    /// Parses and draws the diagram.
    pub(crate) render: fn(&Input<'_>, &RenderOptions, &UrlOptions<'_>) -> Result<Rendered, Error>,
    /// The type's section in Mermaid's config, when it is not the type's own name.
    pub(crate) section: Option<&'static str>,
}

/// The diagram type named `type` (Pele's names), when this build includes it.
pub(crate) fn find(r#type: &str) -> Option<Diagram> {
    match r#type {
        #[cfg(feature = "agentflow")]
        "agentflow" => Some(agentflow::DIAGRAM),
        #[cfg(feature = "architecture")]
        "architecture" => Some(architecture::DIAGRAM),
        #[cfg(feature = "block")]
        "block" => Some(block::DIAGRAM),
        #[cfg(feature = "c4")]
        "c4" => Some(c4::DIAGRAM),
        #[cfg(feature = "class")]
        "class" => Some(class::DIAGRAM),
        #[cfg(feature = "cynefin")]
        "cynefin" => Some(cynefin::DIAGRAM),
        #[cfg(feature = "er")]
        "er" => Some(er::DIAGRAM),
        #[cfg(feature = "eventmodeling")]
        "eventmodeling" => Some(eventmodeling::DIAGRAM),
        #[cfg(feature = "flowchart")]
        "flowchart" => Some(flowchart::DIAGRAM),
        #[cfg(feature = "gantt")]
        "gantt" => Some(gantt::DIAGRAM),
        #[cfg(feature = "git")]
        "gitGraph" => Some(git::DIAGRAM),
        #[cfg(feature = "info")]
        "info" => Some(info::DIAGRAM),
        #[cfg(feature = "ishikawa")]
        "ishikawa" => Some(ishikawa::DIAGRAM),
        #[cfg(feature = "journey")]
        "journey" => Some(journey::DIAGRAM),
        #[cfg(feature = "kanban")]
        "kanban" => Some(kanban::DIAGRAM),
        #[cfg(feature = "mindmap")]
        "mindmap" => Some(mindmap::DIAGRAM),
        #[cfg(feature = "packet")]
        "packet" => Some(packet::DIAGRAM),
        #[cfg(feature = "pie")]
        "pie" => Some(pie::DIAGRAM),
        #[cfg(feature = "quadrant")]
        "quadrantChart" => Some(quadrant::DIAGRAM),
        #[cfg(feature = "radar")]
        "radar" => Some(radar::DIAGRAM),
        #[cfg(feature = "railroad")]
        "railroad" => Some(railroad::DIAGRAM),
        #[cfg(feature = "requirement")]
        "requirement" => Some(requirement::DIAGRAM),
        #[cfg(feature = "sankey")]
        "sankey" => Some(sankey::DIAGRAM),
        #[cfg(feature = "sequence")]
        "sequence" => Some(sequence::DIAGRAM),
        #[cfg(feature = "state")]
        "state" => Some(state::DIAGRAM),
        #[cfg(feature = "swimlane")]
        "swimlane" => Some(swimlane::DIAGRAM),
        #[cfg(feature = "timeline")]
        "timeline" => Some(timeline::DIAGRAM),
        #[cfg(feature = "treemap")]
        "treemap" => Some(treemap::DIAGRAM),
        #[cfg(feature = "treeview")]
        "treeView" => Some(treeview::DIAGRAM),
        #[cfg(feature = "usecase")]
        "usecase" => Some(usecase::DIAGRAM),
        #[cfg(feature = "venn")]
        "venn" => Some(venn::DIAGRAM),
        #[cfg(feature = "wardley")]
        "wardley" => Some(wardley::DIAGRAM),
        #[cfg(feature = "xychart")]
        "xychart" => Some(xychart::DIAGRAM),
        _ => None,
    }
}
