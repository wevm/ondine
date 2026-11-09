/** @module Integration tests for rendering Mermaid source through the WebAssembly compiler. */
import * as Fs from 'node:fs'
import { Render, render, RenderError } from 'ondine/compiler'
import * as Vectors from '../test/Vectors.js'

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

  test('writes the theme into the SVG', () => {
    const { svg } = render('flowchart LR\n  A --> B', {
      theme: { bg: '#000', fg: '#fff', radius: '6px' },
    })

    expect(svg).toMatchInlineSnapshot(
      `"<svg xmlns="http://www.w3.org/2000/svg" class="ondine ondine-flowchart" viewBox="0 0 160 64" width="160" height="64" style="max-width:100%;height:auto;stroke-width:2px;color:#fff;font:16px monospace;letter-spacing:normal;word-spacing:normal;text-transform:none;direction:ltr;font-family:ui-monospace, SFMono-Regular, Menlo, Consolas, 'Liberation Mono', 'DejaVu Sans Mono', monospace;font-kerning:none;font-variant-ligatures:none;text-rendering:geometricPrecision" fill="#fff" role="graphics-document document" aria-roledescription="flowchart"><g class="ondine-edges" fill="none" stroke="#fff" stroke-linecap="round"><g class="ondine-edge" data-id="L_A_B_0"><path d="M56,32L97,32"/><path class="ondine-marker" d="M104,32L96,35.36L96,28.64Z" fill="#fff" stroke="none"/></g></g><g class="ondine-nodes"><g class="ondine-node ondine-shape-rect" data-id="A" transform="translate(32,32)"><rect x="-24" y="-24" width="48" height="48" rx="6px" fill="#000" stroke="#fff"/><text class="ondine-label" x="0" y="5.6" text-anchor="middle">A</text></g><g class="ondine-node ondine-shape-rect" data-id="B" transform="translate(128,32)"><rect x="-24" y="-24" width="48" height="48" rx="6px" fill="#000" stroke="#fff"/><text class="ondine-label" x="0" y="5.6" text-anchor="middle">B</text></g></g></svg>"`,
    )
  })

  test('rejects a theme color that a mix cannot read', () => {
    expect(() =>
      render('pie\n  "A" : 3', { theme: { fg: 'hsl(0 0% 0%)' } }),
    ).toThrowErrorMatchingInlineSnapshot(
      `[Render.RenderError: The theme value for "fg" cannot be mixed. Use a hex, named, rgb(), or transparent color.]`,
    )
  })

  test('writes the theme into every vector', () => {
    const themed = Vectors.types().flatMap((type) =>
      Vectors.of(type).flatMap((vector) => {
        const svg = (() => {
          try {
            return render(vector.source, { now: 0, theme: {} }).svg
          } catch {
            // A vector that the compiler rejects has no SVG to check.
            return ''
          }
        })()
        return svg.includes('var(') || svg.includes('color-mix(')
          ? [`${type}: ${vector.label}`]
          : []
      }),
    )

    expect(themed).toMatchInlineSnapshot(`[]`)
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

  test('rejects the theme option in a size-optimized module', () => {
    const module = new WebAssembly.Module(
      Fs.readFileSync(new URL('./internal/flowchart.generated.wasm', import.meta.url)),
    )
    const renderer = Render.create({ modules: { flowchart: module } })

    const error = (() => {
      try {
        renderer.render('flowchart LR\n  A --> B', { theme: {} })
      } catch (error) {
        if (error instanceof RenderError) return error
        throw error
      }
      return undefined
    })()

    expect(error).toMatchInlineSnapshot(
      `[Render.RenderError: This module does not support the theme option. Use ondine/compiler.]`,
    )
    expect(error?.code).toMatchInlineSnapshot(`"unsupported"`)
  })
})
