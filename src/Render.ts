/** @module Renders Mermaid source to monochrome SVG through compiler modules supplied by the host. */
import * as Wasm from './internal/Wasm.js'

/** Thrown when source cannot be rendered. Messages match Mermaid's diagnostics. */
export class RenderError extends Error {
  override name = 'Render.RenderError'

  /** The kind of failure. */
  readonly code: RenderError.Code
  /** One-based UTF-16 column, or 0 when the error has no position. */
  readonly column: number
  /** One-based line, or 0 when the error has no position. */
  readonly line: number
  /** The offending source line with a caret under the column, or empty. */
  readonly snippet: string
  /** The detected diagram type, when known. */
  readonly type: string | undefined

  constructor(options: RenderError.Options) {
    super(options.message)
    this.code = options.code
    this.column = options.column
    this.line = options.line
    this.snippet = options.snippet
    this.type = options.type
  }
}

export declare namespace RenderError {
  /** The kind of failure. */
  type Code = 'limit' | 'semantic' | 'syntax' | 'unsupported-diagram'

  /** Constructor fields. */
  type Options = {
    readonly code: Code
    readonly column: number
    readonly line: number
    readonly message: string
    readonly snippet: string
    readonly type: string | undefined
  }
}

/**
 * Creates a renderer from caller-supplied diagram modules.
 *
 * Each module holds one diagram type, or `all` of them. Modules may be precompiled by the host's
 * bundler, as Cloudflare Workers require, or given as base64 bytes.
 *
 * @example
 * ```ts
 * import { create } from 'ondine/compiler/wasm'
 * import { wasm as all } from 'ondine/compiler/wasm/all'
 *
 * const { svg } = create({ modules: { all } }).render('flowchart LR\n  A --> B')
 * ```
 */
export function create(options: create.Options): create.ReturnType {
  const instances = new Map<string, Wasm.Instance>()
  const instance = (type: string) => {
    let found = instances.get(type)
    if (found) return found

    const module = options.modules[type]
    if (module === undefined) return undefined
    found = Wasm.instantiate(module)
    instances.set(type, found)
    return found
  }

  const types = Object.keys(options.modules)

  return {
    render(source, renderOptions = {}) {
      const first = types[0] === undefined ? undefined : instance(types[0])
      if (first === undefined) throw new Error('Render.create needs at least one module.')

      const json = serialize(renderOptions)
      // The compiler reads no clock. Like JSON, it reads a time that is not a number as none.
      const time = renderOptions.now ?? Date.now()
      const now = typeof time === 'number' ? time : Number.NaN
      const result = first.render(source, json, now)
      // A module reports the detected type when it does not include it.
      if (!result.ok && result.code === 'unsupported-diagram' && result.type !== undefined) {
        const owner = instance(result.type)
        if (owner) return unwrap(owner.render(source, json, now))
      }
      return unwrap(result)
    },
  }
}

export declare namespace create {
  /** Factory options. */
  type Options = {
    /** WebAssembly modules by diagram type, as compiled modules or base64 bytes. */
    readonly modules: Readonly<Record<string, WebAssembly.Module | string>>
  }

  /** A renderer over the given modules. */
  type ReturnType = {
    /** Renders `source`, choosing the module for its diagram type. */
    readonly render: (source: string, options?: render.Options) => render.ReturnType
  }
}

export declare namespace render {
  /** Rendering options. Every field is optional. */
  type Options = {
    /** Lets a diagram that runs across be drawn running down when it does not fit `maxWidth`. Defaults to `true`. */
    readonly autoDirection?: boolean | undefined
    /** The width under which `autoDirection` turns a diagram. Defaults to `640`. */
    readonly directionBreakpoint?: number | undefined
    /** Font size in pixels. Defaults to `16`. */
    readonly fontSize?: number | undefined
    /** Prefix for element IDs, to keep several diagrams on one page apart. */
    readonly idPrefix?: string | undefined
    /** The most characters of source to accept. Defaults to `50000`. */
    readonly limit?: number | undefined
    /** The most edges a flowchart may have. Defaults to `5000`. */
    readonly maxEdges?: number | undefined
    /** The width available to the diagram, in pixels. */
    readonly maxWidth?: number | undefined
    /** The time a gantt chart schedules against, in milliseconds since the epoch. Defaults to the current time. */
    readonly now?: number | undefined
    /** The most characters of SVG to produce. Defaults to `4000000`. */
    readonly outputLimit?: number | undefined
    /** Padding around the diagram, in pixels. Defaults to `8`. */
    readonly padding?: number | undefined
    /** Whether the SVG scales to its container. Defaults to `true`. */
    readonly responsive?: boolean | undefined
    /**
     * Glyph advances of the font that draws labels, for layout with a proportional font. Set
     * `--ondine-font` to the same font. Each character advances 0.6 em when absent. The compiler
     * ignores the whole option when a table does not have 95 entries or an advance is not a finite,
     * non-negative number.
     */
    readonly textMetrics?: TextMetrics | undefined
  }

  /** A rendered diagram. */
  type ReturnType = {
    /** Height in pixels. */
    readonly height: number
    /** Links in drawing order. */
    readonly links: readonly Wasm.Link[]
    /** The SVG document. */
    readonly svg: string
    /** The detected diagram type. */
    readonly type: string
    /** Width in pixels. */
    readonly width: number
  }

  /** Glyph advances in thousandths of an em, as a font's `advanceWidth / unitsPerEm * 1000`. */
  type TextMetrics = {
    /** Advances of printable ASCII (U+0020 to U+007E) in code point order. Exactly 95 entries. */
    readonly advances: readonly number[]
    /** Advances of printable ASCII in bold text, with 95 entries. Bold text reads `advances` when absent. */
    readonly bold?: readonly number[] | undefined
    /** Advance of every other character that is not wide or zero-width. Defaults to `600`. */
    readonly other?: number | undefined
    /** Advance of a wide (East Asian or emoji) character. Defaults to `1200`. */
    readonly wide?: number | undefined
  }
}

/** Options as the compiler reads them, without the time, which is passed apart. Empty for none. */
function serialize(options: render.Options): string {
  for (const _ in options) return JSON.stringify({ ...options, now: undefined })
  return ''
}

function unwrap(result: Wasm.Result): render.ReturnType {
  if (!result.ok) throw new RenderError({ ...result, code: result.code as RenderError.Code })

  return {
    height: result.height,
    links: result.links,
    svg: result.svg,
    type: result.type,
    width: result.width,
  }
}
