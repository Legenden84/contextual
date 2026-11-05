import { useMemo } from 'react'
import { langFromPath, type Lang } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import { Highlight } from './Highlight'

/** Indent level per line (4 columns); blank lines take the smaller of their neighbours. */
function indentLevels(lines: string[]): number[] {
  const lv = lines.map((l) => (l.trim() ? Math.floor(l.match(/^ */)![0].length / 4) : -1))
  for (let i = 0; i < lv.length; i++) {
    if (lv[i] >= 0) continue
    let j = i
    while (j < lv.length && lv[j] < 0) j++
    const prev = i > 0 ? lv[i - 1] : 0
    const next = j < lv.length ? lv[j] : 0
    for (let k = i; k < j; k++) lv[k] = Math.min(prev, next)
  }
  return lv
}

export interface CodeViewProps {
  code: string
  path?: string
  lang?: Lang
  /** 1-based line drawn with the current-line band. */
  currentLine?: number
  /** Indent guide (0-based) emphasised on the current line. */
  activeGuide?: number
  /** First line number. */
  start?: number
  wrap?: boolean
  guides?: boolean
  onLineClick?: (line: number) => void
  /** Smaller type and gutter, for previews in side panels. */
  compact?: boolean
  className?: string
}

/** Read-only editor view used for every code surface. */
export function CodeView({
  code,
  path,
  lang,
  currentLine,
  activeGuide,
  start = 1,
  wrap,
  guides = true,
  onLineClick,
  compact,
  className,
}: CodeViewProps) {
  const language = lang ?? langFromPath(path)
  const lines = useMemo(() => code.replace(/\t/g, '    ').split('\n'), [code])
  const levels = useMemo(() => indentLevels(lines), [lines])

  return (
    <div
      className={cn(
        'overflow-auto bg-surface-100 py-1 font-mono text-code text-syn-plain [font-variant-ligatures:none] selection:bg-editor-selection',
        compact && 'text-[11px] leading-[17px]',
        className,
      )}
    >
      {lines.map((l, i) => {
        const n = start + i
        const current = n === currentLine
        const lead = l.match(/^ */)![0].length
        const g = guides ? (l.trim() ? Math.min(levels[i], Math.floor(lead / 4)) : levels[i]) : 0
        const rest = l.trim() ? l.slice(g * 4) : ''
        return (
          <div
            key={i}
            onClick={onLineClick && (() => onLineClick(n))}
            className={cn(
              'relative flex',
              compact ? 'min-h-[17px]' : 'min-h-5',
              wrap ? 'break-all whitespace-pre-wrap' : 'whitespace-pre',
              current &&
                'bg-editor-line-current shadow-[inset_0_1px_0_var(--border),inset_0_-1px_0_var(--border)]',
            )}
          >
            <span
              className={cn(
                'flex-none text-right text-editor-gutter-ink select-none',
                compact ? 'w-7 pr-2 text-[10px]' : 'w-12 pr-4 text-[12px]',
                current && 'text-editor-gutter-active',
              )}
            >
              {n}
            </span>
            <span className="flex-1 pr-4">
              {Array.from({ length: g }, (_, k) => (
                <span
                  key={k}
                  className={cn(
                    'inline-block w-[4ch]',
                    k > 0 && 'shadow-[inset_1px_0_0_var(--editor-indent-guide)]',
                    k > 0 && current && activeGuide === k && 'shadow-[inset_1px_0_0_var(--ink-faint)]',
                  )}
                >
                  {'    '}
                </span>
              ))}
              <Highlight line={rest} lang={language} />
              {!rest && !g && ' '}
            </span>
          </div>
        )
      })}
    </div>
  )
}
