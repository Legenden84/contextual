// Git and GitHub (gh CLI) operations for "Create pull request". Commands run with execFile
// (no shell) in the project folder, never prompt for credentials, and only touch the files
// the caller names. Nothing here runs unless the user presses a button in the PR dialog.

import { execFile } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { normalizeInputPath, PathError, resolveInside } from './fs/paths.ts'

const ENV = { ...process.env, GIT_TERMINAL_PROMPT: '0', GH_PROMPT_DISABLED: '1', GH_NO_UPDATE_NOTIFIER: '1' }

interface Run {
  ok: boolean
  stdout: string
  stderr: string
}

function run(cmd: string, args: string[], cwd: string, timeoutMs = 30_000): Promise<Run> {
  return new Promise((resolve) => {
    execFile(cmd, args, { cwd, env: ENV, timeout: timeoutMs, maxBuffer: 16 * 1024 * 1024 }, (err, stdout, stderr) => {
      resolve({ ok: !err, stdout: String(stdout), stderr: String(stderr || (err && 'code' in err && err.code === 'ENOENT' ? `${cmd} is not installed` : err?.message) || '') })
    })
  })
}

async function must(cmd: string, args: string[], cwd: string, what: string, timeoutMs?: number): Promise<string> {
  const r = await run(cmd, args, cwd, timeoutMs)
  if (!r.ok) throw new PathError(409, `${what} failed: ${r.stderr.trim() || 'unknown error'}`)
  return r.stdout.trim()
}

export interface FileState {
  path: string
  /** Porcelain status: M modified, A added (untracked), D deleted, or '' when unchanged vs HEAD. */
  status: 'M' | 'A' | 'D' | ''
}

export interface GitInfo {
  isRepo: boolean
  branch: string | null
  defaultBranch: string | null
  remote: string | null
  /** gh is installed and logged in for this repo's host. */
  github: boolean
  /** Why GitHub PRs aren't possible here, if they aren't. */
  problem: string | null
  files: FileState[]
  /** An open PR for the current branch, if one exists. */
  existingPr: { url: string; number: number } | null
}

function checkPaths(root: string, paths: string[]) {
  for (const p of paths) resolveInside(root, p)
}

async function fileStates(root: string, paths: string[]): Promise<FileState[]> {
  if (!paths.length) return []
  const r = await run('git', ['status', '--porcelain=v1', '-z', '--untracked-files=all', '--', ...paths], root)
  const byPath = new Map<string, FileState['status']>()
  const entries = r.stdout.split('\0').filter(Boolean)
  for (let i = 0; i < entries.length; i++) {
    const e = entries[i]
    const xy = e.slice(0, 2)
    const path = e.slice(3)
    if (xy[0] === 'R' || xy[0] === 'C') i++ // rename/copy: next entry is the source path
    byPath.set(path, xy.includes('D') ? 'D' : xy === '??' || xy.includes('A') ? 'A' : 'M')
  }
  return paths.map((path) => ({ path, status: byPath.get(path) ?? '' }))
}

export async function gitInfo(rootInput: string, paths: string[]): Promise<GitInfo> {
  const root = normalizeInputPath(rootInput)
  checkPaths(root, paths)
  const none: GitInfo = { isRepo: false, branch: null, defaultBranch: null, remote: null, github: false, problem: 'This folder is not a git repository.', files: [], existingPr: null }
  if (!(await run('git', ['rev-parse', '--is-inside-work-tree'], root)).ok) return none

  const branch = (await run('git', ['rev-parse', '--abbrev-ref', 'HEAD'], root)).stdout.trim() || null
  const remote = (await run('git', ['remote', 'get-url', 'origin'], root)).stdout.trim() || null
  const files = await fileStates(root, paths)

  let problem: string | null = null
  let github = false
  let defaultBranch: string | null = null
  if (!remote) problem = 'The repository has no "origin" remote.'
  else {
    const view = await run('gh', ['repo', 'view', '--json', 'defaultBranchRef', '-q', '.defaultBranchRef.name'], root)
    if (view.ok) {
      github = true
      defaultBranch = view.stdout.trim() || null
    } else {
      problem = /not installed/.test(view.stderr)
        ? 'The GitHub CLI (gh) is not installed. Install it and run "gh auth login".'
        : `GitHub CLI can't access this repository: ${view.stderr.trim().split('\n')[0]}`
    }
  }
  if (!defaultBranch) {
    const head = (await run('git', ['symbolic-ref', '--short', 'refs/remotes/origin/HEAD'], root)).stdout.trim()
    defaultBranch = head.replace(/^origin\//, '') || null
  }

  let existingPr: GitInfo['existingPr'] = null
  if (github && branch && branch !== defaultBranch) {
    const pr = await run('gh', ['pr', 'view', branch, '--json', 'url,number,state'], root)
    if (pr.ok) {
      const j = JSON.parse(pr.stdout) as { url: string; number: number; state: string }
      if (j.state === 'OPEN') existingPr = { url: j.url, number: j.number }
    }
  }
  return { isRepo: true, branch, defaultBranch, remote, github, problem, files, existingPr }
}

/** Unified diff of the given files against HEAD (new files shown in full), capped for the model. */
export async function diffFor(rootInput: string, paths: string[], maxChars = 60_000): Promise<string> {
  const root = normalizeInputPath(rootInput)
  checkPaths(root, paths)
  const states = await fileStates(root, paths)
  const parts: string[] = []
  const tracked = states.filter((s) => s.status === 'M' || s.status === 'D').map((s) => s.path)
  if (tracked.length) parts.push((await run('git', ['diff', 'HEAD', '--', ...tracked], root)).stdout)
  for (const s of states.filter((x) => x.status === 'A')) {
    try {
      const text = await readFile(resolveInside(root, s.path), 'utf8')
      parts.push(`new file: ${s.path}\n${text.split('\n').map((l) => `+${l}`).join('\n')}`)
    } catch {
      parts.push(`new file: ${s.path} (unreadable)`)
    }
  }
  const diff = parts.join('\n')
  return diff.length > maxChars ? `${diff.slice(0, maxChars)}\n… (diff truncated)` : diff
}

export interface PrRequest {
  paths: string[]
  title: string
  body: string
  base: string
  draft: boolean
  /** Create and switch to this branch first (when on the default branch). */
  newBranch?: string
}

export interface PrResult {
  url: string
  number: number | null
  branch: string
  commit: string
  /** The branch already had an open PR; the commit was pushed to it. */
  updatedExisting: boolean
}

/**
 * Commits exactly `paths` on the current branch (or a new one), pushes it, and opens a PR.
 * Other changes in the working tree and index are left as they are.
 */
export async function createPullRequest(rootInput: string, req: PrRequest): Promise<PrResult> {
  const root = normalizeInputPath(rootInput)
  checkPaths(root, req.paths)
  if (!req.paths.length) throw new PathError(400, 'No files selected for the pull request.')
  if (!req.title.trim()) throw new PathError(400, 'The pull request needs a title.')

  const info = await gitInfo(root, req.paths)
  if (!info.isRepo || !info.github) throw new PathError(409, info.problem ?? 'GitHub is not available for this repository.')
  if (!info.files.some((f) => f.status)) throw new PathError(409, 'None of the selected files has uncommitted changes.')

  let branch = info.branch
  if (!branch || branch === 'HEAD') throw new PathError(409, 'Not on a branch (detached HEAD).')
  if (branch === info.defaultBranch && !req.newBranch)
    throw new PathError(409, `You're on ${branch}, the default branch. Create a new branch for the pull request.`)

  const original = branch
  if (req.newBranch) {
    await must('git', ['check-ref-format', '--branch', req.newBranch], root, 'Branch name check')
    await must('git', ['switch', '-c', req.newBranch], root, `Creating branch ${req.newBranch}`)
    branch = req.newBranch
  }

  const changed = info.files.filter((f) => f.status)
  const present = changed.filter((f) => f.status !== 'D').map((f) => f.path)
  const deleted = changed.filter((f) => f.status === 'D').map((f) => f.path)
  try {
    // Stage only these paths, then commit only them; other staged changes stay staged.
    if (present.length) await must('git', ['add', '--', ...present], root, 'Staging files')
    if (deleted.length) await must('git', ['rm', '-q', '--cached', '--ignore-unmatch', '--', ...deleted], root, 'Staging deletions')
    await must(
      'git',
      ['commit', '-m', req.title.trim(), '-m', req.body.trim() || ' ', '--only', '--', ...changed.map((f) => f.path)],
      root,
      'Commit',
    )
  } catch (err) {
    // Nothing was committed: go back to where the user was (their changes come along unchanged).
    if (req.newBranch) {
      await run('git', ['switch', original], root)
      await run('git', ['branch', '-D', req.newBranch], root)
    }
    throw err
  }
  const commit = await must('git', ['rev-parse', '--short', 'HEAD'], root, 'Reading the commit')
  await must('git', ['push', '-u', 'origin', 'HEAD'], root, 'Push', 120_000)

  if (info.existingPr && !req.newBranch)
    return { url: info.existingPr.url, number: info.existingPr.number, branch, commit, updatedExisting: true }

  const args = ['pr', 'create', '--base', req.base, '--head', branch, '--title', req.title.trim(), '--body', req.body.trim() || ' ']
  if (req.draft) args.push('--draft')
  const url = (await must('gh', args, root, 'Creating the pull request', 60_000)).split('\n').pop()!.trim()
  const number = Number(/\/pull\/(\d+)/.exec(url)?.[1]) || null
  return { url, number, branch, commit, updatedExisting: false }
}
