/** Exercises the site Worker over HTTP: pages, Markdown twins, agent negotiation, crawler files, and diagram images. @module */
import * as ChildProcess from 'node:child_process'
import * as Zlib from 'node:zlib'
import { afterAll, beforeAll, describe, expect, test } from 'vite-plus/test'

const origin = 'http://localhost:3161'
const encode = (source: string | Buffer) => Zlib.deflateSync(source).toString('base64url')
let server: ChildProcess.ChildProcess | undefined

beforeAll(async () => {
  server = ChildProcess.spawn('pnpm', ['exec', 'vite', 'dev', '--port', '3161', '--strictPort'], {
    cwd: new URL('..', import.meta.url),
    env: { ...process.env, NODE_OPTIONS: '--import=tsx' },
    stdio: 'ignore',
  })

  // The dev server renders every MDX page before it answers, so the first request waits on it.
  const deadline = Date.now() + 120_000
  while (Date.now() < deadline) {
    const ready = await fetch(`${origin}/docs/guides/vite`).then(
      (response) => response.ok,
      () => false,
    )
    if (ready) return
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error('The documentation dev server did not start within 120 seconds.')
}, 130_000)

afterAll(() => {
  server?.kill()
})

describe('fetch', () => {
  test('renders a documentation page with its Mermaid fences compiled to SVG', async () => {
    const response = await fetch(`${origin}/docs/diagrams/pie`)
    const html = await response.text()

    expect(response.status).toMatchInlineSnapshot(`200`)
    expect(html.match(/<title>[^<]*<\/title>/)?.[0]).toMatchInlineSnapshot(
      `"<title>Pie Chart · Diagrams · Ondine</title>"`,
    )
    expect(html.match(/data-ondine="" class="[^"]*"><svg/g)?.length).toMatchInlineSnapshot(`3`)
  })

  test('answers a `.md` path with the page Markdown, keeping Mermaid fences as source', async () => {
    const response = await fetch(`${origin}/docs/diagrams/pie.md`)
    const markdown = await response.text()

    expect(response.headers.get('content-type')).toMatchInlineSnapshot(
      `"text/markdown; charset=utf-8"`,
    )
    expect(markdown.split('\n').slice(0, 3)).toMatchInlineSnapshot(`
      [
        "# Pie Chart",
        "",
        "Slices sized by value, with an optional title.",
      ]
    `)
    expect(
      markdown.includes('```mermaid\npie title Fish caught by the crew'),
    ).toMatchInlineSnapshot(`true`)
  })

  test('negotiates Markdown for agents and HTML for browsers', async () => {
    const accept = await fetch(`${origin}/docs/guides/vite`, {
      headers: { accept: 'text/markdown' },
    })
    const terminal = await fetch(`${origin}/docs/guides/vite`, {
      headers: { 'user-agent': 'curl/8.7.1' },
    })
    const browser = await fetch(`${origin}/docs/guides/vite`, { headers: { accept: 'text/html' } })

    expect(accept.headers.get('content-type')).toMatchInlineSnapshot(
      `"text/markdown; charset=utf-8"`,
    )
    expect((await terminal.text()).split('\n')[0]).toMatchInlineSnapshot(`"# Vite"`)
    expect(browser.headers.get('content-type')).toMatchInlineSnapshot(`"text/html; charset=utf-8"`)
  })

  test('lists every page in llms.txt, linking Markdown twins', async () => {
    const text = await (await fetch(`${origin}/llms.txt`)).text()
    const lines = text.split('\n')

    expect(lines.filter((line) => line.startsWith('## '))).toMatchInlineSnapshot(`
      [
        "## Render Diagrams in Markdown",
        "## Introduction",
        "## Guides",
        "## Diagrams",
        "## API",
      ]
    `)
    expect(
      lines.find((line) => line.includes('/docs/api/compiler/render.md')),
    ).toMatchInlineSnapshot(
      `"- [ondine/compiler › render](http://localhost:3161/docs/api/compiler/render.md): Render Mermaid source to an SVG document in Node."`,
    )
  })

  test('lists site pages in the sitemap and points robots.txt at it', async () => {
    const sitemap = await (await fetch(`${origin}/sitemap.xml`)).text()
    const robots = await (await fetch(`${origin}/robots.txt`)).text()

    expect(sitemap.match(/<loc>[^<]*<\/loc>/g)?.slice(0, 4)).toMatchInlineSnapshot(`
      [
        "<loc>http://localhost:3161/</loc>",
        "<loc>http://localhost:3161/examples</loc>",
        "<loc>http://localhost:3161/playground</loc>",
        "<loc>http://localhost:3161/docs/api/cli</loc>",
      ]
    `)
    expect(robots).toMatchInlineSnapshot(`
      "User-agent: *
      Allow: /

      Sitemap: http://localhost:3161/sitemap.xml
      "
    `)
  })

  test('redirects pre-/docs paths and reports unknown pages', async () => {
    const legacy = await fetch(`${origin}/guides/vite?x=1`, { redirect: 'manual' })
    const missing = await fetch(`${origin}/docs/guides/missing`)

    expect(legacy.status).toMatchInlineSnapshot(`308`)
    expect(legacy.headers.get('location')).toMatchInlineSnapshot(
      `"http://localhost:3161/docs/guides/vite?x=1"`,
    )
    expect(missing.status).toMatchInlineSnapshot(`404`)
  })

  test('renders the examples board from every playground sample', async () => {
    const html = await (await fetch(`${origin}/examples`)).text()

    expect(html.match(/<a[^>]*href="\/playground#source=/g)?.length).toMatchInlineSnapshot(`167`)
  })

  test('links each page to its Open Graph card and serves it as a PNG', async () => {
    const html = await (await fetch(`${origin}/docs/guides/vite`)).text()
    const image = await fetch(`${origin}/og/docs/guides/vite.png`)
    const png = Buffer.from(await image.arrayBuffer())

    expect(html.match(/<meta[^>]*(?:og:image|twitter:)[^>]*>/g)).toMatchInlineSnapshot(`
      [
        "<meta content="https://ondine.sh/og/docs/guides/vite.png" property="og:image"/>",
        "<meta content="Vite · Guides · Ondine" property="og:image:alt"/>",
        "<meta content="630" property="og:image:height"/>",
        "<meta content="image/png" property="og:image:type"/>",
        "<meta content="1200" property="og:image:width"/>",
        "<meta content="summary_large_image" name="twitter:card"/>",
        "<meta content="https://ondine.sh/og/docs/guides/vite.png" name="twitter:image"/>",
      ]
    `)
    expect(image.headers.get('content-type')).toMatchInlineSnapshot(`"image/png"`)
    // A PNG's IHDR chunk holds its width and height at bytes 16 through 23.
    expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toMatchInlineSnapshot(`
      [
        1200,
        630,
      ]
    `)
  })

  test('uses the default Open Graph card on the home page', async () => {
    const html = await (await fetch(`${origin}/`)).text()
    const image = await fetch(`${origin}/og.png`)

    expect(html.match(/<meta[^>]*og:image"[^>]*>/g)).toMatchInlineSnapshot(`
      [
        "<meta content="https://ondine.sh/og.png" property="og:image"/>",
      ]
    `)
    expect(image.status).toMatchInlineSnapshot(`200`)
  })

  test('renders diagram images through the precompiled module in the light scheme', async () => {
    const response = await fetch(`${origin}/${encode('not a diagram')}.svg`)

    expect(response.status).toMatchInlineSnapshot(`400`)
    expect(response.headers.get('cache-control')).toMatchInlineSnapshot(`"public, max-age=300"`)
    expect(response.headers.get('content-security-policy')).toMatchInlineSnapshot(
      `"default-src 'none'; style-src 'unsafe-inline'"`,
    )
    expect(response.headers.get('content-type')).toMatchInlineSnapshot(
      `"image/svg+xml; charset=utf-8"`,
    )
    expect(response.headers.get('x-content-type-options')).toMatchInlineSnapshot(`"nosniff"`)
    expect(await response.text()).toMatchInlineSnapshot(
      `"<svg xmlns="http://www.w3.org/2000/svg" width="394" height="64" viewBox="0 0 394 64"><style>:root{color-scheme:light;--ondine-bg:#fff;--ondine-fg:#000;--ondine-line:#000}</style><rect width="100%" height="100%" fill="var(--ondine-bg)" stroke="var(--ondine-line)" stroke-width="2"/><text font-family="ui-monospace,SFMono-Regular,Menlo,monospace" font-size="14" fill="var(--ondine-fg)" xml:space="preserve"><tspan x="12" y="26">Ondine could not render this diagram.</tspan><tspan x="12" y="46">No diagram type detected for the given text.</tspan></text></svg>"`,
    )
  })

  test('renders diagram images from percent-encoded source', async () => {
    const source = encodeURIComponent('flowchart LR\n  A --> B')
    const response = await fetch(`${origin}/svg?source=${source}`)
    const missing = await fetch(`${origin}/svg`)

    expect(response.status).toMatchInlineSnapshot(`200`)
    expect((await response.text()).match(/^<svg[^>]*class="[^"]*"/)?.[0]).toMatchInlineSnapshot(
      `"<svg xmlns="http://www.w3.org/2000/svg" class="ondine ondine-flowchart""`,
    )
    expect(missing.status).toMatchInlineSnapshot(`400`)
    expect((await missing.text()).match(/The source[^<]*/)?.[0]).toMatchInlineSnapshot(
      `"The source query parameter is missing."`,
    )
  })

  test('draws diagram images on PNG cards', async () => {
    const response = await fetch(`${origin}/${encode('flowchart LR\n  A --> B')}.png`)
    const source = await fetch(`${origin}/png?source=${encodeURIComponent('pie\n  "A" : 1')}`)
    const invalid = await fetch(`${origin}/${encode('not a diagram')}.png`)
    const png = Buffer.from(await response.arrayBuffer())

    expect(response.headers.get('content-type')).toMatchInlineSnapshot(`"image/png"`)
    // A PNG's IHDR chunk holds its width and height at bytes 16 through 23.
    expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toMatchInlineSnapshot(`
      [
        1200,
        630,
      ]
    `)
    expect(source.status).toMatchInlineSnapshot(`200`)
    expect(invalid.status).toMatchInlineSnapshot(`400`)
    expect(invalid.headers.get('content-type')).toMatchInlineSnapshot(`"image/png"`)
  })

  test('draws full-size PNG images at twice the diagram size', async () => {
    const payload = encode('flowchart LR\n  A --> B')
    const svg = await (await fetch(`${origin}/${payload}.svg`)).text()
    const light = await fetch(`${origin}/${payload}.png?size=full`)
    const dark = await fetch(
      `${origin}/png?source=${encodeURIComponent('flowchart LR\n  A --> B')}&size=full&scheme=dark`,
    )
    const invalid = await fetch(`${origin}/${encode('not a diagram')}.png?size=full`)
    const png = Buffer.from(await light.arrayBuffer())
    const size = /width="(\d+)" height="(\d+)"/.exec(svg)

    expect(light.headers.get('content-type')).toMatchInlineSnapshot(`"image/png"`)
    // A PNG's IHDR chunk holds its width and height at bytes 16 through 23.
    expect([png.readUInt32BE(16), png.readUInt32BE(20)]).toEqual([
      Number(size?.[1]) * 2,
      Number(size?.[2]) * 2,
    ])
    expect(dark.status).toMatchInlineSnapshot(`200`)
    expect(Buffer.from(await dark.arrayBuffer()).equals(png)).toBe(false)
    expect(invalid.status).toMatchInlineSnapshot(`400`)
    expect(invalid.headers.get('content-type')).toMatchInlineSnapshot(`"image/png"`)
  })

  test('answers link previewers on SVG addresses with the PNG card', async () => {
    const path = `/svg?source=${encodeURIComponent('flowchart LR\n  A --> B')}&scheme=dark`
    const twitter = await fetch(`${origin}${path}`, { headers: { 'user-agent': 'Twitterbot/1.0' } })
    const slack = await fetch(`${origin}${path}`, {
      headers: { 'user-agent': 'Slackbot-LinkExpanding 1.0' },
    })
    const image = await fetch(`${origin}${path}`)

    expect(twitter.headers.get('content-type')).toMatchInlineSnapshot(`"text/html; charset=utf-8"`)
    expect((await twitter.text()).match(/<meta[^>]*og:image"[^>]*>/)?.[0]).toMatchInlineSnapshot(
      `"<meta content="http://localhost:3161/png?source=flowchart+LR%0A++A+--%3E+B" property="og:image">"`,
    )
    expect(slack.headers.get('content-type')).toMatchInlineSnapshot(`"image/png"`)
    expect(image.headers.get('content-type')).toMatchInlineSnapshot(
      `"image/svg+xml; charset=utf-8"`,
    )
    expect(image.headers.get('vary')).toMatchInlineSnapshot(`"User-Agent"`)
  })

  test('previews a shared playground diagram as its PNG card', async () => {
    const html = await (
      await fetch(`${origin}/playground?d=${encode('flowchart LR\n  A --> B')}`)
    ).text()
    const plain = await (await fetch(`${origin}/playground`)).text()

    expect(html.match(/<meta[^>]*og:image"[^>]*>/g)).toMatchInlineSnapshot(`
      [
        "<meta content="https://ondine.sh/eJxLy8kvT85ILCpR8AniUlBwVNDVtVNwAgBUvQYu.png" property="og:image"/>",
      ]
    `)
    expect(plain.match(/<meta[^>]*og:image"[^>]*>/g)).toMatchInlineSnapshot(`
      [
        "<meta content="https://ondine.sh/og/playground.png" property="og:image"/>",
      ]
    `)
  })

  test('pins a diagram image scheme', async () => {
    const response = await fetch(`${origin}/${encode('not a diagram')}.svg?scheme=dark`)

    expect((await response.text()).match(/<style>.*?<\/style>/)?.[0]).toMatchInlineSnapshot(
      `"<style>:root{color-scheme:dark;--ondine-bg:#000;--ondine-fg:#fff;--ondine-line:#fff}</style>"`,
    )
  })

  test('follows the reader with the auto scheme', async () => {
    const response = await fetch(`${origin}/${encode('not a diagram')}.svg?scheme=auto`)

    expect((await response.text()).match(/<style>.*?<\/style>/)?.[0]).toMatchInlineSnapshot(
      `"<style>:root{color-scheme:light dark;--ondine-bg:#fff;--ondine-fg:#000;--ondine-line:#000}@media (prefers-color-scheme:dark){:root{--ondine-bg:#000;--ondine-fg:#fff;--ondine-line:#fff}}</style>"`,
    )
  })

  test('rejects malformed diagram image payloads', async () => {
    const notBase64 = await fetch(`${origin}/a.svg`)
    expect(notBase64.status).toMatchInlineSnapshot(`400`)
    expect((await notBase64.text()).match(/Payload[^<]*/)?.[0]).toMatchInlineSnapshot(
      `"Payload is not base64url."`,
    )

    const notDeflate = await fetch(
      `${origin}/${Buffer.from('flowchart').toString('base64url')}.svg`,
    )
    expect(notDeflate.status).toMatchInlineSnapshot(`400`)
    expect((await notDeflate.text()).match(/Payload[^<]*/)?.[0]).toMatchInlineSnapshot(
      `"Payload is not zlib-deflated UTF-8."`,
    )
  })

  test('caps inflated diagram image source', async () => {
    const response = await fetch(`${origin}/${encode(Buffer.alloc(65 * 1024, 'a'))}.svg`)

    expect(response.status).toMatchInlineSnapshot(`400`)
    expect((await response.text()).match(/Source[^<]*/)?.[0]).toMatchInlineSnapshot(
      `"Source exceeds 65536 bytes."`,
    )
  })

  test('caps the diagram image path length', async () => {
    const response = await fetch(`${origin}/${'a'.repeat(4096)}.svg`)

    expect((await response.text()).match(/Payload[^<]*/)?.[0]).toMatchInlineSnapshot(
      `"Payload exceeds 4096 characters."`,
    )
  })

  test('rejects other methods for diagram images', async () => {
    const response = await fetch(`${origin}/${encode('flowchart')}.svg`, { method: 'POST' })

    expect(response.status).toMatchInlineSnapshot(`405`)
    expect(response.headers.get('allow')).toMatchInlineSnapshot(`"GET, HEAD"`)
  })
})
