// Markdown parser for assistant responses. Covers what models actually write: headings,
// paragraphs, bullet and numbered lists (nested by indentation), block quotes, tables,
// horizontal rules, fenced code blocks, and inline code, bold, italic and links. Plus the
// `### FILE:` / `### DELETE:` headers the output-format prompt asks for, which drive the
// Changes panel. Anything unrecognised renders as a paragraph.

import type { EditBlock } from './edits'
import { langFromFence, langFromPath, type Lang } from './syntax'

export type Inline =
  | { kind: 'text'; text: string }
  | { kind: 'code'; text: string }
  | { kind: 'bold'; children: Inline[] }
  | { kind: 'italic'; children: Inline[] }
  | { kind: 'strike'; children: Inline[] }
  | { kind: 'link'; href: string; children: Inline[] }

export interface ListItem {
  /** Nesting level, from the item's indentation. */
  depth: number
  /** "1." for numbered items; undefined for bullets. */
  number?: string
  /** `[A]` / `[M]` / `[D]` at the start of an item, shown as a status letter. */
  status?: 'A' | 'M' | 'D'
  /** `[ ]` / `[x]` task-list checkbox. */
  checked?: boolean
  inline: Inline[]
}

export type Align = 'left' | 'center' | 'right' | null

export type Block =
  | { kind: 'p'; inline: Inline[] }
  | { kind: 'heading'; level: 1 | 2 | 3 | 4 | 5 | 6; inline: Inline[] }
  | { kind: 'list'; items: ListItem[] }
  | { kind: 'quote'; blocks: Block[] }
  | { kind: 'table'; head: Inline[][]; align: Align[]; rows: Inline[][][] }
  | { kind: 'hr' }
  | { kind: 'file'; path: string }
  /** Changed regions of an existing file (`### EDIT:` + SEARCH/REPLACE blocks); `partial` while still streaming. */
  | { kind: 'edit'; path: string; edits: EditBlock[]; partial: boolean }
  | { kind: 'delete'; path: string }
  | { kind: 'code'; code: string; lang: Lang; path?: string }

// ------------------------------------------------------------------ inline

/** Earliest inline construct in `text`, as [index, length, node]. */
type Hit = [number, number, Inline]

const INLINE_RULES: { re: RegExp; make: (m: RegExpExecArray) => Inline }[] = [
  { re: /`([^`\n]+)`/, make: (m) => ({ kind: 'code', text: m[1] }) },
  { re: /\[([^\]\n]+)\]\(([^)\s]+)(?:\s+"[^"]*")?\)/, make: (m) => ({ kind: 'link', href: m[2], children: parseInline(m[1]) }) },
  { re: /\*\*(?=\S)([^\n]+?)\*\*|__(?=\S)([^\n]+?)__/, make: (m) => ({ kind: 'bold', children: parseInline(m[1] ?? m[2]) }) },
  { re: /~~(?=\S)([^\n]+?)~~/, make: (m) => ({ kind: 'strike', children: parseInline(m[1]) }) },
  // Single * or _ italics, but not inside words (snake_case, 2*3*4) or list markers.
  {
    re: /(?<![\w*])\*(?=[^\s*])([^*\n]+?)\*(?![\w*])|(?<![\w_])_(?=[^\s_])([^_\n]+?)_(?![\w_])/,
    make: (m) => ({ kind: 'italic', children: parseInline(m[1] ?? m[2]) }),
  },
  // Bare URLs.
  { re: /\bhttps?:\/\/[^\s<>()]+[^\s<>().,;:!?'"]/, make: (m) => ({ kind: 'link', href: m[0], children: [{ kind: 'text', text: m[0] }] }) },
]

export function parseInline(text: string): Inline[] {
  const out: Inline[] = []
  let rest = text
  while (rest) {
    let best: Hit | null = null
    for (const rule of INLINE_RULES) {
      const m = rule.re.exec(rest)
      if (m && (!best || m.index < best[0])) best = [m.index, m[0].length, rule.make(m)]
    }
    if (!best) {
      out.push({ kind: 'text', text: rest })
      break
    }
    if (best[0] > 0) out.push({ kind: 'text', text: rest.slice(0, best[0]) })
    out.push(best[2])
    rest = rest.slice(best[0] + best[1])
  }
  return out
}

/** Plain text of inline nodes (for titles and tests). */
export function inlineText(nodes: Inline[]): string {
  return nodes.map((n) => (n.kind === 'text' || n.kind === 'code' ? n.text : inlineText(n.children))).join('')
}

// ------------------------------------------------------------------ blocks

const FENCE = /^\s*(`{3,}|~{3,})(.*)$/
const SEARCH_MARK = /^\s*<{5,9}\s*SEARCH\s*$/
const DIVIDER_MARK = /^\s*={5,9}\s*$/
const REPLACE_MARK = /^\s*>{5,9}\s*REPLACE\s*$/
/** A header path without backticks or a trailing note such as "(condensed reference, …)". */
const cleanPath = (raw: string) => raw.trim().replace(/\s+\(.*\)$/, '').replace(/^`|`$/g, '')
const HEADING = /^\s{0,3}(#{1,6})\s+(.*?)\s*#*\s*$/
const HR = /^\s{0,3}([-*_])(\s*\1){2,}\s*$/
const LIST_ITEM = /^(\s*)([-*+]|\d{1,9}[.)])\s+(.*)$/
const QUOTE = /^\s{0,3}>\s?(.*)$/
const TABLE_DIVIDER = /^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$/

const splitRow = (line: string) =>
  line
    .trim()
    .replace(/^\||\|$/g, '')
    .split(/(?<!\\)\|/)
    .map((c) => c.trim().replace(/\\\|/g, '|'))

function listItem(m: RegExpExecArray): ListItem {
  const indent = m[1].replace(/\t/g, '    ').length
  const marker = m[2]
  let body = m[3]
  const item: ListItem = { depth: Math.floor(indent / 2), inline: [] }
  if (/^\d/.test(marker)) item.number = marker.replace(')', '.')
  const st = /^\[([AMD])\]\s*/.exec(body)
  if (st) {
    item.status = st[1] as ListItem['status']
    body = body.slice(st[0].length)
  }
  const task = /^\[( |x|X)\]\s+/.exec(body)
  if (task) {
    item.checked = task[1] !== ' '
    body = body.slice(task[0].length)
  }
  item.inline = parseInline(body)
  return item
}

export function parseMarkdown(src: string): Block[] {
  const lines = src.replace(/\r\n/g, '\n').split('\n')
  const blocks: Block[] = []
  let para: string[] = []
  let filePath: string | undefined

  const flushPara = () => {
    if (para.length) blocks.push({ kind: 'p', inline: parseInline(para.join(' ')) })
    para = []
  }

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    const fence = FENCE.exec(line)
    const file = /^\s*#{2,4} FILE:\s*(.+)$/.exec(line)
    const del = /^\s*#{2,4} DELETE:\s*(.+)$/.exec(line)
    const edit = /^\s*#{2,4} EDIT:\s*(.+)$/.exec(line)

    if (fence) {
      flushPara()
      const marker = fence[1]
      const code: string[] = []
      const close = new RegExp(`^\\s*${marker[0] === '`' ? '`' : '~'}{${marker.length},}\\s*$`)
      while (++i < lines.length && !close.test(lines[i])) code.push(lines[i])
      const info = fence[2].trim().split(/\s+/)[0] ?? ''
      const lang = info ? langFromFence(info) : langFromPath(filePath)
      blocks.push({ kind: 'code', code: code.join('\n'), lang, path: filePath })
      filePath = undefined
    } else if (edit) {
      flushPara()
      const path = cleanPath(edit[1])
      const edits: EditBlock[] = []
      let partial = false
      let state: 'idle' | 'search' | 'replace' = 'idle'
      let search: string[] = []
      let replace: string[] = []
      let asFile = false
      for (i++; i < lines.length; i++) {
        const l = lines[i]
        if (state === 'idle') {
          if (SEARCH_MARK.test(l)) {
            state = 'search'
            search = []
            replace = []
          } else if (!l.trim()) continue
          else if (FENCE.test(l)) {
            // A fence that doesn't wrap SEARCH/REPLACE blocks is a whole file under an EDIT header.
            let k = i + 1
            while (k < lines.length && !lines[k].trim()) k++
            if (!edits.length && !SEARCH_MARK.test(lines[k] ?? '') && k < lines.length) {
              asFile = true
              break
            }
          } else break // Anything else ends the edit section.
        } else if (state === 'search') {
          if (DIVIDER_MARK.test(l)) state = 'replace'
          else search.push(l)
        } else if (REPLACE_MARK.test(l)) {
          edits.push({ search: search.join('\n'), replace: replace.join('\n') })
          state = 'idle'
        } else replace.push(l)
      }
      if (state !== 'idle') partial = true
      i--
      if (asFile) {
        filePath = path
        blocks.push({ kind: 'file', path })
      } else blocks.push({ kind: 'edit', path, edits, partial })
    } else if (file) {
      flushPara()
      // Strip a trailing note such as "(condensed reference, read-only…)".
      filePath = file[1].trim().replace(/\s+\(.*\)$/, '').replace(/^`|`$/g, '')
      blocks.push({ kind: 'file', path: filePath })
    } else if (del) {
      flushPara()
      blocks.push({ kind: 'delete', path: del[1].trim().replace(/\s+\(.*\)$/, '').replace(/^`|`$/g, '') })
    } else if (HEADING.test(line)) {
      flushPara()
      const m = HEADING.exec(line)!
      blocks.push({ kind: 'heading', level: m[1].length as 1 | 2 | 3 | 4 | 5 | 6, inline: parseInline(m[2]) })
    } else if (HR.test(line)) {
      flushPara()
      blocks.push({ kind: 'hr' })
    } else if (LIST_ITEM.test(line)) {
      flushPara()
      const items: ListItem[] = []
      for (; i < lines.length; i++) {
        const m = LIST_ITEM.exec(lines[i])
        if (m) {
          items.push(listItem(m))
          continue
        }
        // An indented line continues the previous item; a blank line followed by another item keeps the list going.
        if (/^\s{2,}\S/.test(lines[i]) && !FENCE.test(lines[i]) && items.length) {
          const last = items[items.length - 1]
          last.inline = [...last.inline, { kind: 'text', text: ' ' }, ...parseInline(lines[i].trim())]
          continue
        }
        if (!lines[i].trim() && LIST_ITEM.test(lines[i + 1] ?? '')) continue
        break
      }
      i--
      blocks.push({ kind: 'list', items })
    } else if (QUOTE.test(line)) {
      flushPara()
      const inner: string[] = []
      for (; i < lines.length && QUOTE.test(lines[i]); i++) inner.push(QUOTE.exec(lines[i])![1])
      i--
      blocks.push({ kind: 'quote', blocks: parseMarkdown(inner.join('\n')) })
    } else if (line.includes('|') && TABLE_DIVIDER.test(lines[i + 1] ?? '') && !para.length) {
      const head = splitRow(line).map(parseInline)
      const align: Align[] = splitRow(lines[i + 1]).map((c) =>
        c.startsWith(':') && c.endsWith(':') ? 'center' : c.endsWith(':') ? 'right' : c.startsWith(':') ? 'left' : null,
      )
      const rows: Inline[][][] = []
      for (i += 2; i < lines.length && lines[i].includes('|') && lines[i].trim(); i++) rows.push(splitRow(lines[i]).map(parseInline))
      i--
      blocks.push({ kind: 'table', head, align, rows })
    } else if (!line.trim()) {
      flushPara()
    } else {
      para.push(line.trim())
    }
  }
  flushPara()
  return blocks
}
