/**
 * Serves Ondine diagrams as images for Markdown hosts such as GitHub, and as Open Graph cards.
 *
 * `GET /<payload>.svg` renders the Mermaid source in `payload`, encoded as Kroki does: UTF-8, zlib
 * deflate, then base64url. `GET /svg?source=<source>` reads percent-encoded source instead, which
 * authors and agents can write without a compressor. Images draw the light scheme. `?scheme=dark`
 * draws the dark scheme, and `?scheme=auto` follows `prefers-color-scheme`.
 *
 * `GET /<payload>.png` and `GET /png?source=<source>` draw the diagram on a 1200x630 PNG card. Link
 * previewers that request an SVG address receive an HTML page whose `og:image` is that card.
 * `?size=full` draws the PNG at the diagram's own size instead, in the light or `?scheme=dark` scheme.
 * @module
 */
import { Render, RenderError } from 'ondine/compiler/wasm'
// Every diagram type, optimized for size so the Worker fits resvg under the 3 MB limit. Workers
// download nothing per request.
import module from 'ondine/compiler/wasm/all.wasm'
import * as Card from './Card.js'

/** Thrown when a request path does not hold a decodable payload. */
export class PayloadError extends Error {
  override name = 'Image.PayloadError'
}

/** Matches a payload path and captures its payload and format. Static assets such as `/favicon.svg` are served before the Worker runs. */
const payloadPath = /^\/([\w-]+)\.(png|svg)$/
/** Matches a path that reads percent-encoded source from the `source` query parameter. */
const sourcePath = /^\/(png|svg)$/

// Camo rejects proxied URLs over about 8 kB, and it hex-encodes the origin URL, doubling it.
const maxPath = 4096
// Caps inflated source so a small payload cannot expand without bound.
const maxSource = 64 * 1024

const renderer = Render.create({ modules: { all: module } })

const schemes = {
  dark: '--ondine-bg:#000;--ondine-fg:#fff;--ondine-line:#fff',
  light: '--ondine-bg:#fff;--ondine-fg:#000;--ondine-line:#000',
} as const

/** Whether `url` names a diagram image: a payload path or a `/svg` or `/png` source path. */
export function matches(url: URL): boolean {
  return sourcePath.test(url.pathname) || payloadPath.test(url.pathname)
}

/**
 * Answers a request whose URL {@link matches} with the rendered diagram, or with an error card and
 * status 400. A `preview` request for SVG receives an HTML page that names the PNG card as its
 * `og:image`, since link previews do not draw SVG.
 */
export async function respond(request: Request, options: respond.Options = {}): Promise<Response> {
  if (request.method !== 'GET' && request.method !== 'HEAD')
    return new Response('Method not allowed', { headers: { allow: 'GET, HEAD' }, status: 405 })

  const url = new URL(request.url)
  const path = (payloadPath.exec(url.pathname) ?? sourcePath.exec(url.pathname))?.at(-1)
  if (path !== 'png' && path !== 'svg') return new Response('Not found', { status: 404 })

  // Slack and Discord show an image address at full size, but an `og:image` only as a thumbnail.
  const unfurl = /Slackbot|Discordbot/.test(request.headers.get('user-agent') ?? '')
  if (path === 'svg' && options.preview && !unfurl) return preview(url)
  const format = path === 'svg' && options.preview ? 'png' : path

  const scheme = url.searchParams.get('scheme')
  const full = isFull(url)
  const raster = (svg: string) =>
    full ? Card.full(svg, { scheme: scheme === 'dark' ? 'dark' : 'light' }) : Card.png(svg)
  const style = (() => {
    // `color-scheme` paints a matching canvas behind an image that a browser opens on its own.
    if (scheme === 'dark') return `:root{color-scheme:dark;${schemes.dark}}`
    if (scheme === 'auto')
      return `:root{color-scheme:light dark;${schemes.light}}@media (prefers-color-scheme:dark){:root{${schemes.dark}}}`
    return `:root{color-scheme:light;${schemes.light}}`
  })()

  try {
    const source = await (async () => {
      const payload = payloadPath.exec(url.pathname)?.[1]
      if (payload !== undefined) {
        if (url.pathname.length > maxPath)
          throw new PayloadError(`Payload exceeds ${maxPath} characters.`)
        return inflate(payload)
      }

      const text = url.searchParams.get('source')
      if (text === null) throw new PayloadError('The source query parameter is missing.')
      if (url.search.length > maxPath)
        throw new PayloadError(`Payload exceeds ${maxPath} characters.`)
      return text
    })()
    const { svg } = renderer.render(source)
    // Output depends only on the URL and the deployed compiler, so caches may keep it for a day.
    const cache = 'public, max-age=86400, stale-while-revalidate=604800'

    if (format === 'png')
      return cached(new Request(png(url), request), async () =>
        image(await raster(svg), { 'cache-control': cache }),
      )
    return image(theme(svg, style), { 'cache-control': cache, vary: 'User-Agent' })
  } catch (error) {
    if (!(error instanceof PayloadError || error instanceof RenderError)) throw error

    const headers = { 'cache-control': 'public, max-age=300' }
    if (format === 'png') return image(await raster(card(error.message)), headers, 400)
    return image(theme(card(error.message), style), { ...headers, vary: 'User-Agent' }, 400)
  }
}

export declare namespace respond {
  type Options = {
    /** Whether a link previewer, such as Slackbot, sent the request. */
    preview?: boolean | undefined
  }
}

/**
 * Answers `GET` from the data center's cache, storing a successful response on a miss.
 * Rasterizing a card costs far more CPU than compiling its diagram.
 */
async function cached(request: Request, fn: () => Promise<Response>): Promise<Response> {
  // Workers expose the zone cache as `caches.default`, which the DOM types do not declare.
  const cache = (caches as unknown as { default: Cache }).default
  if (request.method === 'GET') {
    const hit = await cache.match(request)
    if (hit) return hit
  }

  const response = await fn()
  if (request.method === 'GET') await cache.put(request, response.clone())
  return response
}

/** Whether `url` asks for a full-size PNG. SVG addresses, even when previewed as PNG, draw the card. */
function isFull(url: URL): boolean {
  return url.pathname.endsWith('png') && url.searchParams.get('size') === 'full'
}

/** Returns the PNG address for the diagram at `url`. Cards ignore the scheme, and full-size images keep it. */
function png(url: URL): URL {
  const card = new URL(url)
  card.pathname = url.pathname.replace(/svg$/, 'png')
  if (isFull(url)) return card

  card.searchParams.delete('scheme')
  card.searchParams.delete('size')
  return card
}

/** Answers a link previewer with a page whose Open Graph image is the PNG card for the same diagram. */
function preview(url: URL): Response {
  const card = png(url)
  const attribute = (value: string) => escape(value).replaceAll('"', '&quot;')

  const html = `<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Mermaid Diagram · Ondine</title><meta content="Mermaid Diagram" property="og:title"><meta content="Ondine" property="og:site_name"><meta content="${attribute(card.href)}" property="og:image"><meta content="Mermaid diagram" property="og:image:alt"><meta content="630" property="og:image:height"><meta content="image/png" property="og:image:type"><meta content="1200" property="og:image:width"><meta content="summary_large_image" name="twitter:card"><meta content="${attribute(card.href)}" name="twitter:image"></head><body><img alt="Mermaid diagram" src="${attribute(url.href)}"></body></html>`

  return new Response(html, {
    headers: {
      'cache-control': 'public, max-age=86400',
      'content-type': 'text/html; charset=utf-8',
      vary: 'User-Agent',
    },
  })
}

/** Decodes a base64url zlib payload into source text, enforcing {@link maxSource}. */
async function inflate(payload: string): Promise<string> {
  const bytes = (() => {
    try {
      return Uint8Array.from(atob(payload.replaceAll('-', '+').replaceAll('_', '/')), (char) =>
        char.charCodeAt(0),
      )
    } catch {
      throw new PayloadError('Payload is not base64url.')
    }
  })()

  const decoder = new TextDecoder('utf-8', { fatal: true })
  const stream = new Blob([bytes]).stream().pipeThrough(new DecompressionStream('deflate'))
  let size = 0
  let source = ''

  try {
    for await (const chunk of stream) {
      size += chunk.byteLength
      if (size > maxSource) throw new PayloadError(`Source exceeds ${maxSource} bytes.`)

      source += decoder.decode(chunk, { stream: true })
    }

    return source + decoder.decode()
  } catch (error) {
    if (error instanceof PayloadError) throw error
    throw new PayloadError('Payload is not zlib-deflated UTF-8.')
  }
}

/** Sets the `--ondine-*` defaults right after the root element opens, since images cannot inherit page styles. */
function theme(svg: string, style: string): string {
  const at = svg.indexOf('>', svg.indexOf('<svg')) + 1
  return `${svg.slice(0, at)}<style>${style}</style>${svg.slice(at)}`
}

/** Draws an error message as a monospace SVG, so a broken diagram explains itself in place. */
function card(message: string): string {
  const lines = ['Ondine could not render this diagram.', ...message.split('\n')]
  const fontSize = 14
  const lineHeight = 20
  const padding = 12
  const width =
    Math.ceil(Math.max(...lines.map((line) => line.length)) * fontSize * 0.6) + padding * 2
  const height = lines.length * lineHeight + padding * 2

  const text = lines
    .map(
      (line, i) =>
        `<tspan x="${padding}" y="${padding + (i + 1) * lineHeight - 6}">${escape(line)}</tspan>`,
    )
    .join('')

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${width}" height="${height}" viewBox="0 0 ${width} ${height}"><rect width="100%" height="100%" fill="var(--ondine-bg)" stroke="var(--ondine-line)" stroke-width="2"/><text font-family="ui-monospace,SFMono-Regular,Menlo,monospace" font-size="${fontSize}" fill="var(--ondine-fg)" xml:space="preserve">${text}</text></svg>`
}

function escape(text: string): string {
  return text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
}

function image(
  body: string | Uint8Array<ArrayBuffer>,
  headers: Record<string, string>,
  status = 200,
): Response {
  return new Response(body, {
    headers: {
      ...headers,
      'content-security-policy': "default-src 'none'; style-src 'unsafe-inline'",
      'content-type': typeof body === 'string' ? 'image/svg+xml; charset=utf-8' : 'image/png',
      'x-content-type-options': 'nosniff',
    },
    status,
  })
}
