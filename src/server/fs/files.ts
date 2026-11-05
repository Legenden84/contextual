// Reading project files for context and previews, and saving applied changes.
// Saving behaves like "Save" in an editor: files are written in place in the working tree
// (no staging, no commit), keeping each file's line endings, BOM and final newline.
// Writes happen only from an explicit Apply in the UI; the model never calls this.

import { createHash } from 'node:crypto'
import { copyFile, mkdir, readdir, readFile, rm, rmdir, stat, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { condense, CONDENSE_MIN_BYTES, isCondensable } from './condense.ts'
import { IMAGE_EXT, imageMime, MAX_IMAGE_BYTES } from './images.ts'
import { normalizeInputPath, PathError, resolveInside, resolveInsideReal } from './paths.ts'

export interface FileContent {
  path: string
  /** Text with LF line endings and no BOM; saving restores the file's own format. */
  content?: string
  /** For image files: a data URL to send as a picture. */
  image?: string
  /** The content is a condensed copy (original size in bytes); send it as a read-only reference. */
  condensedFrom?: number
  mtimeMs?: number
  error?: string
}

export interface ReadOptions {
  /** Condense large HTML/SVG (for sending to the model; previews want the real file). */
  condense?: boolean
}

/** Largest file sent as context or shown in a preview. */
const MAX_READ_BYTES = 2 * 1024 * 1024
/** Undo snapshots kept per project; older ones are pruned. */
const MAX_SNAPSHOTS = 20

const BOM = '﻿'

interface TextFormat {
  eol: '\n' | '\r\n'
  bom: boolean
  finalNewline: boolean
}

/** Splits raw file text into LF-normalised content and the format needed to write it back. */
function decode(raw: string): { text: string; format: TextFormat } {
  const bom = raw.startsWith(BOM)
  const body = bom ? raw.slice(1) : raw
  const crlf = (body.match(/\r\n/g) ?? []).length
  const lf = (body.match(/\n/g) ?? []).length
  return {
    text: body.replace(/\r\n/g, '\n'),
    format: { eol: crlf > 0 && crlf >= lf - crlf ? '\r\n' : '\n', bom, finalNewline: /\n$/.test(body) },
  }
}

/** Writes LF text back in the given format, like an editor that keeps a file's settings. */
function encode(text: string, format: TextFormat): string {
  let body = text.replace(/\r\n/g, '\n')
  // Match the file's final-newline setting either way, so the last line doesn't show as changed.
  if (format.finalNewline && body && !body.endsWith('\n')) body += '\n'
  if (!format.finalNewline) body = body.replace(/\n$/, '')
  if (format.eol === '\r\n') body = body.replace(/\n/g, '\r\n')
  return (format.bom ? BOM : '') + body
}

const NEW_FILE_FORMAT: TextFormat = { eol: '\n', bom: false, finalNewline: true }

const sha = (s: string) => createHash('sha1').update(s).digest('hex')

export async function readFiles(rootInput: string, paths: string[], opts: ReadOptions = {}): Promise<FileContent[]> {
  const root = normalizeInputPath(rootInput)
  return Promise.all(
    paths.map(async (rel): Promise<FileContent> => {
      try {
        const abs = await resolveInsideReal(root, rel)
        const info = await stat(abs)
        if (!info.isFile()) return { path: rel, error: 'not a file' }
        const mtimeMs = Math.round(info.mtimeMs)
        if (IMAGE_EXT.test(rel)) {
          if (info.size > MAX_IMAGE_BYTES) return { path: rel, error: `too large (${Math.round(info.size / 1024)} KB)` }
          const data = (await readFile(abs)).toString('base64')
          return { path: rel, image: `data:${imageMime(rel)};base64,${data}`, mtimeMs }
        }
        if (info.size > MAX_READ_BYTES) return { path: rel, error: `too large (${Math.round(info.size / 1024)} KB)` }
        const buf = await readFile(abs)
        if (buf.subarray(0, 8000).includes(0)) return { path: rel, error: 'binary' }
        const text = decode(buf.toString('utf8')).text
        if (opts.condense && isCondensable(rel) && info.size >= CONDENSE_MIN_BYTES)
          return { path: rel, content: condense(text), condensedFrom: info.size, mtimeMs }
        return { path: rel, content: text, mtimeMs }
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
  /** New contents for A / M (LF line endings). */
  content?: string
  /** Contents the model saw, for M / D. Saving is refused if the file changed since. */
  expected?: string
}

export interface ApplyResult {
  created: number
  overwritten: number
  deleted: number
  /** Undo snapshot id, when one was kept. */
  backupId: string | null
}

interface Manifest {
  /** Files that did not exist before (undo deletes them). */
  created: string[]
  /** Files that existed and were copied into the snapshot (undo restores them). */
  replaced: string[]
  /** Folders the save created (undo removes them again if empty), deepest first. */
  createdDirs: string[]
  /** sha1 of what was written, so undo won't overwrite later edits. */
  written: Record<string, string | null>
}

/**
 * Undo snapshots live outside the project, under ~/.cache/contextual/undo/<project hash>/,
 * so a save changes nothing in the folder except the files you applied.
 */
function snapshotsDir(root: string): string {
  const cache = process.env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache')
  return path.join(cache, 'contextual', 'undo', sha(root).slice(0, 16))
}

async function readRaw(abs: string): Promise<string | null> {
  try {
    return await readFile(abs, 'utf8')
  } catch {
    return null
  }
}

async function pruneSnapshots(dir: string) {
  try {
    const ids = (await readdir(dir)).sort()
    for (const id of ids.slice(0, Math.max(0, ids.length - MAX_SNAPSHOTS)))
      await rm(path.join(dir, id), { recursive: true, force: true })
  } catch {
    // Pruning is best effort.
  }
}

/**
 * Saves the selected changes into the working tree. Every file is checked first; if any check
 * fails nothing is written. With `backup`, originals are kept outside the project for Undo.
 */
export async function applyChanges(rootInput: string, changes: ApplyChange[], backup: boolean): Promise<ApplyResult> {
  const root = normalizeInputPath(rootInput)
  if (!changes.length) throw new PathError(400, 'No changes to apply.')

  const plan = await Promise.all(
    changes.map(async (c) => {
      const abs = await resolveInsideReal(root, c.path)
      const raw = await readRaw(abs)
      return { ...c, abs, raw, current: raw === null ? null : decode(raw) }
    }),
  )

  const problems: string[] = []
  for (const p of plan) {
    if (p.status === 'A' && p.current) problems.push(`${p.path} already exists`)
    if (p.status !== 'A' && !p.current) problems.push(`${p.path} no longer exists`)
    if (p.status !== 'A' && p.current && p.expected !== undefined && p.current.text !== p.expected.replace(/\r\n/g, '\n'))
      problems.push(`${p.path} changed on disk since it was sent`)
    if (p.status !== 'D' && typeof p.content !== 'string') problems.push(`${p.path} has no content`)
  }
  if (problems.length) throw new PathError(409, `Nothing was saved. ${problems.join('; ')}.`)

  const manifest: Manifest = { created: [], replaced: [], createdDirs: [], written: {} }
  let backupId: string | null = null
  let snapshot: string | null = null
  if (backup) {
    backupId = new Date().toISOString().replace(/[:.]/g, '-')
    snapshot = path.join(snapshotsDir(root), backupId)
    await mkdir(path.join(snapshot, 'files'), { recursive: true })
    for (const p of plan) {
      if (!p.current) continue
      const dest = path.join(snapshot, 'files', p.path)
      await mkdir(path.dirname(dest), { recursive: true })
      await copyFile(p.abs, dest)
      manifest.replaced.push(p.path)
    }
  }

  const result: ApplyResult = { created: 0, overwritten: 0, deleted: 0, backupId }
  for (const p of plan) {
    if (p.status === 'D') {
      await rm(p.abs)
      manifest.written[p.path] = null
      result.deleted++
      continue
    }
    const firstCreated = await mkdir(path.dirname(p.abs), { recursive: true })
    if (firstCreated) {
      // Record every folder from the first one created down to the file's folder.
      for (let d = path.dirname(p.abs); d.length >= firstCreated.length; d = path.dirname(d))
        manifest.createdDirs.push(path.relative(root, d).split(path.sep).join('/'))
    }
    const out = encode(p.content!, p.current?.format ?? NEW_FILE_FORMAT)
    await writeFile(p.abs, out)
    manifest.written[p.path] = sha(out)
    if (p.status === 'A') {
      manifest.created.push(p.path)
      result.created++
    } else result.overwritten++
  }

  if (snapshot) {
    await writeFile(path.join(snapshot, 'manifest.json'), JSON.stringify(manifest, null, 2))
    await pruneSnapshots(path.dirname(snapshot))
  }
  return result
}

/**
 * Restores a snapshot: puts back overwritten and deleted files, removes created files and
 * folders. Refused if any of those files was edited after the save, so later work isn't lost.
 */
export async function undoApply(rootInput: string, backupId: string): Promise<{ restored: number; removed: number }> {
  const root = normalizeInputPath(rootInput)
  if (!/^[\w-]+$/.test(backupId)) throw new PathError(400, 'Invalid undo id.')
  const snapshot = path.join(snapshotsDir(root), backupId)
  let manifest: Manifest
  try {
    manifest = JSON.parse(await readFile(path.join(snapshot, 'manifest.json'), 'utf8')) as Manifest
  } catch {
    throw new PathError(404, 'Nothing to undo: the snapshot is gone.')
  }

  const edited: string[] = []
  for (const [rel, written] of Object.entries(manifest.written)) {
    const now = await readRaw(resolveInside(root, rel))
    if ((now === null ? null : sha(now)) !== written) edited.push(rel)
  }
  if (edited.length) throw new PathError(409, `Not undone: edited since the save: ${edited.join(', ')}.`)

  let restored = 0
  let removed = 0
  for (const rel of manifest.replaced) {
    const abs = resolveInside(root, rel)
    await mkdir(path.dirname(abs), { recursive: true })
    await copyFile(path.join(snapshot, 'files', rel), abs)
    restored++
  }
  for (const rel of manifest.created) {
    await rm(resolveInside(root, rel), { force: true })
    removed++
  }
  // Deepest folders first, so parents are empty by the time they're tried.
  const dirs = [...new Set(manifest.createdDirs)].sort((a, b) => b.split('/').length - a.split('/').length)
  for (const rel of dirs) {
    try {
      await rmdir(resolveInside(root, rel))
    } catch {
      // Not empty (something else lives there now): leave it.
    }
  }
  await rm(snapshot, { recursive: true, force: true })
  return { restored, removed }
}
