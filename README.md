<h1 align="center">Ondine</h1>

<p align="center">
  Mermaid diagrams, compiled with Rust
</p>

<p align="center">
  <a href="#overview">Overview</a> · <a href="#agent-prompt">Agent Prompt</a> · <a href="#philosophy">Philosophy</a> · <a href="#features">Features</a> · <a href="#documentation">Documentation</a> · <a href="site/src/content/docs/guides/vite.mdx">Guides</a> · <a href="site/src/content/docs/api/compiler.mdx">API Reference</a>
</p>

## Overview

<picture>
  <source media="(prefers-color-scheme: dark)" srcset=".github/assets/overview-dark.svg">
  <img alt="Sequence diagram rendered by Ondine" src=".github/assets/overview-light.svg">
</picture>

```text
sequenceDiagram
  participant Browser
  participant Server
  participant Database
  Browser->>Server: GET /orders
  Server->>Database: SELECT * FROM orders
  Database-->>Server: rows
  alt cached
    Server-->>Browser: 304 Not Modified
  else fresh
    Server-->>Browser: 200 OK
  end
```

## Agent Prompt

Paste this prompt into a coding agent:

```txt
Read github.com/wevm/ondine and help me render my Mermaid diagrams with Ondine.
```

## Philosophy

- **Compiled.** Diagrams compile ahead of time, so pages ship SVG instead of a renderer.
- **Compatible.** Every diagram accepts the same input as Mermaid.
- **Precise.** Square outlines and rounded orthogonal edges.
- **Small.** A 210-byte runtime (gzip) mounts precompiled diagrams.

## Features

- [**Vite Plugin**](site/src/content/docs/guides/vite.mdx): `.mmd` imports compiled to SVG at build time.
- [**CLI**](site/src/content/docs/guides/cli.mdx): the `ondine` command compiles diagrams from files or standard input.
- [**Node Compiler**](site/src/content/docs/guides/node.mdx): `ondine/compiler` renders source to SVG in scripts and servers.
- [**Markdown and MDX**](site/src/content/docs/api/remark.mdx): `ondine/remark` replaces ` ```mermaid ` fences with compiled SVG.
- [**In-Browser Compilation**](site/src/content/docs/guides/browser.mdx): opt-in `ondine/compiler/wasm` modules, one per diagram type or `all`.
- [**Swift Package**](site/src/content/docs/guides/swift.mdx): the same modules render in native apps on macOS, iOS, and Linux through WasmKit.
- [**GitHub Images**](site/src/content/docs/guides/github.mdx): an image Worker renders diagrams encoded in a URL for READMEs.
- [**Theming**](site/src/content/docs/guides/theming.mdx): `--ondine-*` CSS custom properties switch schemes without compiling again.
- [**Mounting**](site/src/content/docs/guides/mounting.mdx): `mount` inserts precompiled SVG without downloading a parser or layout code.

## Documentation

Head to the [documentation](https://ondine.sh) to get started and learn more about Ondine.
## License

[MIT](LICENSE)
