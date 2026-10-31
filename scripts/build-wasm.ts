/**
 * Builds one size-optimized WebAssembly module per diagram type and inlines each as base64 in
 * `src/internal/<type>.generated.ts`, beside the raw `src/internal/<type>.generated.wasm` for hosts
 * that only accept precompiled modules. Also builds `all.fast.generated.wasm`, every type optimized
 * for speed with the `theme` option, which `ondine/compiler` loads in Node, and
 * `test/oracles/model.generated.wasm`, every type with the `model` export that the oracle tests
 * read. Outputs are gitignored and rewritten only when they change. Writes the matching
 * `ondine/compiler/wasm/<type>` and `<type>.wasm` exports into `package.json`.
 * @module
 */
import * as ChildProcess from 'node:child_process'
import * as Crypto from 'node:crypto'
import * as Fs from 'node:fs'
import * as Os from 'node:os'
import * as Path from 'node:path'

const root = Path.resolve(import.meta.dirname, '..')

/** Diagram types with a compiler module. Each builds into its own WebAssembly module. */
const types = [
  'agentflow',
  'architecture',
  'block',
  'c4',
  'class',
  'cynefin',
  'er',
  'eventmodeling',
  'flowchart',
  'gantt',
  'git',
  'info',
  'ishikawa',
  'journey',
  'kanban',
  'mindmap',
  'packet',
  'pie',
  'quadrant',
  'radar',
  'railroad',
  'requirement',
  'sankey',
  'sequence',
  'state',
  'swimlane',
  'timeline',
  'treemap',
  'treeview',
  'usecase',
  'venn',
  'wardley',
  'xychart',
] as const

/**
 * The nightly toolchain that rebuilds `std` for size. `ONDINE_WASM_TOOLCHAIN` overrides it. Without
 * it, or without its `rust-src` component, modules build on the stable toolchain and are larger.
 */
const nightly = process.env.ONDINE_WASM_TOOLCHAIN ?? 'nightly-2026-06-29'
const nightlyInstalled = installed(nightly)

if (!nightlyInstalled)
  console.warn(
    `${nightly} with rust-src is not installed. Building larger modules with the stable toolchain. Install it with: rustup toolchain install ${nightly} --component rust-src --target wasm32-unknown-unknown`,
  )

// Exports come first, so a stale manifest is visible before the modules finish building.
write(
  Path.join(root, 'package.json'),
  manifest(Fs.readFileSync(Path.join(root, 'package.json'), 'utf8')),
)

// The stamp stays outside `src/`, which the package publishes.
const stampFile = Path.join(root, 'target/wasm.stamp')
const stamp = inputs()
const modelFile = Path.join(root, 'test/oracles/model.generated.wasm')
const outputs = [
  ...['all', ...types].flatMap((type) => [
    Path.join(root, `src/internal/${type}.generated.ts`),
    Path.join(root, `src/internal/${type}.generated.wasm`),
  ]),
  Path.join(root, 'src/internal/all.fast.generated.wasm'),
  modelFile,
]
const fresh =
  Fs.existsSync(stampFile) &&
  Fs.readFileSync(stampFile, 'utf8') === stamp &&
  outputs.every((file) => Fs.existsSync(file))

if (fresh) console.log('WebAssembly modules are up to date.')
else {
  // `all` includes every diagram type, for the root entrypoint and the tests. Build tools,
  // servers, and the CLI download nothing, so `all.fast` trades size for speed and alone adds the
  // `theme` option. Browsers and the image Worker load the size modules, which stay without it.
  const jobs: (() => Promise<void>)[] = [
    ...['all', ...types].map((type) => async () => {
      const bytes = await build(type)
      write(Path.join(root, `src/internal/${type}.generated.ts`), source(type, bytes))
      write(Path.join(root, `src/internal/${type}.generated.wasm`), bytes)
    }),
    async () =>
      write(
        Path.join(root, 'src/internal/all.fast.generated.wasm'),
        await build('all,theme', { fast: true }),
      ),
    // Tests compare parsed models, which no published module exports.
    async () => write(modelFile, await build('all,model', { model: true })),
  ]

  // Each module has its own target directory, so builds run side by side.
  await parallel(jobs, Number(process.env.ONDINE_WASM_JOBS) || Os.availableParallelism())
  write(stampFile, stamp)
}

/**
 * A hash of everything a module build reads: the Rust sources and manifests, the toolchain names,
 * this script, and the optimizer version. An unchanged hash skips the build.
 */
function inputs(): string {
  const hash = Crypto.createHash('sha256')
  const files = [
    'Cargo.lock',
    'Cargo.toml',
    'rust-toolchain.toml',
    'scripts/build-wasm.ts',
    'node_modules/binaryen/package.json',
    ...(Fs.globSync('crates/**/*', { cwd: root }) as string[]).filter((file) =>
      Fs.statSync(Path.join(root, file)).isFile(),
    ),
  ].sort()

  // The toolchain name, not whether it is installed: jobs that restore built modules need none.
  hash.update(`${nightly}\n`)
  for (const file of files) {
    hash.update(`${file}\n`)
    hash.update(Fs.readFileSync(Path.join(root, file)))
  }
  return hash.digest('hex')
}

/** Runs `jobs` with at most `limit` in flight, failing on the first error. */
async function parallel(jobs: readonly (() => Promise<void>)[], limit: number): Promise<void> {
  let next = 0
  const worker = async () => {
    while (next < jobs.length) await jobs[next++]?.()
  }
  await Promise.all(Array.from({ length: Math.max(1, limit) }, worker))
}

/** Runs a command, rejecting when it exits unsuccessfully. */
function run(command: string, args: readonly string[], env: NodeJS.ProcessEnv): Promise<void> {
  return new Promise((resolve, reject) => {
    const child = ChildProcess.spawn(command, args, { cwd: root, env, stdio: 'inherit' })
    child.on('error', reject)
    child.on('exit', (code) =>
      code === 0
        ? resolve()
        : reject(new Error(`${command} ${args.join(' ')} exited with ${code}`)),
    )
  })
}

/** Builds the `wasm` crate with `feature`, or a comma-separated list, optimizes it, and returns its bytes. */
async function build(feature: string, options: build.Options = {}): Promise<Buffer> {
  const fast = options.fast === true
  const model = options.model === true
  const targetDir = model
    ? 'target/wasm-model'
    : `target/wasm-${feature.replaceAll(',', '-')}${fast ? '-fast' : ''}`
  const sized = !fast && !model && nightlyInstalled

  // Rebuilding `std` with `optimize_for_size` and aborting panics without formatting their messages
  // removes `core::fmt` and selects smaller `std` algorithms.
  const nightlyArgs = ['-Zbuild-std=std,panic_abort', '-Zbuild-std-features=optimize_for_size']
  const env = sized
    ? { ...process.env, RUSTFLAGS: '-Zunstable-options -Cpanic=immediate-abort' }
    : fast
      ? { ...process.env, CARGO_PROFILE_WASM_OPT_LEVEL: '3' }
      : process.env

  await run(
    'cargo',
    [
      ...(sized ? [`+${nightly}`] : []),
      'build',
      '--package',
      'wasm',
      '--no-default-features',
      '--features',
      feature,
      '--profile',
      'wasm',
      '--target',
      'wasm32-unknown-unknown',
      '--target-dir',
      targetDir,
      ...(sized ? nightlyArgs : []),
    ],
    env,
  )

  const input = Path.join(root, targetDir, 'wasm32-unknown-unknown/wasm/wasm.wasm')

  // Only tests load the model module, so it skips optimization.
  if (model) return Fs.readFileSync(input)

  const output = Path.join(root, targetDir, 'optimized.wasm')

  // Rust's wasm32 target enables these proposals by default, so the optimizer must accept them.
  const proposals = [
    '--enable-bulk-memory',
    '--enable-multivalue',
    '--enable-mutable-globals',
    '--enable-nontrapping-float-to-int',
    '--enable-reference-types',
    '--enable-sign-ext',
  ]

  // The pinned npm build of Binaryen keeps output identical across machines. Rust places the stack
  // and static data above the first kilobyte, so `--low-memory-unused` can fold address offsets.
  await run(
    Path.join(root, 'node_modules/.bin/wasm-opt'),
    [
      fast ? '-O3' : '-Oz',
      '--converge',
      '--low-memory-unused',
      ...proposals,
      '--strip-debug',
      '--strip-producers',
      input,
      '-o',
      output,
    ],
    process.env,
  )

  return Fs.readFileSync(output)
}

declare namespace build {
  type Options = {
    /** Optimizes for speed instead of size, on the stable toolchain. */
    readonly fast?: boolean | undefined
    /** Builds the unoptimized test module with the `model` export, on the stable toolchain. */
    readonly model?: boolean | undefined
  }
}

/** Whether rustup has `toolchain` with the `std` sources that `-Zbuild-std` compiles. */
function installed(toolchain: string): boolean {
  try {
    const components = ChildProcess.execFileSync(
      'rustup',
      ['component', 'list', '--installed', '--toolchain', toolchain],
      { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] },
    )
    return components.split('\n').includes('rust-src')
  } catch {
    return false
  }
}

/**
 * `package.json` with one export per generated module: `ondine/compiler/wasm/<type>` for the base64
 * module and `<type>.wasm` for its raw bytes. Zile copies each raw module into `dist/internal/`,
 * where `all.fast.wasm` also serves `ondine/compiler`. Other exports are kept, and keys are sorted.
 */
function manifest(json: string): string {
  const pkg = JSON.parse(json)
  // `ondine/compiler/wasm/all` is authored in `src/compiler/all.ts`. Every other key under
  // `./compiler/wasm/` is generated, so removed types drop out.
  const exports: Record<string, unknown> = Object.fromEntries(
    Object.entries(pkg.exports as Record<string, unknown>).filter(
      (entry) => !entry[0].startsWith('./compiler/wasm/') || entry[0] === './compiler/wasm/all',
    ),
  )

  // Condition order is observable: `default` comes last.
  for (const type of types)
    exports[`./compiler/wasm/${type}`] = {
      src: `./src/internal/${type}.generated.ts`,
      types: `./dist/internal/${type}.generated.d.ts`,
      default: `./dist/internal/${type}.generated.js`,
    }

  for (const name of ['all', 'all.fast', ...types])
    exports[`./compiler/wasm/${name}.wasm`] = {
      src: `./src/internal/${name}.generated.wasm`,
      default: `./dist/internal/${name}.generated.wasm`,
    }

  const keys = Object.keys(exports).sort((a, b) => (a < b ? -1 : a > b ? 1 : 0))
  pkg.exports = Object.fromEntries(keys.map((key) => [key, exports[key]]))
  return `${JSON.stringify(pkg, null, 2)}\n`
}

/** The generated module that inlines `bytes`. */
function source(type: string, bytes: Buffer): string {
  const subject = type === 'all' ? 'every diagram type' : `\`${type}\` diagrams`
  return `// Generated by scripts/build-wasm.ts. Do not edit.\n/** @module The size-optimized compiler module for ${subject}. */\n\n/** The module's bytes in base64, for \`create({ modules })\` from \`ondine/compiler/wasm\`. */\nexport const wasm = '${bytes.toString('base64')}'\n`
}

/** Writes a file only when its contents change, so watchers skip unchanged outputs. */
function write(file: string, contents: string | Buffer): void {
  if (Fs.existsSync(file) && Fs.readFileSync(file).equals(Buffer.from(contents))) return

  Fs.mkdirSync(Path.dirname(file), { recursive: true })
  Fs.writeFileSync(file, contents)
}
