// Git and GitHub (gh CLI) operations: a VS Code-style Source Control view (status, stage,
// unstage, diff, commit, push) and pull requests that compare the branch with its base, like
// GitHub. Commands run with execFile (no shell) in the project folder and never prompt for
// credentials. Each operation runs only when the user presses its button.

import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { normalizeInputPath, PathError, resolveInside } from './fs/paths.ts'

const ENV = { ...process.env, GIT_TERMINAL_PROMPT: '0', GH_PROMPT_DISABLED: '1', GH_NO_UPDATE_NOTIFIER: '1' }
/** Larger files are listed but not shown as diffs. */
const MAX_DIFF_FILE = 400 * 1024

interface Run {
  ok: boolean
  stdout: string
  stderr: string
}

function run(cmd: string, args: string[], cwd: string, timeoutMs = 30_000): Promise<Run> {
  return new Promise((resolve) => {
    execFile(cmd, args, { cwd, env: ENV, timeout: timeoutMs, maxBuffer: 32 * 1024 * 1024 }, (err, stdout, stderr) => {
      const missing = err && 'code' in err && err.code === 'ENOENT'
      resolve({ ok: !err, stdout: String(stdout), stderr: String(stderr || (missing ? `${cmd} is not installed` : err?.message) || '') })
    })
  })
}

async function must(cmd: string, args: string[], cwd: string, what: string, timeoutMs?: number): Promise<string> {
  const r = await run(cmd, args, cwd, timeoutMs)
  if (!r.ok) throw new PathError(409, `${what} failed: ${r.stderr.trim() || 'unknown error'}`)
  return r.stdout.trim()
}

async function repoRoot(rootInput: string): Promise<string> {
  const root = normalizeInputPath(rootInput)
  if (!(await run('git', ['rev-parse', '--is-inside-work-tree'], root)).ok) throw new PathError(409, 'This folder is not a git repository.')
  return root
}

const lf = (s: string) => s.replace(/\r\n/g, '\n')

// ------------------------------------------------------------------ status

/** VS Code-style letters: M modified, A added, D deleted, R renamed, U untracked, C conflict. */
export type ChangeLetter = 'M' | 'A' | 'D' | 'R' | 'U' | 'C'

export interface StatusEntry {
  path: string
  /** Original path of a rename. */
  from?: string
  letter: ChangeLetter
}

export interface GitStatus {
  branch: string | null
  upstream: string | null
  ahead: number
  behind: number
  staged: StatusEntry[]
  changes: StatusEntry[]
  conflicts: StatusEntry[]
}

const letterOf = (c: string): ChangeLetter => (c === 'A' ? 'A' : c === 'D' ? 'D' : c === 'R' || c === 'C' ? 'R' : 'M')

export async function gitStatus(rootInput: string): Promise<GitStatus> {
  const root = await repoRoot(rootInput)
  const out = await must('git', ['status', '--porcelain=v2', '--branch', '-z', '--untracked-files=all'], root, 'git status')
  const st: GitStatus = { branch: null, upstream: null, ahead: 0, behind: 0, staged: [], changes: [], conflicts: [] }
  const parts = out.split('\0')
  for (let i = 0; i < parts.length; i++) {
    const line = parts[i]
    if (!line) continue
    if (line.startsWith('# branch.head ')) st.branch = line.slice(14) === '(detached)' ? null : line.slice(14)
    else if (line.startsWith('# branch.upstream ')) st.upstream = line.slice(18)
    else if (line.startsWith('# branch.ab ')) {
      const [a, b] = line.slice(12).split(' ')
      st.ahead = Math.abs(Number(a))
      st.behind = Math.abs(Number(b))
    } else if (line.startsWith('1 ') || line.startsWith('2 ')) {
      const f = line.split(' ')
      const xy = f[1]
      const path = line.startsWith('1 ') ? f.slice(8).join(' ') : f.slice(9).join(' ')
      const from = line.startsWith('2 ') ? parts[++i] : undefined
      if (xy[0] !== '.') st.staged.push({ path, from, letter: letterOf(xy[0]) })
      if (xy[1] !== '.') st.changes.push({ path, letter: letterOf(xy[1]) })
    } else if (line.startsWith('? ')) st.changes.push({ path: line.slice(2), letter: 'U' })
    else if (line.startsWith('u ')) st.conflicts.push({ path: line.split(' ').slice(10).join(' '), letter: 'C' })
  }
  const byPath = (a: StatusEntry, b: StatusEntry) => a.path.localeCompare(b.path)
  st.staged.sort(byPath)
  st.changes.sort(byPath)
  return st
}

export async function stage(rootInput: string, paths: string[]) {
  const root = await repoRoot(rootInput)
  for (const p of paths) resolveInside(root, p)
  // `git add -A` stages edits, new files and deletions; it can't name a path that is gone from
  // both the worktree and the index, so those are skipped (already staged as deleted).
  const known = await run('git', ['ls-files', '--cached', '--others', '--exclude-standard', '-z', '--', ...paths], root)
  const present = new Set(known.stdout.split('\0').filter(Boolean))
  const addable = paths.filter((p) => present.has(p))
  if (addable.length) await must('git', ['add', '-A', '--', ...addable], root, 'Staging')
}

export async function unstage(rootInput: string, paths: string[]) {
  const root = await repoRoot(rootInput)
  for (const p of paths) resolveInside(root, p)
  const hasHead = (await run('git', ['rev-parse', '--verify', '-q', 'HEAD'], root)).ok
  if (hasHead) await must('git', ['restore', '--staged', '--', ...paths], root, 'Unstaging')
  else await must('git', ['rm', '-q', '--cached', '-r', '--ignore-unmatch', '--', ...paths], root, 'Unstaging')
}

async function show(root: string, spec: string): Promise<string | null> {
  const r = await run('git', ['show', spec], root)
  return r.ok ? r.stdout : null
}

export interface FileDiff {
  path: string
  old: string
  new: string
  /** Not shown: binary or too large. */
  skipped?: string
}

const skipReason = (...texts: (string | null)[]) =>
  texts.some((t) => t && t.length > MAX_DIFF_FILE) ? 'too large to show' : texts.some((t) => t?.includes('\0')) ? 'binary' : undefined

/** Staged: HEAD → index. Unstaged: index → working tree (like VS Code's two diff views). */
export async function fileDiff(rootInput: string, path: string, staged: boolean): Promise<FileDiff> {
  const root = await repoRoot(rootInput)
  const abs = resolveInside(root, path)
  let before: string | null
  let after: string | null
  if (staged) {
    before = await show(root, `HEAD:${path}`)
    after = await show(root, `:${path}`)
  } else {
    before = (await show(root, `:${path}`)) ?? (await show(root, `HEAD:${path}`))
    after = await readFile(abs, 'utf8').catch(() => null)
  }
  const skipped = skipReason(before, after)
  return { path, old: skipped ? '' : lf(before ?? ''), new: skipped ? '' : lf(after ?? ''), skipped }
}

/** Unified diff text for drafting messages, capped for the model. */
async function diffText(root: string, args: string[], maxChars = 60_000): Promise<string> {
  const d = (await run('git', ['diff', '--no-color', ...args], root)).stdout
  return d.length > maxChars ? `${d.slice(0, maxChars)}\n… (diff truncated)` : d
}

/** The staged diff (or every change, when nothing is staged), for an AI commit message. */
export async function commitDiff(rootInput: string): Promise<string> {
  const root = await repoRoot(rootInput)
  const staged = await diffText(root, ['--cached'])
  if (staged.trim()) return staged
  const st = await gitStatus(root)
  const untracked = st.changes.filter((c) => c.letter === 'U').map((c) => `new file: ${c.path}`)
  return [await diffText(root, ['HEAD']), ...untracked].join('\n')
}

export async function commit(rootInput: string, message: string, all: boolean): Promise<{ commit: string; files: number }> {
  const root = await repoRoot(rootInput)
  if (!message.trim()) throw new PathError(400, 'Write a commit message first.')
  if (all) await must('git', ['add', '-A'], root, 'Staging all changes')
  const files = (await run('git', ['diff', '--cached', '--name-only', '-z'], root)).stdout.split('\0').filter(Boolean).length
  if (!files) throw new PathError(409, 'There are no staged changes to commit.')
  await must('git', ['commit', '-m', message.trim()], root, 'Commit')
  return { commit: await must('git', ['rev-parse', '--short', 'HEAD'], root, 'Reading the commit'), files }
}

export async function push(rootInput: string): Promise<{ branch: string; upstream: string }> {
  const root = await repoRoot(rootInput)
  const st = await gitStatus(root)
  if (!st.branch) throw new PathError(409, 'Not on a branch (detached HEAD).')
  if (st.upstream) await must('git', ['push'], root, 'Push', 120_000)
  else await must('git', ['push', '-u', 'origin', 'HEAD'], root, 'Publishing the branch', 120_000)
  const after = await gitStatus(root)
  return { branch: st.branch, upstream: after.upstream ?? `origin/${st.branch}` }
}

// ------------------------------------------------------------------ branches

export interface BranchInfo {
  /** Local: "feature/x"; remote: "origin/feature/x". */
  name: string
  upstream: string | null
  sha: string
  when: string
  subject: string
}

export interface BranchList {
  current: string | null
  local: BranchInfo[]
  remote: BranchInfo[]
}

/** Local and origin branches, most recently committed first. `fetch` refreshes the remote ones first. */
export async function listBranches(rootInput: string, fetch = false): Promise<BranchList> {
  const root = await repoRoot(rootInput)
  if (fetch) await run('git', ['fetch', '--prune', '--quiet', 'origin'], root, 20_000)
  const out = await must(
    'git',
    ['for-each-ref', '--sort=-committerdate', '--format=%(refname)%00%(upstream:short)%00%(objectname:short)%00%(committerdate:relative)%00%(contents:subject)%00', 'refs/heads', 'refs/remotes/origin'],
    root,
    'Listing branches',
  )
  const f = out.split('\0')
  const list: BranchList = { current: (await gitStatus(root)).branch, local: [], remote: [] }
  for (let i = 0; i + 4 < f.length; i += 5) {
    const ref = f[i].replace(/^\n/, '')
    const info = { upstream: f[i + 1] || null, sha: f[i + 2], when: f[i + 3], subject: f[i + 4] }
    if (ref.startsWith('refs/heads/')) list.local.push({ name: ref.slice(11), ...info })
    else if (ref.startsWith('refs/remotes/') && !ref.endsWith('/HEAD')) list.remote.push({ name: ref.slice(13), ...info })
  }
  return list
}

const PARENT_KEY = (branch: string) => `branch.${branch}.contextualParent`

/** Switches to a branch. A remote branch without a local copy is checked out as a new tracking branch. */
export async function switchBranch(rootInput: string, name: string): Promise<GitStatus> {
  const root = await repoRoot(rootInput)
  if (name.startsWith('origin/')) {
    const local = name.slice(7)
    const exists = (await run('git', ['rev-parse', '--verify', '-q', `refs/heads/${local}`], root)).ok
    if (exists) await must('git', ['switch', local], root, `Switching to ${local}`)
    else await must('git', ['switch', '--track', name], root, `Checking out ${name}`)
  } else {
    await must('git', ['check-ref-format', '--branch', name], root, 'Branch name check')
    await must('git', ['switch', name], root, `Switching to ${name}`)
  }
  return gitStatus(root)
}

/** Creates a branch from `from` (default: the current branch), switches to it and records its parent. */
export async function createBranch(rootInput: string, name: string, from?: string): Promise<GitStatus> {
  const root = await repoRoot(rootInput)
  await must('git', ['check-ref-format', '--branch', name], root, 'Branch name check')
  const parent = from || (await gitStatus(root)).branch
  await must('git', ['switch', '-c', name, ...(from ? [from] : [])], root, `Creating ${name}`)
  if (parent) await run('git', ['config', PARENT_KEY(name), parent.replace(/^origin\//, '')], root)
  return gitStatus(root)
}

/** A local branch if it exists, else its origin copy. */
async function refFor(root: string, branch: string): Promise<string | null> {
  if ((await run('git', ['rev-parse', '--verify', '-q', `refs/heads/${branch}`], root)).ok) return branch
  if ((await run('git', ['rev-parse', '--verify', '-q', `refs/remotes/origin/${branch}`], root)).ok) return `origin/${branch}`
  return null
}

/** Base for comparing: the origin copy when it exists (it is what the PR merges into), else the local branch. */
async function baseRefFor(root: string, branch: string): Promise<string> {
  return (await run('git', ['rev-parse', '--verify', '-q', `refs/remotes/origin/${branch}`], root)).ok ? `origin/${branch}` : branch
}

/**
 * The branch `branch` was created from, like GitHub's suggested base: the parent recorded when
 * Contextual created it, else the branch it forked from most recently (fewest commits since the
 * fork point), else the default branch.
 */
async function parentOf(root: string, branch: string, defaultBranch: string | null, names: string[]): Promise<string | null> {
  const recorded = (await run('git', ['config', '--get', PARENT_KEY(branch)], root)).stdout.trim()
  if (recorded && recorded !== branch && (await refFor(root, recorded))) return recorded

  const head = await refFor(root, branch)
  if (!head) return defaultBranch
  const tip = (await run('git', ['rev-parse', head], root)).stdout.trim()
  let best: { name: string; ahead: number } | null = null
  for (const name of names.filter((n) => n !== branch).slice(0, 40)) {
    const ref = await refFor(root, name)
    if (!ref) continue
    const mb = (await run('git', ['merge-base', ref, head], root)).stdout.trim()
    // No shared history, or this branch is fully contained in the candidate (a child or a copy).
    if (!mb || mb === tip) continue
    const ahead = Number((await run('git', ['rev-list', '--count', `${mb}..${head}`], root)).stdout.trim())
    if (!best || ahead < best.ahead || (ahead === best.ahead && name === defaultBranch)) best = { name, ahead }
  }
  return best?.name ?? defaultBranch
}

// ------------------------------------------------------------------ pull requests

export interface Commit {
  sha: string
  subject: string
  author: string
  when: string
}

export interface Comparison {
  /** Source branch of the PR (compare). */
  branch: string | null
  /** The checked-out branch. */
  current: string | null
  defaultBranch: string | null
  /** Target branch of the PR (base). */
  base: string
  remote: string | null
  /** gh can reach the repository. */
  github: boolean
  /** Why a PR can't be created here, if it can't. */
  problem: string | null
  commits: Commit[]
  files: (FileDiff & { letter: ChangeLetter })[]
  /** Uncommitted changes (only counted when the source is the checked-out branch). */
  uncommitted: number
  /** Commits of the source branch not on origin yet (they're pushed when the PR is created). */
  unpushed: number
  existingPr: { url: string; number: number } | null
}

async function defaultBranchOf(root: string): Promise<{ name: string | null; github: boolean; problem: string | null }> {
  const view = await run('gh', ['repo', 'view', '--json', 'defaultBranchRef', '-q', '.defaultBranchRef.name'], root)
  if (view.ok) return { name: view.stdout.trim() || null, github: true, problem: null }
  const head = (await run('git', ['symbolic-ref', '--short', 'refs/remotes/origin/HEAD'], root)).stdout.trim().replace(/^origin\//, '')
  const problem = /not installed/.test(view.stderr)
    ? 'The GitHub CLI (gh) is not installed. Install it and run "gh auth login".'
    : `The GitHub CLI can't access this repository: ${view.stderr.trim().split('\n')[0]}`
  return { name: head || null, github: false, problem }
}

/**
 * What a PR from `head` (default: the checked-out branch) into `base` (default: head's parent)
 * would contain, like GitHub's compare view.
 */
export async function compare(rootInput: string, baseInput?: string, headInput?: string): Promise<Comparison> {
  const root = await repoRoot(rootInput)
  const st = await gitStatus(root)
  const remote = (await run('git', ['remote', 'get-url', 'origin'], root)).stdout.trim() || null
  if (remote) await run('git', ['fetch', '--quiet', 'origin'], root, 20_000)
  const def = remote ? await defaultBranchOf(root) : { name: null, github: false, problem: 'The repository has no "origin" remote.' }

  const head = headInput?.trim() || st.branch
  const isCurrent = !!head && head === st.branch
  const headRef = !head ? 'HEAD' : isCurrent ? 'HEAD' : await refFor(root, head)
  const branches = await listBranches(root)
  const names = [...new Set([...branches.local.map((b) => b.name), ...branches.remote.map((b) => b.name.replace(/^origin\//, ''))])]
  const base = baseInput?.trim() || (head ? await parentOf(root, head, def.name, names) : def.name) || def.name || 'main'

  const result: Comparison = {
    branch: head,
    current: st.branch,
    defaultBranch: def.name,
    base,
    remote,
    github: def.github,
    problem: def.problem,
    commits: [],
    files: [],
    uncommitted: isCurrent ? st.staged.length + st.changes.length : 0,
    unpushed: 0,
    existingPr: null,
  }
  if (!headRef) {
    result.problem ??= `Branch ${head} doesn't exist.`
    return result
  }
  const baseRef = await baseRefFor(root, base)
  const mergeBase = (await run('git', ['merge-base', baseRef, headRef], root)).stdout.trim()
  if (!mergeBase) {
    result.problem ??= `Can't compare ${head} with ${base}: the branches have no common history.`
    return result
  }

  const log = await run('git', ['log', '--format=%h%x00%s%x00%an%x00%ar%x00', `${mergeBase}..${headRef}`], root)
  const fields = log.stdout.split('\0')
  for (let i = 0; i + 3 < fields.length; i += 4)
    result.commits.push({ sha: fields[i].replace(/^\n/, '').trim(), subject: fields[i + 1], author: fields[i + 2], when: fields[i + 3] })

  if (head) {
    const onOrigin = (await run('git', ['rev-parse', '--verify', '-q', `refs/remotes/origin/${head}`], root)).ok
    result.unpushed = onOrigin ? Number((await run('git', ['rev-list', '--count', `origin/${head}..${headRef}`], root)).stdout.trim()) : result.commits.length
  }

  const names2 = (await run('git', ['diff', '--name-status', '-z', '-M', mergeBase, headRef], root)).stdout.split('\0')
  for (let i = 0; i < names2.length; i++) {
    const code = names2[i]
    if (!code) continue
    const letter: ChangeLetter = code[0] === 'R' ? 'R' : code[0] === 'A' ? 'A' : code[0] === 'D' ? 'D' : 'M'
    const from = code[0] === 'R' || code[0] === 'C' ? names2[++i] : undefined
    const path = names2[++i]
    const before = letter === 'A' ? '' : await show(root, `${mergeBase}:${from ?? path}`)
    const after = letter === 'D' ? '' : await show(root, `${headRef}:${path}`)
    const skipped = skipReason(before, after)
    result.files.push({ path, letter, old: skipped ? '' : lf(before ?? ''), new: skipped ? '' : lf(after ?? ''), skipped })
  }

  if (def.github && head && head !== def.name) {
    const pr = await run('gh', ['pr', 'view', head, '--json', 'url,number,state,baseRefName'], root)
    if (pr.ok) {
      const j = JSON.parse(pr.stdout) as { url: string; number: number; state: string }
      if (j.state === 'OPEN') result.existingPr = { url: j.url, number: j.number }
    }
  }
  return result
}

/** Diff of base...head plus the commit list, for an AI-written PR description. */
export async function prDiff(rootInput: string, base: string, head?: string): Promise<{ diff: string; commits: string; branch: string }> {
  const root = await repoRoot(rootInput)
  const c = await compare(root, base, head)
  const headRef = !c.branch || c.branch === c.current ? 'HEAD' : ((await refFor(root, c.branch)) ?? 'HEAD')
  return {
    diff: await diffText(root, [`${await baseRefFor(root, c.base)}...${headRef}`]),
    commits: c.commits.map((x) => `- ${x.subject}`).join('\n'),
    branch: c.branch ?? '',
  }
}

export interface PrRequest {
  title: string
  body: string
  base: string
  /** Source branch; default: the checked-out branch. */
  head?: string
  draft: boolean
  /** Create this branch at HEAD first (when the source is the checked-out default branch). */
  newBranch?: string
}

export interface PrResult {
  url: string
  number: number | null
  branch: string
  updatedExisting: boolean
}

/** Pushes the source branch (or a new one from HEAD) and opens a PR into `base`. */
export async function createPullRequest(rootInput: string, req: PrRequest): Promise<PrResult> {
  const root = await repoRoot(rootInput)
  if (!req.title.trim()) throw new PathError(400, 'The pull request needs a title.')
  const c = await compare(root, req.base, req.head)
  if (!c.github) throw new PathError(409, c.problem ?? 'GitHub is not available for this repository.')
  if (!c.branch) throw new PathError(409, 'Not on a branch (detached HEAD).')
  if (c.branch === c.base) throw new PathError(409, 'Choose different branches to compare.')
  if (!c.commits.length) throw new PathError(409, `There are no commits to compare between ${c.base} and ${c.branch}.`)

  let branch = c.branch
  const isCurrent = branch === c.current
  if (req.newBranch && isCurrent) {
    await must('git', ['check-ref-format', '--branch', req.newBranch], root, 'Branch name check')
    const from = branch
    await must('git', ['switch', '-c', req.newBranch], root, `Creating branch ${req.newBranch}`)
    await run('git', ['config', PARENT_KEY(req.newBranch), c.base], root)
    branch = req.newBranch
    // The commits now live on the new branch; put the default branch back where the remote has it,
    // as GitHub Desktop does, but only when that loses nothing (it was purely ahead of origin).
    if (from === c.defaultBranch) {
      const behind = (await run('git', ['rev-list', '--count', `${from}..origin/${from}`], root)).stdout.trim()
      if (behind === '0') await run('git', ['branch', '-f', from, `origin/${from}`], root)
    }
  } else if (branch === c.defaultBranch) {
    throw new PathError(409, `${branch} is the default branch. Create a new branch for the pull request.`)
  }

  // Push the source branch (a local branch that isn't checked out is pushed by name).
  const hasLocal = (await run('git', ['rev-parse', '--verify', '-q', `refs/heads/${branch}`], root)).ok
  if (hasLocal) await must('git', ['push', '-u', 'origin', isCurrent || req.newBranch ? 'HEAD' : branch], root, 'Push', 120_000)
  if (c.existingPr && !req.newBranch) return { url: c.existingPr.url, number: c.existingPr.number, branch, updatedExisting: true }

  const args = ['pr', 'create', '--base', c.base, '--head', branch, '--title', req.title.trim(), '--body', req.body.trim() || ' ']
  if (req.draft) args.push('--draft')
  const url = (await must('gh', args, root, 'Creating the pull request', 60_000)).split('\n').pop()!.trim()
  return { url, number: Number(/\/pull\/(\d+)/.exec(url)?.[1]) || null, branch, updatedExisting: false }
}
