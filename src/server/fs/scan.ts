// Scans an opened project folder into the explorer tree: token estimates, exclusions
// (globs, .gitignore, binaries, size, hidden), the git branch and instruction files.

import { open, readdir, readFile, stat } from 'node:fs/promises'
import path from 'node:path'
import ignore, { type Ignore } from 'ignore'
import picomatch from 'picomatch'
import { condense, CONDENSE_MIN_BYTES, isCondensable } from './condense.ts'
import { IMAGE_EXT, MAX_IMAGE_BYTES } from './images.ts'
import { normalizeInputPath, PathError, toRel } from './paths.ts'

export interface ScanOptions {
  exclusions: string[]
  maxFileSizeKb: number
  respectGitignore: boolean
  skipBinary: boolean
  includeHidden: boolean
}

export interface ScanNode {
  name: string
  path: string
  kind: 'dir' | 'file'
  tokens?: number
  children?: ScanNode[]
  excluded?: string
  mtimeMs?: number
  /** Sent to the model as a picture, not as text. */
  image?: boolean
  /** Large HTML/SVG sent condensed: `tokens` is the condensed size; this is the original. */
  condensedFrom?: number
}

/** Tokens a picture costs at high detail (OpenAI/Azure vision); used when dimensions are unknown. */
export const IMAGE_TOKENS_ESTIMATE = 765

export interface RuleFile {
  path: string
  type: 'Rule' | 'Skill'
  tokens: number
  content: string
}

export interface ScanResult {
  name: string
  root: string
  branch: string | null
  tree: ScanNode[]
  rules: RuleFile[]
  files: number
  /** The scan stopped at MAX_ENTRIES. */
  truncated: boolean
}

/** Larger HTML/SVG files aren't condensed during the scan (same cap as reading). */
const MAX_CONDENSE_BYTES = 2 * 1024 * 1024
/** Stop listing after this many entries, so a huge folder can't hang the app. */
const MAX_ENTRIES = 20000
/** Never listed or traversed: VCS internals and Contextual's own backups. */
const ALWAYS_SKIP = new Set(['.git', '.contextual'])
/** Hidden folders still worth showing: they hold instruction files. */
const HIDDEN_ALLOWED = new Set(['.github', '.cursor', '.claude', '.gitignore'])
const LOCK_FILES = new Set([
  'package-lock.json',
  'yarn.lock',
  'pnpm-lock.yaml',
  'poetry.lock',
  'Pipfile.lock',
  'packages.lock.json',
  'Cargo.lock',
  'composer.lock',
  'Gemfile.lock',
  'bun.lockb',
])
/** .env, .env.local, .env.production… but not .env.example / .sample / .template. */
const SECRET_FILE = /^\.env(?!\.(example|sample|template)$)(\..+)?$/
const BINARY_EXT =
  /\.(png|jpe?g|gif|bmp|ico|webp|avif|tiff?|psd|pdf|zip|gz|tgz|7z|rar|tar|jar|war|dll|exe|pdb|so|dylib|a|lib|o|obj|class|pyc|wasm|woff2?|ttf|otf|eot|mp3|mp4|mov|avi|webm|wav|ogg|flac|sqlite|db|mdb|bin|dat|nupkg|snk|pfx)$/i

/** Rough token estimate: about 4 bytes of source per token. */
export const estimateTokens = (bytes: number) => Math.max(1, Math.round(bytes / 4))

async function looksBinary(file: string): Promise<boolean> {
  const fh = await open(file, 'r')
  try {
    const buf = Buffer.alloc(8000)
    const { bytesRead } = await fh.read(buf, 0, buf.length, 0)
    return buf.subarray(0, bytesRead).includes(0)
  } finally {
    await fh.close()
  }
}

async function gitBranch(root: string): Promise<string | null> {
  try {
    let gitDir = path.join(root, '.git')
    const info = await stat(gitDir)
    if (info.isFile()) {
      // Worktree or submodule: ".git" is a file pointing at the real git dir.
      const m = /gitdir:\s*(.+)/.exec(await readFile(gitDir, 'utf8'))
      if (!m) return null
      gitDir = path.resolve(root, m[1].trim())
    }
    const head = (await readFile(path.join(gitDir, 'HEAD'), 'utf8')).trim()
    const ref = /^ref: refs\/heads\/(.+)$/.exec(head)
    return ref ? ref[1] : head.slice(0, 7)
  } catch {
    return null
  }
}

async function readRule(root: string, rel: string, type: RuleFile['type']): Promise<RuleFile | null> {
  try {
    const abs = path.join(root, rel)
    const info = await stat(abs)
    if (!info.isFile() || info.size > 200 * 1024) return null
    const content = await readFile(abs, 'utf8')
    return { path: rel, type, tokens: estimateTokens(Buffer.byteLength(content)), content }
  } catch {
    return null
  }
}

async function listNames(dir: string): Promise<string[]> {
  try {
    return (await readdir(dir)).sort()
  } catch {
    return []
  }
}

/** AGENTS.md, CLAUDE.md, .github/copilot-instructions.md, .cursor/rules/*, .claude/skills/x/SKILL.md */
async function findRules(root: string): Promise<RuleFile[]> {
  const candidates: [string, RuleFile['type']][] = [
    ['AGENTS.md', 'Rule'],
    ['CLAUDE.md', 'Rule'],
    ['.github/copilot-instructions.md', 'Rule'],
  ]
  for (const n of await listNames(path.join(root, '.cursor/rules')))
    if (/\.(md|mdc)$/i.test(n)) candidates.push([`.cursor/rules/${n}`, 'Rule'])
  for (const n of await listNames(path.join(root, '.claude/skills')))
    candidates.push([`.claude/skills/${n}/SKILL.md`, 'Skill'])
  const found = await Promise.all(candidates.map(([rel, type]) => readRule(root, rel, type)))
  return found.filter((r): r is RuleFile => r !== null)
}

export async function scanProject(input: string, opts: ScanOptions): Promise<ScanResult> {
  const root = normalizeInputPath(input)
  try {
    if (!(await stat(root)).isDirectory()) throw new Error()
  } catch {
    throw new PathError(404, `Folder not found: ${root}`)
  }

  const globs = opts.exclusions
    .map((g) => g.trim())
    .filter(Boolean)
    .map((g) => ({ glob: g, match: picomatch(g, { dot: true, basename: !g.includes('/') }) }))

  let gitignore: Ignore | null = null
  if (opts.respectGitignore) {
    try {
      gitignore = ignore().add(await readFile(path.join(root, '.gitignore'), 'utf8'))
    } catch {
      gitignore = null
    }
  }

  const maxBytes = opts.maxFileSizeKb * 1024
  let count = 0
  let files = 0
  let truncated = false

  const exclusionFor = (rel: string, name: string, isDir: boolean): string | undefined => {
    const hit = globs.find((g) => g.match(rel) || (isDir && g.match(`${rel}/x`)))
    if (hit) return isDir ? `ignored: ${name}` : LOCK_FILES.has(name) ? 'lock file' : `ignored: ${hit.glob}`
    if (gitignore?.ignores(isDir ? `${rel}/` : rel)) return '.gitignore'
    if (!isDir && SECRET_FILE.test(name)) return 'secrets'
    if (!opts.includeHidden && name.startsWith('.') && !HIDDEN_ALLOWED.has(name)) return 'hidden'
    return undefined
  }

  const walk = async (dir: string): Promise<ScanNode[]> => {
    let entries
    try {
      entries = await readdir(dir, { withFileTypes: true })
    } catch {
      return []
    }
    entries.sort((a, b) => {
      const ad = a.isDirectory() ? 0 : 1
      const bd = b.isDirectory() ? 0 : 1
      return ad - bd || a.name.localeCompare(b.name, undefined, { sensitivity: 'base' })
    })

    const out: ScanNode[] = []
    for (const e of entries) {
      if (ALWAYS_SKIP.has(e.name)) continue
      if (count >= MAX_ENTRIES) {
        truncated = true
        break
      }
      count++
      const abs = path.join(dir, e.name)
      const rel = toRel(root, abs)

      if (e.isSymbolicLink()) {
        out.push({ name: e.name, path: rel, kind: 'file', excluded: 'symlink' })
        continue
      }
      if (e.isDirectory()) {
        const excluded = exclusionFor(rel, e.name, true)
        out.push({ name: e.name, path: rel, kind: 'dir', excluded, children: excluded ? [] : await walk(abs) })
        continue
      }
      if (!e.isFile()) continue

      let info
      try {
        info = await stat(abs)
      } catch {
        continue
      }
      let excluded = exclusionFor(rel, e.name, false)
      const image = IMAGE_EXT.test(e.name)
      let tokens = image ? IMAGE_TOKENS_ESTIMATE : estimateTokens(info.size)
      let condensedFrom: number | undefined

      // Large HTML/SVG: measure the condensed copy, which is what gets sent.
      if (!excluded && !image && isCondensable(e.name) && info.size >= CONDENSE_MIN_BYTES && info.size <= MAX_CONDENSE_BYTES) {
        try {
          const small = condense(await readFile(abs, 'utf8'))
          condensedFrom = tokens
          tokens = estimateTokens(Buffer.byteLength(small))
        } catch {
          // Unreadable: fall back to the raw size.
        }
      }

      const sentBytes = condensedFrom ? tokens * 4 : info.size
      if (!excluded && image && info.size > MAX_IMAGE_BYTES) excluded = `too large (${Math.round(info.size / 1024)} KB)`
      if (!excluded && !image && sentBytes > maxBytes) excluded = `too large (${Math.round(info.size / 1024)} KB)`
      if (!excluded && !image && opts.skipBinary && (BINARY_EXT.test(e.name) || (await looksBinary(abs).catch(() => true))))
        excluded = 'binary'
      if (!excluded) files++
      out.push({
        name: e.name,
        path: rel,
        kind: 'file',
        tokens,
        excluded,
        mtimeMs: Math.round(info.mtimeMs),
        ...(image && { image: true }),
        ...(condensedFrom && { condensedFrom }),
      })
    }
    return out
  }

  const tree = await walk(root)
  return {
    name: path.basename(root) || root,
    root,
    branch: await gitBranch(root),
    tree,
    rules: await findRules(root),
    files,
    truncated,
  }
}
