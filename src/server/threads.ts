// Saved conversations per project, outside the project folder:
// ~/.cache/contextual/threads/<project hash>/<thread id>.json plus a small index.json.
// The thread body is opaque to the server (the client owns its shape).

import { createHash } from 'node:crypto'
import { mkdir, readFile, rename, rm, writeFile } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { normalizeInputPath, PathError } from './fs/paths.ts'

export interface ThreadSummary {
  id: string
  title: string
  updatedAt: string
  turns: number
}

/** Threads kept per project; older ones are deleted. */
const MAX_THREADS = 30

function threadsDir(rootInput: string): string {
  const root = normalizeInputPath(rootInput)
  const cache = process.env.XDG_CACHE_HOME || path.join(os.homedir(), '.cache')
  return path.join(cache, 'contextual', 'threads', createHash('sha1').update(root).digest('hex').slice(0, 16))
}

function checkId(id: string) {
  if (!/^[\w-]{1,64}$/.test(id)) throw new PathError(400, 'Invalid thread id.')
}

async function readIndex(dir: string): Promise<ThreadSummary[]> {
  try {
    const list = JSON.parse(await readFile(path.join(dir, 'index.json'), 'utf8')) as unknown
    return Array.isArray(list) ? (list as ThreadSummary[]) : []
  } catch {
    return []
  }
}

/** Writes via a temp file so a crash never leaves half a JSON file. */
async function writeAtomic(file: string, data: string) {
  const tmp = `${file}.${process.pid}.tmp`
  await writeFile(tmp, data)
  await rename(tmp, file)
}

export async function listThreads(root: string): Promise<ThreadSummary[]> {
  return (await readIndex(threadsDir(root))).sort((a, b) => b.updatedAt.localeCompare(a.updatedAt))
}

export async function loadThread(root: string, id: string): Promise<unknown> {
  checkId(id)
  try {
    return JSON.parse(await readFile(path.join(threadsDir(root), `${id}.json`), 'utf8')) as unknown
  } catch {
    throw new PathError(404, 'Conversation not found.')
  }
}

export async function saveThread(root: string, id: string, summary: Omit<ThreadSummary, 'id'>, thread: unknown) {
  checkId(id)
  const dir = threadsDir(root)
  await mkdir(dir, { recursive: true })
  await writeAtomic(path.join(dir, `${id}.json`), JSON.stringify(thread))

  const index = [{ id, ...summary }, ...(await readIndex(dir)).filter((t) => t.id !== id)].sort((a, b) =>
    b.updatedAt.localeCompare(a.updatedAt),
  )
  for (const old of index.slice(MAX_THREADS)) await rm(path.join(dir, `${old.id}.json`), { force: true })
  await writeAtomic(path.join(dir, 'index.json'), JSON.stringify(index.slice(0, MAX_THREADS)))
}

export async function deleteThread(root: string, id: string) {
  checkId(id)
  const dir = threadsDir(root)
  await rm(path.join(dir, `${id}.json`), { force: true })
  await writeAtomic(path.join(dir, 'index.json'), JSON.stringify((await readIndex(dir)).filter((t) => t.id !== id)))
}

/** Renames a conversation: the index entry and the saved conversation (its `title` field). */
export async function renameThread(root: string, id: string, title: string) {
  checkId(id)
  const clean = title.trim().slice(0, 200)
  if (!clean) throw new PathError(400, 'The name cannot be empty.')
  const dir = threadsDir(root)
  const file = path.join(dir, `${id}.json`)
  let thread: Record<string, unknown>
  try {
    thread = JSON.parse(await readFile(file, 'utf8')) as Record<string, unknown>
  } catch {
    throw new PathError(404, 'Conversation not found.')
  }
  await writeAtomic(file, JSON.stringify({ ...thread, title: clean }))
  await writeAtomic(path.join(dir, 'index.json'), JSON.stringify((await readIndex(dir)).map((t) => (t.id === id ? { ...t, title: clean } : t))))
}

/** Deletes every saved conversation of a project. */
export async function deleteAllThreads(root: string): Promise<number> {
  const dir = threadsDir(root)
  await mkdir(dir, { recursive: true })
  const index = await readIndex(dir)
  for (const t of index) await rm(path.join(dir, `${t.id}.json`), { force: true })
  await writeAtomic(path.join(dir, 'index.json'), '[]')
  return index.length
}
