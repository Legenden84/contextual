// Applies the model's edit blocks to the local copy of a file.
//
// To save output tokens, the model returns only the changed region of an existing file: a few
// unchanged lines that locate it (SEARCH) and what they become (REPLACE). Contextual rebuilds the
// full file here, so everything downstream (Changes panel, diffs, Save) works on whole files.

export interface EditBlock {
  search: string
  replace: string
}

export interface EditOutcome {
  /** The file with every block that could be placed applied. */
  content: string
  /** Blocks whose SEARCH text wasn't found. */
  failed: EditBlock[]
}

const toLines = (s: string) => {
  const lines = s.replace(/\r\n/g, '\n').split('\n')
  // A block ends with a newline before the marker; drop that empty tail.
  if (lines.length > 1 && lines[lines.length - 1] === '') lines.pop()
  return lines
}

const indentOf = (line: string) => /^\s*/.exec(line)![0]

type Compare = (a: string, b: string) => boolean
const EXACT: Compare = (a, b) => a === b
const TRAILING: Compare = (a, b) => a.trimEnd() === b.trimEnd()
const INDENT: Compare = (a, b) => a.trim() === b.trim()

/** Start indexes where `needle` occurs in `hay` under `eq`. */
function findAll(hay: string[], needle: string[], eq: Compare): number[] {
  const hits: number[] = []
  outer: for (let i = 0; i + needle.length <= hay.length; i++) {
    for (let j = 0; j < needle.length; j++) if (!eq(hay[i + j], needle[j])) continue outer
    hits.push(i)
  }
  return hits
}

/** Shifts replacement lines by the indentation difference between the file and the SEARCH text. */
function reindent(replace: string[], fileLine: string, searchLine: string): string[] {
  const have = indentOf(fileLine)
  const wrote = indentOf(searchLine)
  if (have === wrote) return replace
  return replace.map((l) => {
    if (!l.trim()) return l
    if (have.startsWith(wrote)) return have.slice(wrote.length) + l
    if (wrote.startsWith(have) && l.startsWith(wrote.slice(have.length))) return l.slice(wrote.length - have.length)
    return l
  })
}

const trimBlankEdges = (lines: string[]) => {
  let a = 0
  let b = lines.length
  while (a < b && !lines[a].trim()) a++
  while (b > a && !lines[b - 1].trim()) b--
  return lines.slice(a, b)
}

/**
 * Applies blocks in order. Each SEARCH is matched exactly, then ignoring trailing whitespace,
 * then ignoring indentation (re-indenting the replacement), then with blank edge lines dropped.
 * When a SEARCH occurs more than once, the first occurrence after the previous edit wins.
 */
export function applyEdits(original: string, edits: EditBlock[]): EditOutcome {
  let lines = toLines(original)
  const failed: EditBlock[] = []
  let cursor = 0

  for (const edit of edits) {
    const search = toLines(edit.search)
    let replace = toLines(edit.replace)
    if (edit.replace === '') replace = []

    // An empty SEARCH only makes sense for an empty file: the replacement is the whole file.
    if (!search.join('').trim()) {
      if (!lines.join('').trim()) {
        lines = replace
        cursor = lines.length
      } else failed.push(edit)
      continue
    }

    let placed = false
    for (const [needle, repl] of [
      [search, replace],
      [trimBlankEdges(search), trimBlankEdges(replace)],
    ] as const) {
      if (!needle.length) continue
      for (const eq of [EXACT, TRAILING, INDENT]) {
        const hits = findAll(lines, needle, eq)
        if (!hits.length) continue
        const at = hits.find((h) => h >= cursor) ?? hits[0]
        const firstReal = needle.findIndex((l) => l.trim())
        const fixed = eq === INDENT && firstReal >= 0 ? reindent([...repl], lines[at + firstReal], needle[firstReal]) : [...repl]
        lines = [...lines.slice(0, at), ...fixed, ...lines.slice(at + needle.length)]
        cursor = at + fixed.length
        placed = true
        break
      }
      if (placed) break
    }
    if (!placed) failed.push(edit)
  }

  const endsWithNewline = /\n$/.test(original)
  return { content: lines.join('\n') + (endsWithNewline && lines.length ? '\n' : ''), failed }
}

/** The blocks as the model wrote them, for asking it to redo a file in full. */
export function formatEdits(edits: EditBlock[]): string {
  return edits.map((e) => `<<<<<<< SEARCH\n${e.search}\n=======\n${e.replace}\n>>>>>>> REPLACE`).join('\n\n')
}
