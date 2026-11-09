/** @module Integration tests for rendering Mermaid source through the WebAssembly compiler. */
import * as Fs from 'node:fs'
import { Render, render, RenderError } from 'ondine/compiler'

describe('render', () => {
  test('renders a flowchart', () => {
    const result = render(
      'flowchart LR\n  A[Mermaid text] --> B{Supported?}\n  B -- Yes --> C[Ondine]',
    )

    expect(result.type).toMatchInlineSnapshot(`"flowchart"`)
    expect([result.width, result.height]).toMatchInlineSnapshot(`
      [
        558,
        118,
      ]
    `)
    expect(result.svg).toMatchInlineSnapshot(
      `"<svg xmlns="http://www.w3.org/2000/svg" class="ondine ondine-flowchart" viewBox="0 0 558 118" width="558" height="118" style="max-width:100%;height:auto;--_bg:var(--ondine-bg,#fff);--_fg:var(--ondine-fg,#000);--_m:var(--ondine-muted,var(--_fg));--_l:var(--ondine-line,var(--_fg));--_s:var(--ondine-surface,var(--_bg));--_a:var(--ondine-surface-alt,color-mix(in srgb,var(--_fg) 20%,var(--_bg)));--_b:var(--ondine-border,var(--_fg));--_c:var(--ondine-accent,var(--_fg));--_sw:var(--ondine-stroke,2px);stroke-width:var(--_sw);color:var(--_fg);--_r:var(--ondine-radius,0);--_fm:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font:16px monospace;letter-spacing:normal;word-spacing:normal;text-transform:none;direction:ltr;font-family:var(--ondine-font,ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace);font-kerning:none;font-variant-ligatures:none;text-rendering:geometricPrecision" fill="var(--_fg)" role="graphics-document document" aria-roledescription="flowchart"><g class="ondine-edges" fill="none" stroke="var(--_l)" stroke-linecap="round"><g class="ondine-edge" data-id="L_A_B_0"><path d="M159.2,59L200.2,59"/><path class="ondine-marker" d="M207.2,59L199.2,62.36L199.2,55.64Z" fill="var(--_l)" stroke="none"/></g><g class="ondine-edge" data-id="L_B_C_0"><path d="M370.4,59L448.6,59"/><path class="ondine-marker" d="M455.6,59L447.6,62.36L447.6,55.64Z" fill="var(--_l)" stroke="none"/></g></g><g class="ondine-edge-labels" font-size="14"><g class="ondine-edge-label" data-id="L_B_C_0"><rect x="392.9" y="48.5" width="33.2" height="21" rx="3" fill="var(--_bg)"/><text x="409.5" y="63.9" text-anchor="middle">Yes</text></g></g><g class="ondine-nodes"><g class="ondine-node ondine-shape-rect" data-id="A" transform="translate(83.6,59)"><rect x="-75.6" y="-24" width="151.2" height="48" fill="var(--_s)" stroke="var(--_b)" style="rx:var(--_r)"/><text class="ondine-label" x="0" y="5.6" text-anchor="middle">Mermaid text</text></g><g class="ondine-node ondine-shape-diam" data-id="B" transform="translate(288.8,59)"><polygon points="0,-51 81.6,0 0,51 -81.6,0" fill="var(--_s)" stroke="var(--_b)"/><text class="ondine-label" x="0" y="5.6" text-anchor="middle">Supported?</text></g><g class="ondine-node ondine-shape-rect" data-id="C" transform="translate(502.4,59)"><rect x="-46.8" y="-24" width="93.6" height="48" fill="var(--_s)" stroke="var(--_b)" style="rx:var(--_r)"/><text class="ondine-label" x="0" y="5.6" text-anchor="middle">Ondine</text></g></g></svg>"`,
    )
  })

  test('stacks xychart bar series', () => {
    // Positive values stack above the baseline and negative values below it.
    const result = render(
      '---\nconfig:\n  xyChart:\n    stacked: true\n    showDataLabel: true\n---\nxychart\n  x-axis [a, b]\n  bar "up" [2, 3]\n  bar "down" [-1, 1]\n',
    )

    const bars = result.svg.match(/<g class="ondine-series ondine-bars".*?<\/g>/g)
    const totals = result.svg.match(/<g class="ondine-data-labels".*?<\/g>/)?.[0]

    expect(bars).toMatchInlineSnapshot(`
      [
        "<g class="ondine-series ondine-bars" data-id="up" fill="var(--ondine-series-1,color-mix(in srgb,var(--_fg) 100%,var(--_bg)))"><rect x="78.27" y="203.2" width="202.86" height="173.2"/><rect x="368.07" y="116.6" width="202.86" height="259.8"/></g>",
        "<g class="ondine-series ondine-bars" data-id="down" fill="var(--ondine-series-2,color-mix(in srgb,var(--_fg) 35%,var(--_bg)))"><rect x="78.27" y="376.4" width="202.86" height="86.6"/><rect x="368.07" y="30" width="202.86" height="85.6"/></g>",
      ]
    `)
    expect(totals).toMatchInlineSnapshot(
      `"<g class="ondine-data-labels" font-size="12"><text x="179.7" y="478.2" text-anchor="middle">-1</text><text x="179.7" y="196.4" text-anchor="middle">2</text><text x="469.5" y="23.2" text-anchor="middle">4</text></g>"`,
    )
  })

  test('draws xychart series on a right axis', () => {
    // The line named in rightAxis.series is measured against the right axis, from 0 to 10.
    const result = render(
      '---\nconfig:\n  xyChart:\n    rightAxis:\n      series: [rate]\n      title: Rate\n      min: 0\n      max: 10\n---\nxychart\n  x-axis [a, b]\n  bar "count" [100, 200]\n  line "rate" [5, 10]\n',
    )

    const line = result.svg.match(/<g class="ondine-series ondine-line".*?<\/g>/)?.[0]
    const titles = result.svg.match(/<g class="ondine-axis-titles".*?<\/g><\/g>/)?.[0]

    expect(line).toMatchInlineSnapshot(
      `"<g class="ondine-series ondine-line" data-id="rate" fill="var(--ondine-series-2,color-mix(in srgb,var(--_fg) 35%,var(--_bg)))"><path d="M170.45,240.75L424.95,18.5" fill="none" stroke="var(--ondine-series-2,color-mix(in srgb,var(--_fg) 35%,var(--_bg)))" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/><circle cx="170.45" cy="240.75" r="3"/><circle cx="424.95" cy="18.5" r="3"/></g>"`,
    )
    expect(titles).toMatchInlineSnapshot(
      `"<g class="ondine-axis-titles" font-size="14"><g transform="translate(595.5,240.75) rotate(90)"><text x="0" y="4.9" text-anchor="middle">Rate</text></g></g>"`,
    )
  })

  test('rejects text without a diagram type', () => {
    expect(() => render('not a diagram')).toThrowErrorMatchingInlineSnapshot(
      `[Render.RenderError: No diagram type detected for the given text.]`,
    )
    expect(() => render('not a diagram')).toThrow(RenderError)
  })
})

describe('create', () => {
  test('renders with a precompiled module', () => {
    const module = new WebAssembly.Module(
      Fs.readFileSync(new URL('./internal/flowchart.generated.wasm', import.meta.url)),
    )
    const renderer = Render.create({ modules: { flowchart: module } })

    expect(() => renderer.render('not a diagram')).toThrowErrorMatchingInlineSnapshot(
      `[Render.RenderError: No diagram type detected for the given text.]`,
    )
  })
})
