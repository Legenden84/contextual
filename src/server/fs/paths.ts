// Path handling for the local file system. The server runs inside WSL (or Linux/macOS),
// so Windows-style input is translated: C:\src\x → /mnt/c/src/x, \\wsl$\Ubuntu\home → /home.

import { realpath } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

export class PathError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

/** Turns what the user typed into an absolute path on this machine. */
export function normalizeInputPath(input: string | null | undefined): string {
  let p = (input ?? '').trim().replace(/^["']|["']$/g, '')
  if (!p || p === '~') return os.homedir()
  if (p.startsWith('~/')) return path.resolve(os.homedir(), p.slice(2))

  if (process.platform !== 'win32') {
    const wsl = /^\\\\wsl(?:\$|\.localhost)\\[^\\]+(?:\\(.*))?$/i.exec(p)
    if (wsl) p = '/' + (wsl[1] ?? '').replace(/\\/g, '/')
    const drive = /^([A-Za-z]):(?:[\\/](.*))?$/.exec(p)
    if (drive) p = `/mnt/${drive[1].toLowerCase()}/${(drive[2] ?? '').replace(/\\/g, '/')}`
  }
  return path.resolve(p)
}

/** Validates a project-relative path ("src/a.cs") and returns its absolute path inside `root`. */
export function resolveInside(root: string, rel: string): string {
  if (typeof rel !== 'string' || !rel || path.isAbsolute(rel) || rel.includes('\0'))
    throw new PathError(400, `Invalid project path: ${String(rel)}`)
  const abs = path.resolve(root, rel)
  if (abs !== root && !abs.startsWith(root + path.sep)) throw new PathError(400, `Path escapes the project: ${rel}`)
  return abs
}

/** Like resolveInside, but also follows symlinks of the deepest existing ancestor to keep writes inside. */
export async function resolveInsideReal(root: string, rel: string): Promise<string> {
  const abs = resolveInside(root, rel)
  const realRoot = await realpath(root)
  let probe = abs
  for (;;) {
    try {
      const real = await realpath(probe)
      if (real !== realRoot && !real.startsWith(realRoot + path.sep))
        throw new PathError(400, `Path leaves the project through a symlink: ${rel}`)
      return abs
    } catch (err) {
      if (err instanceof PathError) throw err
      const parent = path.dirname(probe)
      if (parent === probe) return abs
      probe = parent
    }
  }
}

/** Project-relative path with forward slashes. */
export function toRel(root: string, abs: string): string {
  return path.relative(root, abs).split(path.sep).join('/')
}
