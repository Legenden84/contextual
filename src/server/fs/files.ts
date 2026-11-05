// Reading project files for context and previews, and writing applied changes.
// Writes happen only from an explicit Apply in the UI; the model never calls this.

import { copyFile, mkdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import path from 'node:path'
import { normalizeInputPath, PathError, resolveInside, resolveInsideReal } from './paths.ts'

export interface FileContent {
  path: string
  content?: string
  mtimeMs?: number
  error?: string
}

/** Largest file sent as context or shown in a preview. */
const MAX_READ_BYTES = 2 * 1024 * 1024

export async function readFiles(rootInput: string, paths: string[]): Promise<FileContent[]> {
  const root = normalizeInputPath(rootInput)
  return Promise.all(
    paths.map(async (rel): Promise<FileContent> => {
      try {
        const abs = await resolveInsideReal(root, rel)
        const info = await stat(abs)
        if (!info.isFile()) return { path: rel, error: 'not a file' }
        if (info.size > MAX_READ_BYTES) return { path: rel, error: `too large (${Math.round(info.size / 1024)} KB)` }
        const buf = await readFile(abs)
        if (buf.subarray(0, 8000).includes(0)) return { path: rel, error: 'binary' }
        return { path: rel, content: buf.toString('utf8'), mtimeMs: Math.round(info.mtimeMs) }
      } catch (err) {
        if (err instanceof PathError) return { path: rel, error: err.message }
        return { path: rel, error: 'not found' }
      }
    }),
  )
}

export interface ApplyChange {
  path: string
  status: 'A' | 'M' | 'D'
  /** New contents for A / M. */
  content?: string
  /** Contents the model saw, for M / D. Writing is refused if the file changed since. */
  expected?: string
}

export interface ApplyResult {
  created: number
  overwritten: number
  deleted: number
  backupId: string | null
  backupPath: string | null
}

interface Manifest {
  /** Files that did not exist before the apply (undo deletes them). */
  created: string[]
  /** Files that existed and were copied into the backup (undo restores them). */
  replaced: string[]
}

const BACKUP_DIR = '.contextual/backups'

async function readIfExists(abs: string): Promise<string | null> {
  try {
    return await readFile(abs, 'utf8')
  } catch {
    return null
  }
}

/**
 * Writes the selected changes. Every file is checked first; if any check fails nothing is written.
 * With `backup`, existing files are copied to .contextual/backups/<id>/ so the apply can be undone.
 */
export async function applyChanges(rootInput: string, changes: ApplyChange[], backup: boolean): Promise<ApplyResult> {
  const root = normalizeInputPath(rootInput)
  if (!changes.length) throw new PathError(400, 'No changes to apply.')

  const plan = await Promise.all(
    changes.map(async (c) => ({ ...c, abs: await resolveInsideReal(root, c.path), current: null as string | null })),
  )
  const problems: string[] = []
  for (const p of plan) {
    p.current = await readIfExists(p.abs)
    if (p.status === 'A' && p.current !== null) problems.push(`${p.path} already exists`)
    if (p.status !== 'A' && p.current === null) problems.push(`${p.path} no longer exists`)
    if (p.status !== 'A' && p.current !== null && p.expected !== undefined && p.current !== p.expected)
      problems.push(`${p.path} changed on disk since it was sent`)
    if (p.status !== 'D' && typeof p.content !== 'string') problems.push(`${p.path} has no content`)
  }
  if (problems.length) throw new PathError(409, `Nothing was written. ${problems.join('; ')}.`)

  let backupId: string | null = null
  let backupRoot: string | null = null
  if (backup) {
    backupId = new Date().toISOString().replace(/[:.]/g, '-')
    backupRoot = path.join(root, BACKUP_DIR, backupId)
    await mkdir(backupRoot, { recursive: true })
    // Keep backups out of git without touching the project's own .gitignore.
    await writeFile(path.join(root, '.contextual', '.gitignore'), '*\n')
    const manifest: Manifest = { created: [], replaced: [] }
    for (const p of plan) {
      if (p.current === null) manifest.created.push(p.path)
      else {
        const dest = path.join(backupRoot, 'files', p.path)
        await mkdir(path.dirname(dest), { recursive: true })
        await copyFile(p.abs, dest)
        manifest.replaced.push(p.path)
      }
    }
    await writeFile(path.join(backupRoot, 'manifest.json'), JSON.stringify(manifest, null, 2))
  }

  const result: ApplyResult = { created: 0, overwritten: 0, deleted: 0, backupId, backupPath: backupRoot }
  for (const p of plan) {
    if (p.status === 'D') {
      await rm(p.abs)
      result.deleted++
    } else {
      await mkdir(path.dirname(p.abs), { recursive: true })
      await writeFile(p.abs, p.content!)
      if (p.status === 'A') result.created++
      else result.overwritten++
    }
  }
  return result
}

/** Restores files from a backup: re-creates replaced/deleted files and removes created ones. */
export async function undoApply(rootInput: string, backupId: string): Promise<{ restored: number; removed: number }> {
  const root = normalizeInputPath(rootInput)
  if (!/^[\w-]+$/.test(backupId)) throw new PathError(400, 'Invalid backup id.')
  const backupRoot = path.join(root, BACKUP_DIR, backupId)
  let manifest: Manifest
  try {
    manifest = JSON.parse(await readFile(path.join(backupRoot, 'manifest.json'), 'utf8')) as Manifest
  } catch {
    throw new PathError(404, `Backup not found: ${backupId}`)
  }

  let restored = 0
  let removed = 0
  for (const rel of manifest.replaced) {
    const abs = resolveInside(root, rel)
    await mkdir(path.dirname(abs), { recursive: true })
    await copyFile(path.join(backupRoot, 'files', rel), abs)
    restored++
  }
  for (const rel of manifest.created) {
    await rm(resolveInside(root, rel), { force: true })
    removed++
  }
  return { restored, removed }
}
