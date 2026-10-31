//! Renders diagrams with the `theme` option, which writes the theme into the SVG as literal
//! values for renderers without CSS custom properties or `color-mix()`.

use std::fmt::Write;

/// Renders `source` with `options` and returns the SVG.
fn svg(source: &str, options: &str) -> String {
    compiler::render(source, options, 0.0)
        .expect("the source renders")
        .svg
}

/// The value of the first `name` attribute after `marker` in `svg`.
fn attribute<'a>(svg: &'a str, marker: &str, name: &str) -> &'a str {
    let rest = &svg[svg.find(marker).expect("the marker is present")..];
    let start = rest
        .find(&format!(" {name}=\""))
        .expect("the attribute is present")
        + name.len()
        + 3;
    &rest[start..start + rest[start..].find('"').expect("the value ends")]
}

#[test]
fn render_writes_default_theme() {
    insta::assert_snapshot!(svg("flowchart LR\n  A --> B", r#"{"theme":{}}"#), @r##"<svg xmlns="http://www.w3.org/2000/svg" class="ondine ondine-flowchart" viewBox="0 0 160 64" width="160" height="64" style="max-width:100%;height:auto;stroke-width:2px;color:#000;font:16px monospace;letter-spacing:normal;word-spacing:normal;text-transform:none;direction:ltr;font-family:ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace;font-kerning:none;font-variant-ligatures:none;text-rendering:geometricPrecision" fill="#000" role="graphics-document document" aria-roledescription="flowchart"><g class="ondine-edges" fill="none" stroke="#000" stroke-linecap="round"><g class="ondine-edge" data-id="L_A_B_0"><path d="M56,32L97,32"/><path class="ondine-marker" d="M104,32L96,35.36L96,28.64Z" fill="#000" stroke="none"/></g></g><g class="ondine-nodes"><g class="ondine-node ondine-shape-rect" data-id="A" transform="translate(32,32)"><rect x="-24" y="-24" width="48" height="48" rx="0" fill="#fff" stroke="#000"/><text class="ondine-label" x="0" y="5.6" text-anchor="middle">A</text></g><g class="ondine-node ondine-shape-rect" data-id="B" transform="translate(128,32)"><rect x="-24" y="-24" width="48" height="48" rx="0" fill="#fff" stroke="#000"/><text class="ondine-label" x="0" y="5.6" text-anchor="middle">B</text></g></g></svg>"##);
}

#[test]
fn render_writes_host_theme() {
    let options = r##"{"theme":{"bg":"#101010","border":"#555","fg":"#eee","font":"'Inter', sans-serif","radius":"6px","stroke":"1.5px","surface":"#202020"}}"##;

    insta::assert_snapshot!(svg("flowchart LR\n  A[\"var(--ondine-bg)\"] --> B", options), @r##"<svg xmlns="http://www.w3.org/2000/svg" class="ondine ondine-flowchart" viewBox="0 0 302 64" width="302" height="64" style="max-width:100%;height:auto;stroke-width:1.5px;color:#eee;font:16px monospace;letter-spacing:normal;word-spacing:normal;text-transform:none;direction:ltr;font-family:'Inter', sans-serif;font-kerning:none;font-variant-ligatures:none;text-rendering:geometricPrecision" fill="#eee" role="graphics-document document" aria-roledescription="flowchart"><g class="ondine-edges" fill="none" stroke="#eee" stroke-linecap="round"><g class="ondine-edge" data-id="L_A_B_0"><path d="M197.6,32L238.6,32"/><path class="ondine-marker" d="M245.6,32L237.6,35.36L237.6,28.64Z" fill="#eee" stroke="none"/></g></g><g class="ondine-nodes"><g class="ondine-node ondine-shape-rect" data-id="A" transform="translate(102.8,32)"><rect x="-94.8" y="-24" width="189.6" height="48" rx="6px" fill="#202020" stroke="#555"/><text class="ondine-label" x="0" y="5.6" text-anchor="middle">var(--ondine-bg)</text></g><g class="ondine-node ondine-shape-rect" data-id="B" transform="translate(269.6,32)"><rect x="-24" y="-24" width="48" height="48" rx="6px" fill="#202020" stroke="#555"/><text class="ondine-label" x="0" y="5.6" text-anchor="middle">B</text></g></g></svg>"##);
}

#[test]
fn render_mixes_series_colors() {
    insta::assert_snapshot!(svg("pie\n  \"A\" : 3\n  \"B\" : 2\n  \"C\" : 1", r#"{"theme":{}}"#), @r##"<svg xmlns="http://www.w3.org/2000/svg" class="ondine ondine-pie" viewBox="0 0 317 256" width="317" height="256" style="max-width:100%;height:auto;stroke-width:2px;color:#000;font:16px monospace;letter-spacing:normal;word-spacing:normal;text-transform:none;direction:ltr;font-family:ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace;font-kerning:none;font-variant-ligatures:none;text-rendering:geometricPrecision" fill="#000" role="graphics-document document" aria-roledescription="pie"><g class="ondine-slices" transform="translate(128,128)" stroke="#fff" stroke-width="1" stroke-linejoin="round"><path class="ondine-slice" data-id="A" d="M0,0L0,-120A120,120 0 0 1 0,120Z" fill="#000000"/><path class="ondine-slice" data-id="B" d="M0,0L0,120A120,120 0 0 1 -103.92,-60Z" fill="#6e6e6e"/><path class="ondine-slice" data-id="C" d="M0,0L-103.92,-60A120,120 0 0 1 0,-120Z" fill="#242424"/></g><g class="ondine-slice-labels" transform="translate(128,128)" font-size="14" text-anchor="middle" fill="#fff"><text x="90" y="4.9">50%</text><text x="-77.94" y="49.9">33%</text><text x="-45" y="-73.04">17%</text></g><g class="ondine-legend" font-size="14"><g class="ondine-legend-item" data-id="A"><rect x="280" y="96" width="12" height="12" rx="2" fill="#000000"/><text x="304.2" y="106.9" text-anchor="middle">A</text></g><g class="ondine-legend-item" data-id="B"><rect x="280" y="122" width="12" height="12" rx="2" fill="#6e6e6e"/><text x="304.2" y="132.9" text-anchor="middle">B</text></g><g class="ondine-legend-item" data-id="C"><rect x="280" y="148" width="12" height="12" rx="2" fill="#242424"/><text x="304.2" y="158.9" text-anchor="middle">C</text></g></g></svg>"##);
}

#[test]
fn render_leaves_no_custom_properties() {
    let sources = [
        "classDiagram\n  class Account {\n    +deposit(amount)\n  }\n  Account <|-- Savings",
        "flowchart TD\n  subgraph S\n    A --> B\n  end\n  B --> C",
        "gantt\n  dateFormat YYYY-MM-DD\n  section Work\n  Done :done, a, 2024-01-01, 2d\n  Next :crit, b, after a, 1d",
        "pie title Share\n  \"A\" : 3\n  \"B\" : 2",
        "sequenceDiagram\n  Alice->>Bob: Hi\n  Note over Alice,Bob: Greeting\n  Bob-->>Alice: Hello",
        "stateDiagram-v2\n  [*] --> Idle\n  Idle --> Busy\n  Busy --> [*]",
        "xychart-beta\n  x-axis [a, b, c]\n  bar [1, 2, 3]\n  line [3, 1, 2]\n  line [2, 2, 1]",
    ];

    let mut report = String::new();
    for source in sources {
        let rendered =
            compiler::render(source, r#"{"theme":{}}"#, 0.0).expect("the source renders");
        writeln!(
            report,
            "{}: var {}, color-mix {}, aliases {}",
            rendered.r#type,
            rendered.svg.contains("var("),
            rendered.svg.contains("color-mix("),
            rendered.svg.contains("--_"),
        )
        .expect("the report is a string");
    }

    insta::assert_snapshot!(report, @"
    class: var false, color-mix false, aliases false
    flowchart: var false, color-mix false, aliases false
    gantt: var false, color-mix false, aliases false
    pie: var false, color-mix false, aliases false
    sequence: var false, color-mix false, aliases false
    state: var false, color-mix false, aliases false
    xychart: var false, color-mix false, aliases false
    ");
}

#[test]
fn render_mixes_host_colors() {
    let source = "xychart-beta\n  x-axis [a, b]\n  bar [1, 2]\n  bar [2, 1]";
    let mut report = String::new();
    for fg in [
        "#336699",
        "#36c",
        "#33669980",
        "rgb(51 102 153)",
        "rgba(51, 102, 153, 0.5)",
        "transparent",
        "red",
        "url(#x)",
    ] {
        let options = format!(r##"{{"theme":{{"bg":"#ffffff","fg":"{fg}"}}}}"##);
        let svg = svg(source, &options);
        writeln!(report, "{fg}: {}", attribute(&svg, "ondine-bar", "fill"))
            .expect("the report is a string");
    }

    insta::assert_snapshot!(report, @"
    #336699: #336699
    #36c: #3366cc
    #33669980: #33669980
    rgb(51 102 153): rgb(51 102 153)
    rgba(51, 102, 153, 0.5): rgba(51, 102, 153, 0.5)
    transparent: #00000000
    red: red
    url(#x): #000000
    ");
}

#[test]
fn render_normalizes_mix_percentages() {
    let source = "flowchart TD\n  subgraph S\n    A\n  end";
    let mut report = String::new();
    for alt in [
        "color-mix(in srgb, #ff0000 30%, #0000ff 30%)",
        "color-mix(in srgb, #ff0000 80%, #0000ff 80%)",
        "color-mix(in srgb, #ff0000, #0000ff)",
        "color-mix(in srgb, #ff0000, #0000ff 25%)",
        "color-mix(in srgb, #ff000000 50%, #0000ff)",
        "color-mix(in srgb, color-mix(in srgb, #000 50%, #fff) 50%, #fff)",
        "color-mix(in oklch, #ff0000 50%, #0000ff)",
    ] {
        let options = format!(r#"{{"theme":{{"surfaceAlt":"{alt}"}}}}"#);
        let svg = svg(source, &options);
        writeln!(
            report,
            "{alt}: {}",
            attribute(&svg, "ondine-cluster", "fill")
        )
        .expect("the report is a string");
    }

    insta::assert_snapshot!(report, @"
    color-mix(in srgb, #ff0000 30%, #0000ff 30%): #80008099
    color-mix(in srgb, #ff0000 80%, #0000ff 80%): #800080
    color-mix(in srgb, #ff0000, #0000ff): #800080
    color-mix(in srgb, #ff0000, #0000ff 25%): #bf0040
    color-mix(in srgb, #ff000000 50%, #0000ff): #0000ff80
    color-mix(in srgb, color-mix(in srgb, #000 50%, #fff) 50%, #fff): #c0c0c0
    color-mix(in oklch, #ff0000 50%, #0000ff): #ff0000
    ");
}
