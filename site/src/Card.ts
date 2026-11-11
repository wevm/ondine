/**
 * Rasterizes a diagram in the Worker with resvg, either onto a 1200x630 Open Graph card, alone on
 * white and scaled to fit, or at its own size for embedding. Build-time page cards come from `Og.tsx`.
 * @module
 */
import regular from '@fontsource/ibm-plex-mono/files/ibm-plex-mono-latin-400-normal.woff2?inline'
import { initWasm, Resvg, type ResvgRenderOptions } from '@resvg/resvg-wasm'
import resvg from '@resvg/resvg-wasm/index_bg.wasm'
import * as Head from './Head.js'
import * as Svg from './Svg.js'

const fonts = [regular].map((url) =>
  Uint8Array.from(atob(url.slice(url.indexOf(',') + 1)), (char) => char.charCodeAt(0)),
)

/** The color schemes, resolved for resvg. resvg has neither custom properties nor `color-mix()`. */
const schemes = {
  dark: {
    '--ondine-bg': '#000',
    '--ondine-fg': '#fff',
    '--ondine-font': "'IBM Plex Mono', monospace",
    '--ondine-line': '#fff',
    '--ondine-surface-alt': '#333',
  },
  light: {
    '--ondine-bg': '#fff',
    '--ondine-fg': '#000',
    '--ondine-font': "'IBM Plex Mono', monospace",
    '--ondine-line': '#000',
    '--ondine-surface-alt': '#ccc',
  },
} as const

/** The area the diagram fits into, inside a 60-pixel margin. */
const frame = { height: 510, width: 1080, x: 60, y: 60 } as const

// Small diagrams scale up to fill the card, but not so far that strokes turn heavy.
const maxScale = 2.5

// Full-size images draw two pixels per SVG unit, so they stay sharp on high-density displays.
const density = 2
// Caps the pixels of a full-size image, since resvg holds the whole RGBA buffer in Worker memory.
const maxPixels = 8 * 1024 * 1024

let ready: Promise<void> | undefined

/** Renders `svg`, an Ondine diagram or error card, onto a PNG card at {@link Head.imageSize}. */
export async function png(svg: string): Promise<Uint8Array<ArrayBuffer>> {
  return rasterize(sheet(Svg.flatten(svg, schemes.light)))
}

/**
 * Renders `svg`, an Ondine diagram or error card, as a PNG at its own size and {@link density},
 * on the background of `scheme`. Large diagrams scale down to stay within {@link maxPixels}.
 */
export async function full(
  svg: string,
  options: full.Options = {},
): Promise<Uint8Array<ArrayBuffer>> {
  const scheme = schemes[options.scheme ?? 'light']
  const { height, width } = size(svg)
  const scale = Math.min(density, Math.sqrt(maxPixels / (width * height)))

  return rasterize(Svg.flatten(svg, scheme), {
    background: scheme['--ondine-bg'],
    fitTo: { mode: 'zoom', value: scale },
  })
}

export declare namespace full {
  type Options = {
    /** The color scheme to draw. Defaults to `light`. */
    scheme?: keyof typeof schemes | undefined
  }
}

async function rasterize(
  svg: string,
  options: Pick<ResvgRenderOptions, 'background' | 'fitTo'> = {},
): Promise<Uint8Array<ArrayBuffer>> {
  ready ??= initWasm(resvg)
  await ready

  const renderer = new Resvg(svg, {
    ...options,
    font: { defaultFontFamily: 'IBM Plex Mono', fontBuffers: fonts, loadSystemFonts: false },
  })
  try {
    return new Uint8Array(renderer.render().asPng())
  } finally {
    renderer.free()
  }
}

/** Reads the width and height a diagram's root element declares. */
function size(svg: string): { height: number; width: number } {
  const open = svg.slice(0, svg.indexOf('>') + 1)
  return {
    height: Number(/\sheight="([\d.]+)"/.exec(open)?.[1] ?? frame.height),
    width: Number(/\swidth="([\d.]+)"/.exec(open)?.[1] ?? frame.width),
  }
}

/** Places a flattened diagram on the card, centered in {@link frame}. */
function sheet(diagram: string): string {
  const open = diagram.slice(0, diagram.indexOf('>') + 1)
  const { height, width } = size(diagram)
  const scale = Math.min(frame.width / width, frame.height / height, maxScale)
  const box = {
    height: height * scale,
    width: width * scale,
    x: frame.x + (frame.width - width * scale) / 2,
    y: frame.y + (frame.height - height * scale) / 2,
  }

  // The nested root takes the frame's box. Its own size and responsive style would override it.
  const root = open
    .replace(/\s(?:width|height)="[^"]*"/g, '')
    .replace('max-width:100%;height:auto;', '')
    .replace('<svg', `<svg x="${box.x}" y="${box.y}" width="${box.width}" height="${box.height}"`)
  const { height: cardHeight, width: cardWidth } = Head.imageSize

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${cardWidth}" height="${cardHeight}" viewBox="0 0 ${cardWidth} ${cardHeight}"><rect width="${cardWidth}" height="${cardHeight}" fill="#fff"/>${root}${diagram.slice(open.length)}</svg>`
}
