// Minimal markdown parser for assistant responses: paragraphs, bullet lists, inline code,
// bold, fenced code blocks, and the `### FILE:` / `### DELETE:` headers the output-format
// prompt asks for. Anything else renders as a paragraph.

import { langFromFence, langFromPath, type Lang } from './syntax'

export type Inline = { kind: 'text'; text: string } | { kind: 'code'; text: string } | { kind: 'bold'; text: string }

export type Block =
  | { kind: 'p'; inline: Inline[] }
  | { kind: 'ul'; items: { status?: 'A' | 'M' | 'D'; inline: Inline[] }[] }
  | { kind: 'file'; path: string }
  | { kind: 'delete'; path: string }
  | { kind: 'code'; code: string; lang: Lang; path?: string }

export function parseInline(text: string): Inline[] {
  const out: Inline[] = []
  const re = /`([^`]+)`|\*\*([^*]+)\*\*/g
  let last = 0
  for (let m = re.exec(text); m; m = re.exec(text)) {
    if (m.index > last) out.push({ kind: 'text', text: text.slice(last, m.index) })
    out.push(m[1] != null ? { kind: 'code', text: m[1] } : { kind: 'bold', text: m[2] })
    last = m.index + m[0].length
  }
  if (last < text.length) out.push({ kind: 'text', text: text.slice(last) })
  return out
}

export function parseMarkdown(src: string): Block[] {
  const lines = src.split('\n')
  const blocks: Block[] = []
  let para: string[] = []
  let filePath: string | undefined

  const flushPara = () => {
    if (para.length) blocks.push({ kind: 'p', inline: parseInline(para.join(' ')) })
    para = []
  }

  for (let i = 0; i < lines.length; i++) {
    const line = lines[i]
    const fence = /^```(.*)$/.exec(line)
    const file = /^### FILE:\s*(.+)$/.exec(line)
    const del = /^### DELETE:\s*(.+)$/.exec(line)

    if (fence) {
      flushPara()
      const code: string[] = []
      while (++i < lines.length && !/^```\s*$/.test(lines[i])) code.push(lines[i])
      const lang = fence[1].trim() ? langFromFence(fence[1]) : langFromPath(filePath)
      blocks.push({ kind: 'code', code: code.join('\n'), lang, path: filePath })
      filePath = undefined
    } else if (file) {
      flushPara()
      filePath = file[1].trim()
      blocks.push({ kind: 'file', path: filePath })
    } else if (del) {
      flushPara()
      blocks.push({ kind: 'delete', path: del[1].trim() })
    } else if (/^\s*[-*] /.test(line)) {
      flushPara()
      const items: { status?: 'A' | 'M' | 'D'; inline: Inline[] }[] = []
      for (; i < lines.length && /^\s*[-*] /.test(lines[i]); i++) {
        const body = lines[i].replace(/^\s*[-*] /, '')
        const st = /^\[([AMD])\]\s*/.exec(body)
        items.push({ status: st?.[1] as 'A' | 'M' | 'D' | undefined, inline: parseInline(st ? body.slice(st[0].length) : body) })
      }
      i--
      blocks.push({ kind: 'ul', items })
    } else if (!line.trim()) {
      flushPara()
    } else {
      para.push(line.trim())
    }
  }
  flushPara()
  return blocks
}
