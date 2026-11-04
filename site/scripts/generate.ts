/**
 * Generates the diagram pages and the diagram navigation from the playground samples. Run
 * `pnpm build:wasm` first.
 * @module
 */
import * as Fs from 'node:fs'
import * as Path from 'node:path'
import { render, RenderError } from 'ondine/compiler'

const root = Path.resolve(import.meta.dirname, '../..')
const site = Path.resolve(import.meta.dirname, '..')

/** Diagram pages in `samples.json` order: stable types, then beta types. */
const diagrams: Readonly<
  Record<
    string,
    {
      readonly description: string
      /** Markdown placed before the examples, for options the samples do not show. */
      readonly guide?: string | undefined
      readonly title: string
      readonly type: string
    }
  >
> = {
  flowchart: {
    description: 'Nodes and edges, grouped into subgraphs, laid out in one direction.',
    title: 'Flowchart',
    type: 'flowchart',
  },
  block: {
    description: 'Blocks placed on a grid of columns, with spans, nesting, and arrows.',
    title: 'Block',
    type: 'block',
  },
  c4: {
    description: 'Context, container, component, dynamic, and deployment views of a system.',
    title: 'C4',
    type: 'c4',
  },
  class: {
    description: 'Classes with members, relationships, cardinality, and namespaces.',
    title: 'Class',
    type: 'class',
  },
  er: {
    description: 'Entities with attributes and the relationships between them.',
    title: 'Entity Relationship',
    type: 'er',
  },
  eventmodeling: {
    description: 'Screens, commands, events, and views placed along time frames.',
    title: 'Event Modeling',
    type: 'eventmodeling',
  },
  gantt: {
    description: 'Tasks and milestones scheduled on a time axis, grouped into sections.',
    title: 'Gantt',
    type: 'gantt',
  },
  git: {
    description: 'Commits, branches, merges, and cherry-picks of a Git history.',
    title: 'Git Graph',
    type: 'gitGraph',
  },
  info: {
    description: 'The renderer name and version.',
    title: 'Info',
    type: 'info',
  },
  journey: {
    description: 'Tasks in sections, scored for each actor taking part.',
    title: 'User Journey',
    type: 'journey',
  },
  kanban: {
    description: 'Columns of cards with optional metadata.',
    title: 'Kanban',
    type: 'kanban',
  },
  mindmap: {
    description: 'A tree of ideas drawn out from a root, set by indentation.',
    title: 'Mindmap',
    type: 'mindmap',
  },
  packet: {
    description: 'Fields of a network packet laid out by bit range.',
    title: 'Packet',
    type: 'packet',
  },
  pie: {
    description: 'Slices sized by value, with an optional title.',
    title: 'Pie Chart',
    type: 'pie',
  },
  quadrant: {
    description: 'Points plotted on two axes split into four quadrants.',
    title: 'Quadrant Chart',
    type: 'quadrantChart',
  },
  requirement: {
    description: 'Requirements, elements, and the relationships that trace them.',
    title: 'Requirement',
    type: 'requirement',
  },
  sankey: {
    description: 'Flows between nodes, with widths proportional to value.',
    title: 'Sankey',
    type: 'sankey',
  },
  sequence: {
    description: 'Messages between participants over time, with blocks and notes.',
    title: 'Sequence',
    type: 'sequence',
  },
  state: {
    description: 'States, transitions, composite states, forks, and choices.',
    title: 'State',
    type: 'state',
  },
  timeline: {
    description: 'Events listed under time periods, grouped into sections.',
    title: 'Timeline',
    type: 'timeline',
  },
  xychart: {
    description: 'Bar and line series on an x and y axis.',
    guide: [
      '## Stacked Bars',
      'The `stacked` option of `xyChart` draws all bar series as one bar per category. Positive values stack above the baseline, and negative values stack below it. Lines do not stack.',
      'If no `y-axis` range is set, the value axis fits the stack totals. With `showDataLabel`, each stack shows its total.',
      fence(
        'mermaid',
        [
          '---',
          'config:',
          '  xyChart:',
          '    stacked: true',
          '    showDataLabel: true',
          '---',
          'xychart',
          '    title "Orders by channel"',
          '    x-axis [Q1, Q2, Q3, Q4]',
          '    %% Each bar series is one layer of the stack',
          '    bar "Online" [12, 15, 18, 22]',
          '    bar "Store" [8, 7, 9, 10]',
          '    bar "Partner" [3, 4, 6, 7]',
        ].join('\n'),
      ),
    ].join('\n\n'),
    title: 'XY Chart',
    type: 'xychart',
  },
  agentflow: {
    description: 'Inputs, agents, tools, and the flows that connect them.',
    title: 'Agent Flow',
    type: 'agentflow',
  },
  architecture: {
    description: 'Services and groups connected through their sides.',
    title: 'Architecture',
    type: 'architecture',
  },
  cynefin: {
    description: 'Items placed in the domains of the Cynefin framework.',
    title: 'Cynefin',
    type: 'cynefin',
  },
  ishikawa: {
    description: 'Causes branching from a spine toward an effect.',
    title: 'Ishikawa',
    type: 'ishikawa',
  },
  radar: {
    description: 'Series plotted on axes that radiate from a center.',
    title: 'Radar',
    type: 'radar',
  },
  railroad: {
    description: 'Grammar rules drawn as railroad tracks.',
    title: 'Railroad',
    type: 'railroad',
  },
  swimlane: {
    description: 'Flowchart steps placed in lanes.',
    title: 'Swimlane',
    type: 'swimlane',
  },
  treemap: {
    description: 'Nested rectangles sized by value.',
    title: 'Treemap',
    type: 'treemap',
  },
  treeview: {
    description: 'A file or outline tree drawn from indentation.',
    title: 'Tree View',
    type: 'treeView',
  },
  usecase: {
    description: 'Actors, use cases, and the associations between them.',
    title: 'Use Case',
    type: 'usecase',
  },
  venn: {
    description: 'Sets and their overlaps.',
    title: 'Venn',
    type: 'venn',
  },
  wardley: {
    description: 'Components of a value chain placed by evolution and visibility.',
    title: 'Wardley Map',
    type: 'wardley',
  },
}

const samples = JSON.parse(
  Fs.readFileSync(Path.join(root, 'test/vectors/samples.json'), 'utf8'),
) as readonly {
  readonly beta: boolean
  readonly samples: readonly { readonly label: string; readonly source: string }[]
  readonly type: string
}[]

const entries = samples.map((group) => {
  const diagram = diagrams[group.type]
  if (!diagram) throw new Error(`No page for diagram type "${group.type}".`)

  return { ...diagram, beta: group.beta, samples: group.samples, slug: group.type }
})

const pages = Path.join(site, 'src/content/docs/diagrams')
Fs.rmSync(pages, { force: true, recursive: true })
Fs.mkdirSync(pages, { recursive: true })

for (const entry of entries) {
  const examples = entry.samples.map((sample, i) => {
    const heading = `### Example ${i + 1}`
    const error = (() => {
      try {
        render(sample.source)
        return undefined
      } catch (error) {
        if (!(error instanceof RenderError)) throw error
        return error.message
      }
    })()

    if (error !== undefined)
      return `${heading}\n\nOndine reports an error for this source:\n\n${fence('txt', error)}\n\n${fence('txt', sample.source)}`
    return `${heading}\n\n${fence('mermaid', sample.source)}`
  })

  const page = [
    `{/* Generated by scripts/generate.ts. Do not edit. */}`,
    `# ${entry.title}`,
    `${entry.description} ${entry.beta ? 'Mermaid marks this type as beta.' : ''}`.trim(),
    entry.guide ?? '',
    '## Examples',
    'The playground samples for this type, rendered by Ondine when the documentation builds.',
    ...examples,
  ]

  Fs.writeFileSync(
    Path.join(pages, `${entry.slug}.mdx`),
    `${page.filter((part) => part !== '').join('\n\n')}\n`,
  )
}

Fs.writeFileSync(
  Path.join(site, 'src/diagrams.generated.json'),
  `${JSON.stringify(
    entries.map((entry) => ({ slug: entry.slug, title: entry.title })),
    null,
    2,
  )}\n`,
)

function fence(lang: string, source: string): string {
  const ticks = source.includes('```') ? '````' : '```'
  return `${ticks}${lang}\n${source}\n${ticks}`
}
