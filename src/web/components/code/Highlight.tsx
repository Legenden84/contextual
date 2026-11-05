import { memo, type ReactNode } from 'react'
import { tokenize, type Lang, type TokenClass } from '@/lib/syntax'
import { cn } from '@/lib/utils'

const TOKEN_CLASS: Record<TokenClass, string> = {
  plain: 'text-syn-plain',
  pun: 'text-syn-plain',
  kw: 'text-syn-keyword',
  ctl: 'text-syn-control',
  type: 'text-syn-type',
  fn: 'text-syn-function',
  str: 'text-syn-string',
  num: 'text-syn-number',
  com: 'text-syn-comment italic',
  var: 'text-syn-variable',
  tag: 'text-syn-tag',
  head: 'text-syn-heading font-semibold',
}

export interface HighlightProps {
  line: string
  lang: Lang
  /** [start, end) of characters to mark as changed (word-level diff). */
  range?: [number, number]
  /** Background class for the changed characters. */
  markClass?: string
}

/** One syntax-highlighted line of code. */
export const Highlight = memo(function Highlight({ line, lang, range, markClass }: HighlightProps) {
  const out: ReactNode[] = []
  let at = 0
  tokenize(line, lang).forEach((tk, i) => {
    const s = at
    const e = at + tk.t.length
    const cls = TOKEN_CLASS[tk.c]
    if (!range || e <= range[0] || s >= range[1]) {
      out.push(
        <span key={i} className={cls}>
          {tk.t}
        </span>,
      )
    } else {
      const a = Math.max(s, range[0]) - s
      const b = Math.min(e, range[1]) - s
      if (a > 0) out.push(<span key={`${i}a`} className={cls}>{tk.t.slice(0, a)}</span>)
      out.push(
        <span key={`${i}m`} className={cn(cls, 'rounded-[2px]', markClass)}>
          {tk.t.slice(a, b)}
        </span>,
      )
      if (b < tk.t.length) out.push(<span key={`${i}b`} className={cls}>{tk.t.slice(b)}</span>)
    }
    at = e
  })
  return <>{out}</>
})
