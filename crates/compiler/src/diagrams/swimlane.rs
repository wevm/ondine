//! Swimlane diagrams: flowcharts whose top-level subgraphs are lanes, with the same syntax and
//! model. Port of Pele's `src/diagrams/swimlane/index.ts`.

/// The JSON reader of the parity fixtures, loaded once per build.
#[cfg(all(test, not(feature = "sankey")))]
#[path = "../../tests/fixtures/json.rs"]
mod fixture;
#[cfg(all(test, feature = "sankey"))]
use super::sankey::fixture;

use super::flowchart::graph::{FlowGraph, GraphNode};
use super::flowchart::render::{self, Variant};
use super::flowchart::types::LabelType;
use super::{Diagram, Input, Rendered};
use crate::error::Error;
use crate::layout::compound::{CEdge, CNode, CompoundResult, Dir};
use crate::layout::lanes::{LANE_PAD, lane_layout};
use crate::layout::layered::LayeredOptions;
use crate::svg::builder::{label_svg, push_num};
use crate::svg::root::RenderOptions;
use crate::svg::theme::ResolvedStyle;
use crate::text::label::Label;
use crate::util::url::UrlOptions;

/// The `swimlane` diagram type.
pub(crate) const DIAGRAM: Diagram = Diagram {
    keep_comments: false,
    render: render_input,
    section: None,
};

/// The lane that holds nodes outside every lane.
const DEFAULT_LANE: &str = "__swimlane_default__";

/// The flowchart variant that draws lanes.
const VARIANT: Variant = Variant {
    draw_group: Some(draw_lane),
    layout,
    prepare,
    r#type: "swimlane",
};

fn render_input(
    input: &Input<'_>,
    options: &RenderOptions,
    url: &UrlOptions<'_>,
) -> Result<Rendered, Error> {
    let db = super::flowchart::parse(
        input.source,
        input.lines,
        input.config,
        input.title,
        Some(input.max_edges),
    )?;
    Ok(render::render(
        &db,
        input.config,
        options,
        url,
        Some(&VARIANT),
    ))
}

/// Nodes outside every lane share one unnamed lane, as in Mermaid. Lanes all run the same way, so
/// a `direction` inside a subgraph has no effect.
fn prepare(graph: &mut FlowGraph) {
    let mut loose = false;
    for node in &mut graph.nodes {
        if node.is_group {
            node.dir = None;
        } else if node.parent_id.is_none() {
            loose = true;
        }
    }
    if !loose {
        return;
    }

    for node in &mut graph.nodes {
        if !node.is_group && node.parent_id.is_none() {
            node.parent_id = Some(DEFAULT_LANE.to_owned());
        }
    }
    graph.nodes.push(GraphNode {
        asset_height: None,
        asset_width: None,
        constraint: None,
        css_classes: String::new(),
        css_compiled_styles: Vec::new(),
        css_styles: Vec::new(),
        dir: None,
        icon: None,
        id: DEFAULT_LANE.to_owned(),
        img: None,
        is_group: true,
        label: None,
        label_type: LabelType::Text,
        lines: Vec::new(),
        link: None,
        link_target: None,
        parent_id: None,
        pos: None,
        shape: "rect".to_owned(),
        tooltip: None,
    });
}

fn layout(
    nodes: &mut [CNode],
    edges: &mut [CEdge],
    dir: Dir,
    options: &LayeredOptions,
    tight: bool,
) -> CompoundResult {
    lane_layout(
        nodes,
        edges,
        dir,
        options,
        if tight { 12.0 } else { LANE_PAD },
    )
}

/// A lane: an outline the length of the diagram, with its title in a band at the start.
fn draw_lane(
    c: &CNode,
    label: &Label,
    style: &ResolvedStyle,
    classes: &str,
    id: &str,
    lines: &[u32],
    dir: Dir,
) -> String {
    let left = c.x - c.w / 2.0;
    let top = c.y - c.h / 2.0;
    let sideways = matches!(dir, Dir::Lr | Dir::Rl);
    let cx = if sideways {
        left + c.pad_top / 2.0
    } else {
        c.x
    };
    let cy = if sideways { c.y } else { top + c.pad_top / 2.0 };
    let mut attrs = String::from(" class=\"ondine-cluster-label\" fill=\"var(--_m)\"");
    attrs.push_str(&style.text);
    let text = label_svg(label, cx, cy, &attrs, None);

    let mut out = String::from("<g class=\"ondine-cluster ondine-lane");
    out.push_str(classes);
    out.push_str("\" data-id=\"");
    out.push_str(id);
    out.push('"');
    crate::lines::push_attribute(&mut out, lines);
    out.push_str("><rect x=\"");
    push_num(&mut out, left);
    out.push_str("\" y=\"");
    push_num(&mut out, top);
    out.push_str("\" width=\"");
    push_num(&mut out, c.w);
    out.push_str("\" height=\"");
    push_num(&mut out, c.h);
    out.push_str("\" fill=\"none\" stroke=\"var(--_b)\"/><rect x=\"");
    push_num(&mut out, left);
    out.push_str("\" y=\"");
    push_num(&mut out, top);
    out.push_str("\" width=\"");
    push_num(&mut out, if sideways { c.pad_top } else { c.w });
    out.push_str("\" height=\"");
    push_num(&mut out, if sideways { c.h } else { c.pad_top });
    out.push_str("\" fill=\"var(--_a)\" stroke=\"var(--_b)\"");
    out.push_str(&style.shape);
    out.push_str("/>");
    if sideways && !text.is_empty() {
        out.push_str("<g transform=\"rotate(-90 ");
        push_num(&mut out, cx);
        out.push(' ');
        push_num(&mut out, cy);
        out.push_str(")\">");
        out.push_str(&text);
        out.push_str("</g>");
    } else {
        out.push_str(&text);
    }
    out.push_str("</g>");
    out
}

#[cfg(test)]
mod tests {
    //! Parity with Pele's `swimlane.parse`, which is the flowchart's, over every swimlane vector,
    //! hand-written cases, and fuzzed variants, as recorded by running Pele's TypeScript.

    use super::fixture::{Json, cases};
    use crate::diagrams::flowchart::parse;
    use crate::yaml::Value;

    /// The parity fixture: Pele's parser input and its model JSON or error.
    const FIXTURE: &str = include_str!("../../tests/fixtures/swimlane-parse.json");

    #[test]
    fn matches_pele() {
        let mut differing = Vec::new();
        let cases = cases(FIXTURE);
        for case in &cases {
            let input = case.str("input").unwrap_or_default();
            let inherit = matches!(case.get("inheritDir"), Some(Json::Bool(true)));
            let config = Value::Object(vec![(
                "flowchart".to_owned(),
                Value::Object(vec![("inheritDir".to_owned(), Value::Bool(inherit))]),
            )]);
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                reason = "test data"
            )]
            let max_edges = case.num("maxEdges").map(|n| n as usize);
            let actual = parse(input, None, &config, case.str("title"), max_edges);
            let same = match (&actual, case.get("error")) {
                (Ok(_), None) => {
                    #[cfg(feature = "model")]
                    {
                        actual
                            .as_ref()
                            .map(crate::diagrams::flowchart::to_json)
                            .ok()
                            .as_deref()
                            == case.str("model")
                    }
                    #[cfg(not(feature = "model"))]
                    {
                        true
                    }
                }
                (Err(error), Some(expected)) => {
                    Some(error.code.as_str()) == expected.str("code")
                        && Some(error.message.as_str()) == expected.str("message")
                        && Some(f64::from(error.line)) == expected.num("line")
                        && Some(f64::from(error.column)) == expected.num("column")
                }
                _ => false,
            };
            if !same {
                differing.push((case.str("origin").unwrap_or("?"), input, actual));
            }
        }
        assert!(
            differing.is_empty(),
            "{} of {} cases differ from Pele: {:#?}",
            differing.len(),
            cases.len(),
            differing.iter().take(10).collect::<Vec<_>>()
        );
    }
}
