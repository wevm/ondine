//! Renders diagrams with a host-supplied text advance table through the public `render` function.

use insta::assert_snapshot;

/// A flowchart with one labeled node.
const SOURCE: &str = "flowchart LR\n  A[Mermaid text] --> B[Ondine]";

/// A JSON advance table of `entries` entries, each `advance` thousandths of an em.
fn table(advance: u32, entries: usize) -> String {
    format!("[{}]", vec![advance.to_string(); entries].join(","))
}

/// Render options with every printable ASCII advance set to `advance` thousandths of an em.
fn options(advance: u32, entries: usize) -> String {
    format!(
        r#"{{"textMetrics":{{"advances":{}}}}}"#,
        table(advance, entries)
    )
}

/// The outline of node `id` in `svg`.
fn outline<'a>(svg: &'a str, id: &str) -> &'a str {
    let marker = format!("data-id=\"{id}\"");
    let node = &svg[svg.find(&marker).expect("node")..];
    let rect = &node[node.find("<rect").expect("outline")..];
    &rect[..=rect.find("/>").expect("end") + 1]
}

#[test]
fn renders_uniform_advances_without_text_metrics() {
    let rendered = compiler::render(SOURCE, "", 0.0).expect("render");

    assert_snapshot!(outline(&rendered.svg, "A"), @r#"<rect x="-75.6" y="-24" width="151.2" height="48" fill="var(--_s)" stroke="var(--_b)" style="rx:var(--_r)"/>"#);
}

#[test]
fn renders_narrow_advances_from_text_metrics() {
    let rendered = compiler::render(SOURCE, &options(500, 95), 0.0).expect("render");

    assert_snapshot!(outline(&rendered.svg, "A"), @r#"<rect x="-66" y="-24" width="132" height="48" fill="var(--_s)" stroke="var(--_b)" style="rx:var(--_r)"/>"#);
    assert_snapshot!(rendered.width, @"280");
}

#[test]
fn renders_identical_bytes_for_a_uniform_table() {
    let plain = compiler::render(SOURCE, "", 0.0).expect("render");
    let uniform = compiler::render(SOURCE, &options(600, 95), 0.0).expect("render");

    assert_snapshot!(plain.svg == uniform.svg, @"true");
}

#[test]
fn ignores_text_metrics_with_the_wrong_length() {
    let plain = compiler::render(SOURCE, "", 0.0).expect("render");
    let short = compiler::render(SOURCE, &options(500, 94), 0.0).expect("render");

    assert_snapshot!(plain.svg == short.svg, @"true");
}

#[test]
fn renders_bold_advances_from_text_metrics() {
    let source = "classDiagram\n  class Animal";
    let regular = options(600, 95);
    let bold = format!(
        r#"{{"textMetrics":{{"advances":{},"bold":{}}}}}"#,
        table(600, 95),
        table(800, 95)
    );

    let plain = compiler::render(source, &regular, 0.0).expect("render");
    let wide = compiler::render(source, &bold, 0.0).expect("render");

    assert_snapshot!(plain.width, @"98");
    assert_snapshot!(wide.width, @"117");
}

#[test]
fn restores_uniform_advances_after_a_render() {
    compiler::render(SOURCE, &options(500, 95), 0.0).expect("render");
    let rendered = compiler::render(SOURCE, "", 0.0).expect("render");

    assert_snapshot!(outline(&rendered.svg, "A"), @r#"<rect x="-75.6" y="-24" width="151.2" height="48" fill="var(--_s)" stroke="var(--_b)" style="rx:var(--_r)"/>"#);
}
