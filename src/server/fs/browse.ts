// Folder browser for the Open folder dialog: lists sub-directories of a path on this machine.

import { access, readdir, stat } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { normalizeInputPath, PathError } from './paths.ts'

export interface DirEntry {
  name: string
  path: string
  /** Looks like a project root (has .git, a solution, package.json, pyproject…). */
  project: boolean
}

export interface DirListing {
  path: string
  parent: string | null
  dirs: DirEntry[]
  places: { label: string; path: string }[]
  /** More sub-directories exist than were listed. */
  truncated: boolean
}

const MAX_DIRS = 500
/** Project markers are only checked for the first entries, to stay fast on /mnt/c. */
const MAX_MARKER_CHECKS = 200
const MARKERS = ['.git', 'package.json', 'pyproject.toml', 'Cargo.toml', 'go.mod']

async function exists(p: string) {
  try {
    await access(p)
    return true
  } catch {
    return false
  }
}

async function looksLikeProject(dir: string): Promise<boolean> {
  for (const m of MARKERS) if (await exists(path.join(dir, m))) return true
  try {
    return (await readdir(dir)).some((n) => /\.(sln|csproj)$/i.test(n))
  } catch {
    return false
  }
}

async function places() {
  const list = [
    { label: 'Home', path: os.homedir() },
    { label: '/', path: '/' },
  ]
  if (process.platform === 'linux') {
    try {
      const drives = (await readdir('/mnt')).filter((n) => /^[a-z]$/.test(n)).sort()
      for (const d of drives) list.push({ label: `${d.toUpperCase()}:`, path: `/mnt/${d}` })
    } catch {
      // Not WSL, or /mnt is unreadable.
    }
  }
  return list
}

export async function listDirs(input: string | null, showHidden: boolean): Promise<DirListing> {
  const dir = normalizeInputPath(input)
  let info
  try {
    info = await stat(dir)
  } catch {
    throw new PathError(404, `Folder not found: ${dir}`)
  }
  if (!info.isDirectory()) throw new PathError(400, `Not a folder: ${dir}`)

  let entries
  try {
    entries = await readdir(dir, { withFileTypes: true })
  } catch {
    throw new PathError(403, `Cannot read folder: ${dir}`)
  }

  const names = entries
    .filter((e) => e.isDirectory() && (showHidden || !e.name.startsWith('.')))
    .map((e) => e.name)
    .sort((a, b) => a.localeCompare(b, undefined, { sensitivity: 'base' }))

  const shown = names.slice(0, MAX_DIRS)
  const dirs = await Promise.all(
    shown.map(async (name, i) => {
      const p = path.join(dir, name)
      return { name, path: p, project: i < MAX_MARKER_CHECKS ? await looksLikeProject(p) : false }
    }),
  )

  const parent = path.dirname(dir)
  return { path: dir, parent: parent === dir ? null : parent, dirs, places: await places(), truncated: names.length > MAX_DIRS }
}
