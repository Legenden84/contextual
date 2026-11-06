// Records what the TypeScript server's fs functions (src/server/fs/*.ts) return for the fixture
// projects, as the Phase 0 parity baseline for the Rust port (crates/core/tests/fs_parity.rs).
//
// Run from the repository root with Node 24+ (type stripping) and node_modules installed:
//   node crates/core/tests/fixtures/record-fs.mjs
// Writes crates/core/tests/fixtures/fs/{scan,read,browse,apply}.json.
//
// Each run copies fixtures/projects/* into a fresh temp folder and adds what can't be committed
// (see `materialize`, mirrored in fs_parity.rs). Output is normalised so it doesn't depend on the
// machine: the temp folder becomes `<TMP>`, `mtimeMs` values become "<mtime>", undo ids become
// "<backupId>", and the folder browser's `places` (Home, drives) are left out.

import { createHash } from 'node:crypto'
import fs from 'node:fs'
import os from 'node:os'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const here = path.dirname(fileURLToPath(import.meta.url))
const repo = path.resolve(here, '../../../..')
const out = path.join(here, 'fs')

const { scanProject } = await import(path.join(repo, 'src/server/fs/scan.ts'))
const { readFiles, applyChanges, undoApply } = await import(path.join(repo, 'src/server/fs/files.ts'))
const { listDirs } = await import(path.join(repo, 'src/server/fs/browse.ts'))
const { PathError } = await import(path.join(repo, 'src/server/fs/paths.ts'))

const defaultExclusions = [
  '**/node_modules/**',
  '**/.venv/**',
  '**/venv/**',
  '**/__pycache__/**',
  '**/bin/**',
  '**/obj/**',
  '**/dist/**',
  '**/build/**',
  '**/.git/**',
  'package-lock.json',
  'yarn.lock',
  'pnpm-lock.yaml',
  'poetry.lock',
  'Pipfile.lock',
  'packages.lock.json',
  'Cargo.lock',
]
const defaults = { exclusions: defaultExclusions, maxFileSizeKb: 256, respectGitignore: true, skipBinary: true, includeHidden: false }

/** Copies the committed fixtures into `tmp` and adds generated parts. Mirrored in fs_parity.rs. */
function materialize(tmp) {
  fs.cpSync(path.join(here, 'projects/basic'), path.join(tmp, 'basic'), { recursive: true })
  fs.cpSync(path.join(here, 'projects/skill-root'), path.join(tmp, 'skill-root'), { recursive: true })
  const b = path.join(tmp, 'basic')
  fs.mkdirSync(path.join(b, '.git/refs/heads'), { recursive: true })
  fs.writeFileSync(path.join(b, '.git/HEAD'), 'ref: refs/heads/feature/parity\n')
  fs.symlinkSync('src', path.join(b, 'link-to-src'))
  fs.symlinkSync('main.ts', path.join(b, 'src/link.ts'))
  fs.mkdirSync(path.join(tmp, 'outside'))
  fs.writeFileSync(path.join(tmp, 'outside/secret.txt'), 'outside\n')
  fs.symlinkSync('../outside', path.join(b, 'escape'))
  fs.writeFileSync(path.join(b, 'big.txt'), 'abcdefghi\n'.repeat(30 * 1024))
  fs.writeFileSync(path.join(b, 'huge.txt'), ('x'.repeat(99) + '\n').repeat(25 * 1024))
  fs.writeFileSync(path.join(b, 'web/huge.html'), '<html>\n' + '<p>paragraph</p>\n'.repeat(130 * 1024))
  // A git worktree: `.git` is a file pointing at the real git dir, which has a detached HEAD.
  fs.mkdirSync(path.join(tmp, 'worktree'))
  fs.writeFileSync(path.join(tmp, 'worktree/.git'), 'gitdir: ../wt-gitdir\n')
  fs.writeFileSync(path.join(tmp, 'worktree/a.txt'), 'a\n')
  fs.mkdirSync(path.join(tmp, 'wt-gitdir'))
  fs.writeFileSync(path.join(tmp, 'wt-gitdir/HEAD'), '0123456789abcdef0123456789abcdef01234567\n')
}

const tmps = []
process.on('exit', () => tmps.forEach((t) => fs.rmSync(t, { recursive: true, force: true })))

function newTmp() {
  const tmp = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), 'ctx-parity-')))
  tmps.push(tmp)
  materialize(tmp)
  return tmp
}

function normalize(value, tmp) {
  if (typeof value === 'string') return value.split(tmp).join('<TMP>')
  if (Array.isArray(value)) return value.map((v) => normalize(v, tmp))
  if (value && typeof value === 'object') {
    const o = {}
    for (const [k, v] of Object.entries(value)) {
      if (k === 'mtimeMs') o[k] = '<mtime>'
      else if (k === 'backupId' && typeof v === 'string') o[k] = '<backupId>'
      else if (k === 'places') continue
      else o[k] = normalize(v, tmp)
    }
    return o
  }
  return value
}

const errorOf = (err) =>
  err instanceof PathError ? { status: err.status, message: err.message } : { status: 500, message: String(err.message) }

async function outcome(fn, tmp) {
  try {
    return { result: normalize(await fn(), tmp) }
  } catch (err) {
    return { error: normalize(errorOf(err), tmp) }
  }
}

const sub = (s, tmp) => s.split('<TMP>').join(tmp)

function write(name, data) {
  fs.mkdirSync(out, { recursive: true })
  fs.writeFileSync(path.join(out, name), JSON.stringify(data, null, 2) + '\n')
  console.log(`wrote fs/${name}`)
}

// ---- scan --------------------------------------------------------------------------------
{
  const tmp = newTmp()
  const cases = [
    { input: '<TMP>/basic', options: defaults },
    {
      input: '"<TMP>/basic/"',
      options: {
        exclusions: ['**/node_modules/**', '  ', '*.MD', 'src/nested/**', 'web/*.svg', '**/*.{png,woff2}', '!**/*', 'Cargo.lock'],
        maxFileSizeKb: 1,
        respectGitignore: false,
        skipBinary: false,
        includeHidden: true,
      },
    },
    { input: '<TMP>/basic', options: { ...defaults, exclusions: [], maxFileSizeKb: 12 } },
    { input: '<TMP>/skill-root', options: defaults },
    { input: '<TMP>/worktree', options: defaults },
    { input: '<TMP>/missing', options: defaults },
    { input: '<TMP>/basic/AGENTS.md', options: defaults },
  ]
  for (const c of cases) Object.assign(c, await outcome(() => scanProject(sub(c.input, tmp), c.options), tmp))
  write('scan.json', cases)
}

// ---- read --------------------------------------------------------------------------------
{
  const tmp = newTmp()
  const paths = [
    'src/main.ts',
    'src/crlf.cs',
    'src/bom.txt',
    'src/no-newline.txt',
    'src/mixed.txt',
    'src/invalid-utf8.txt',
    'src/unicode-é.txt',
    'CLAUDE.md',
    'assets/logo.png',
    'assets/photo.JPG',
    'assets/data.bin',
    'assets/nul.txt',
    'web/small.html',
    'huge.txt',
    'src',
    'missing.txt',
    '../outside/secret.txt',
    '/etc/passwd',
    '',
    'escape/secret.txt',
    'link-to-src/main.ts',
    'src/link.ts',
    'src/../src/main.ts',
  ]
  const cases = [
    { root: '<TMP>/basic', paths, condense: false },
    { root: '<TMP>/basic', paths: ['web/page.html', 'web/icon.svg', 'web/small.html'], condense: false },
    { root: '<TMP>/basic', paths: ['web/page.html', 'web/icon.svg', 'web/small.html', 'src/main.ts'], condense: true },
    { root: '<TMP>/missing', paths: ['a.txt', '../x'], condense: false },
  ]
  for (const c of cases)
    Object.assign(c, await outcome(() => readFiles(sub(c.root, tmp), c.paths, { condense: c.condense }), tmp))
  write('read.json', cases)
}

// ---- browse ------------------------------------------------------------------------------
{
  const tmp = newTmp()
  const cases = [
    { input: '<TMP>/basic', hidden: false },
    { input: '<TMP>/basic', hidden: true },
    { input: '<TMP>/basic/src', hidden: false },
    { input: '  "<TMP>/basic/web/" ', hidden: false },
    { input: '<TMP>/basic/src/../tools', hidden: false },
    { input: '<TMP>/missing', hidden: false },
    { input: '<TMP>/basic/AGENTS.md', hidden: false },
  ]
  for (const c of cases) Object.assign(c, await outcome(() => listDirs(sub(c.input, tmp), c.hidden), tmp))
  write('browse.json', cases)
}

// ---- apply / undo ------------------------------------------------------------------------
{
  const tmp = newTmp()
  process.env.XDG_CACHE_HOME = path.join(tmp, 'cache')
  const undoBase = path.join(tmp, 'cache/contextual/undo')
  const root = path.join(tmp, 'basic')
  const tracked = [
    'src/crlf.cs',
    'src/bom.txt',
    'src/no-newline.txt',
    'src/mixed.txt',
    'src/lower.ts',
    'src/main.ts',
    'new/dir/deep/file.txt',
    'top.txt',
    'nob.txt',
  ]
  const trackedDirs = ['new', 'new/dir', 'new/dir/deep']
  const state = () => {
    const files = {}
    for (const t of tracked) {
      const p = path.join(root, t)
      files[t] = fs.existsSync(p) ? fs.readFileSync(p).toString('base64') : null
    }
    const dirs = {}
    for (const d of trackedDirs) dirs[d] = fs.existsSync(path.join(root, d))
    return { files, dirs }
  }
  const snapshots = () => {
    // One project hash folder; list its snapshots with their manifest and saved files.
    if (!fs.existsSync(undoBase)) return []
    const list = []
    for (const hash of fs.readdirSync(undoBase))
      for (const id of fs.readdirSync(path.join(undoBase, hash)).sort()) {
        const dir = path.join(undoBase, hash, id)
        const files = {}
        const walk = (d, rel) => {
          for (const e of fs.readdirSync(d, { withFileTypes: true }).sort((a, b) => (a.name < b.name ? -1 : 1))) {
            const r = rel ? `${rel}/${e.name}` : e.name
            if (e.isDirectory()) walk(path.join(d, e.name), r)
            else files[r] = fs.readFileSync(path.join(d, e.name)).toString('base64')
          }
        }
        walk(path.join(dir, 'files'), '')
        const manifestPath = path.join(dir, 'manifest.json')
        const manifest = fs.existsSync(manifestPath) ? JSON.parse(fs.readFileSync(manifestPath, 'utf8')) : null
        list.push({ hashMatches: hash === createHash('sha1').update(root).digest('hex').slice(0, 16), manifest, files })
      }
    return list
  }

  let lastId = null
  const steps = []
  const run = async (step) => {
    let res
    if (step.op === 'apply') {
      res = await outcome(async () => {
        const r = await applyChanges(root, step.changes, step.backup)
        lastId = r.backupId
        return r
      }, tmp)
    } else if (step.op === 'undo') {
      const id = step.backupId === '$last' ? lastId : step.backupId
      res = await outcome(() => undoApply(root, id), tmp)
    } else if (step.op === 'write') {
      fs.writeFileSync(path.join(root, step.path), step.content)
      res = {}
    }
    steps.push({ ...step, ...res, after: state(), snapshots: normalize(snapshots(), tmp) })
  }

  await run({
    op: 'apply',
    backup: true,
    changes: [
      { path: 'src/main.ts', status: 'A', content: 'x' },
      { path: 'gone.txt', status: 'M', content: 'x' },
      { path: 'src/crlf.cs', status: 'M', content: 'x', expected: 'stale' },
      { path: 'src/bom.txt', status: 'M' },
      { path: 'missing.txt', status: 'D' },
      { path: 'top.txt', status: 'A', content: 'fine' },
    ],
  })
  await run({ op: 'apply', backup: true, changes: [{ path: '../x.txt', status: 'A', content: 'x' }] })
  await run({ op: 'apply', backup: true, changes: [{ path: 'escape/secret.txt', status: 'M', content: 'x' }] })
  await run({ op: 'apply', backup: true, changes: [{ path: 'escape/new.txt', status: 'A', content: 'x' }] })
  await run({ op: 'apply', backup: true, changes: [{ path: '/abs.txt', status: 'A', content: 'x' }] })
  await run({ op: 'apply', backup: true, changes: [] })
  await run({
    op: 'apply',
    backup: true,
    changes: [
      { path: 'src/crlf.cs', status: 'M', content: 'namespace Y;\n\npublic class B\n{\n}\n', expected: 'namespace X;\r\n\r\npublic class A\r\n{\r\n}\r\n' },
      { path: 'src/bom.txt', status: 'M', content: 'BOM line 1\nchanged\n', expected: 'BOM line 1\nline 2\n' },
      { path: 'src/no-newline.txt', status: 'M', content: 'new last line\n' },
      { path: 'src/mixed.txt', status: 'M', content: '1\n2\n3' },
      { path: 'new/dir/deep/file.txt', status: 'A', content: 'deep new' },
      { path: 'top.txt', status: 'A', content: 'top\r\nwin' },
      { path: 'src/lower.ts', status: 'D', expected: 'export {}\n' },
      { path: 'link-to-src/main.ts', status: 'M', content: 'via link\n' },
    ],
  })
  await run({ op: 'undo', backupId: '$last' })
  await run({ op: 'undo', backupId: '$last' })
  await run({ op: 'undo', backupId: 'bad id!' })
  await run({ op: 'apply', backup: true, changes: [{ path: 'src/main.ts', status: 'M', content: 'edited by save\n' }] })
  await run({ op: 'write', path: 'src/main.ts', content: 'edited later\n' })
  await run({ op: 'undo', backupId: '$last' })
  await run({ op: 'apply', backup: false, changes: [{ path: 'nob.txt', status: 'A', content: 'no backup' }] })
  await run({ op: 'apply', backup: true, changes: [{ path: 'src', status: 'A', content: 'folder' }] })

  const sha1 = Object.fromEntries(
    ['', 'abc', '/home/me/proj', 'C:\\Users\\Me', 'héllo — 😀', 'a'.repeat(1000)].map((s) => [s, createHash('sha1').update(s).digest('hex')]),
  )
  write('apply.json', { tracked, trackedDirs, sha1, steps })
}
