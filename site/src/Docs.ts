/** Indexes authored MDX pages for navigation and route rendering. @module */
import type { ComponentType, ElementType, ReactPromise } from 'react'
import diagrams from './diagrams.generated.json' with { type: 'json' }
import * as Manifest from './Manifest.js'

/** Authored sidebar order. Diagram pages sort by title. */
export const groups: readonly { pages: readonly Item[]; title: string }[] = [
  {
    title: 'Introduction',
    pages: [
      { path: 'introduction/getting-started', title: 'Getting Started' },
      { path: 'introduction/why-ondine', title: 'Why Ondine' },
      { path: 'introduction/benchmarks', title: 'Benchmarks' },
      { path: 'introduction/faq', title: 'FAQ' },
    ],
  },
  {
    title: 'Guides',
    pages: [
      { path: 'guides/vite', title: 'Vite' },
      { path: 'guides/cli', title: 'CLI' },
      { path: 'guides/mdx', title: 'MDX' },
      { path: 'guides/node', title: 'Node Compiler' },
      { path: 'guides/browser', title: 'In-Browser Compilation' },
      { path: 'guides/swift', title: 'Swift Package' },
      { path: 'guides/github', title: 'GitHub Images' },
      { path: 'guides/theming', title: 'Theming' },
      { path: 'guides/mounting', title: 'Mounting' },
    ],
  },
  {
    title: 'Diagrams',
    pages: diagrams
      .map((diagram) => ({ path: `diagrams/${diagram.slug}`, title: diagram.title }))
      .sort((a, b) => a.title.localeCompare(b.title)),
  },
  {
    title: 'API',
    pages: [
      {
        title: 'ondine',
        children: [
          { path: 'api/ondine', title: 'Overview' },
          { path: 'api/ondine/mount', title: 'mount' },
        ],
      },
      {
        title: 'ondine/compiler',
        children: [
          { path: 'api/compiler', title: 'Overview' },
          { path: 'api/compiler/render', title: 'render' },
          { path: 'api/compiler/create', title: 'create' },
          { path: 'api/compiler/RenderError', title: 'RenderError' },
        ],
      },
      {
        title: 'ondine/compiler/wasm',
        children: [
          { path: 'api/compiler-wasm', title: 'Overview' },
          { path: 'api/compiler-wasm/all', title: 'wasm (all)' },
          { path: 'api/compiler-wasm/types', title: 'Diagram Modules' },
        ],
      },
      {
        title: 'ondine/remark',
        children: [
          { path: 'api/remark', title: 'Overview' },
          { path: 'api/remark/remarkOndine', title: 'remarkOndine' },
        ],
      },
      {
        title: 'ondine/vite',
        children: [
          { path: 'api/vite', title: 'Overview' },
          { path: 'api/vite/ondine', title: 'ondine' },
        ],
      },
      { path: 'api/cli', title: 'CLI' },
      { path: 'api/worker', title: 'Image Worker' },
    ],
  },
]

// Lazy imports give each page its own chunk, keeping every asset under the Workers size limit.
const modules = import.meta.glob<Content>('./content/docs/**/*.mdx', {
  import: 'default',
})
const loads = new Map<string, ReactPromise<Content>>()

/** Loads a page's compiled MDX once, marking the promise fulfilled so `use` reads it without suspending again. */
export function load(path: string): ReactPromise<Content> {
  const previous = loads.get(path)
  if (previous) return previous

  const module = modules[`./content/docs/${path}.mdx`]
  if (!module) throw new Error(`Missing documentation page: ${path}`)

  const promise: Promise<Content> = module().then(
    (content) => {
      Object.assign(promise, { status: 'fulfilled', value: content })
      return content
    },
    (error: unknown) => {
      // A failed chunk request must not stay cached, so a later attempt can retry it.
      loads.delete(path)
      throw error
    },
  )
  loads.set(path, promise)

  return promise
}

/** Lists an item's path and every path nested beneath it. */
export function paths(item: Item): readonly string[] {
  return [...(item.path === undefined ? [] : [item.path]), ...(item.children ?? []).flatMap(paths)]
}

/**
 * Lists the sidebar group and topics above a page, outermost first, such as `['API', 'ondine']`.
 * Pages outside the sidebar return an empty list.
 */
export function trail(path: string): readonly string[] {
  function find(items: readonly Item[]): readonly string[] | undefined {
    for (const item of items) {
      if (item.path === path) return []

      const children = item.children && find(item.children)
      if (children) return [item.title, ...children]
    }
    return undefined
  }

  for (const group of groups) {
    const titles = find(group.pages)
    if (titles) return [group.title, ...titles]
  }
  return []
}

/** The site area a page belongs to: diagram pages have their own sidebar, apart from the docs. */
export function section(path: string): 'diagrams' | 'docs' {
  return path.startsWith('diagrams/') ? 'diagrams' : 'docs'
}

/** The sidebar groups shown beside a page, from its section. */
export function sidebar(path: string): typeof groups {
  const diagrams = section(path) === 'diagrams'
  return groups.filter((group) => (group.title === 'Diagrams') === diagrams)
}

/** Pages of a page's sidebar in order, for previous and next links. */
export function sequence(path: string): readonly string[] {
  return sidebar(path).flatMap((group) =>
    group.pages.flatMap(paths).filter((page) => Object.hasOwn(Manifest.pages, page)),
  )
}

/** Compiled MDX component for one documentation page. */
export type Content = ComponentType<{
  /** Elements and MDX components that replace the defaults while rendering. */
  components?: Record<string, ElementType>
}>

/** A page or nested topic in documentation navigation. */
export type Item = {
  /** Pages shown beneath the topic. */
  children?: readonly Item[] | undefined
  /** Documentation path, such as `guides/vite`. */
  path?: string | undefined
  /** Human-readable navigation label. */
  title: string
}
