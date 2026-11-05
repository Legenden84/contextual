// Turns an assistant response into proposed file changes, from the `### FILE:` and
// `### DELETE:` headers the output-format prompt asks for.

import type { Change } from '@/data/types'
import { parseMarkdown } from './markdown'

export interface ChangeContext {
  /** Does the path exist in the project? */
  exists: (path: string) => boolean
  /** Contents as they were sent to the model, when known. */
  original: (path: string) => string | undefined
  /** Has the file changed on disk since it was loaded into context? */
  changedOnDisk: (path: string) => boolean
  /** Sent only as a condensed reference: never offered for apply. */
  readOnly?: (path: string) => boolean
}

const CONFLICT = 'Changed on disk after it was sent. The proposal was made against the older version.'

export function parseChanges(response: string, ctx: ChangeContext): Change[] {
  const byPath = new Map<string, Change>()
  for (const b of parseMarkdown(response)) {
    if ((b.kind === 'code' || b.kind === 'delete') && b.path && ctx.readOnly?.(b.path)) continue
    if (b.kind === 'code' && b.path) {
      const old = ctx.original(b.path) ?? ''
      byPath.set(b.path, {
        path: b.path,
        status: ctx.exists(b.path) ? 'M' : 'A',
        old,
        new: b.code,
        conflict: ctx.changedOnDisk(b.path) ? CONFLICT : undefined,
      })
    } else if (b.kind === 'delete') {
      byPath.set(b.path, {
        path: b.path,
        status: 'D',
        old: ctx.original(b.path) ?? '',
        new: '',
        conflict: ctx.changedOnDisk(b.path) ? CONFLICT : undefined,
      })
    }
  }
  // A "modified" file whose new content equals the old one isn't a change.
  return [...byPath.values()].filter((c) => !(c.status === 'M' && c.old === c.new))
}
