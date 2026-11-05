import { useMemo, useState, type ReactNode } from 'react'
import { Highlight } from '@/components/code/Highlight'
import { Icon } from '@/components/ui/Icon'
import { hunkHeader, makeHunks, type Diff, type DiffOp } from '@/lib/diff'
import type { Lang } from '@/lib/syntax'
import { cn } from '@/lib/utils'

const NO = 'px-2 text-right align-top text-[12px] text-editor-gutter-ink select-none'
const CODE = 'relative pr-4 pl-[22px] text-syn-plain'
const SIGN = 'absolute top-0 left-2 select-none'
const ADD_WORD = 'bg-diff-add-word'
const DEL_WORD = 'bg-diff-del-word'

function Sign({ k }: { k: DiffOp['k'] }) {
  if (k === ' ') return null
  return <span className={cn(SIGN, k === '+' ? 'text-success' : 'text-danger')}>{k === '+' ? '+' : '−'}</span>
}

export interface DiffTableProps {
  diff: Diff
  lang: Lang
  mode: 'unified' | 'split'
  /** Expand every collapsed region. */
  expandAll?: boolean
}

/** GitHub-style diff body: unified or split, with collapsed unchanged regions. */
export function DiffTable({ diff, lang, mode, expandAll }: DiffTableProps) {
  const items = useMemo(() => makeHunks(diff.ops, 3), [diff])
  const [expanded, setExpanded] = useState<ReadonlySet<number>>(new Set())
  const split = mode === 'split'
  const noCols = split ? 1 : 2
  const codeCols = split ? 3 : 1

  const contextRow = (o: DiffOp, key: string) =>
    split ? (
      <tr key={key}>
        <td className={NO}>{o.a}</td>
        <td className={cn(CODE, 'wrap-anywhere whitespace-pre-wrap')}>
          <Highlight line={o.t} lang={lang} />
        </td>
        <td className={cn(NO, 'border-l border-border')}>{o.b}</td>
        <td className={cn(CODE, 'wrap-anywhere whitespace-pre-wrap')}>
          <Highlight line={o.t} lang={lang} />
        </td>
      </tr>
    ) : (
      <tr key={key}>
        <td className={NO}>{o.a}</td>
        <td className={NO}>{o.b}</td>
        <td className={cn(CODE, 'whitespace-pre')}>
          <Highlight line={o.t} lang={lang} />
        </td>
      </tr>
    )

  const hunkRow = (key: string, label: string | null, onExpand?: () => void, count?: number) => (
    <tr key={key} className="h-8 bg-diff-hunk-bg text-[12px] whitespace-nowrap text-ink-muted">
      <td colSpan={noCols} className={cn('text-center', onExpand && 'cursor-pointer text-accent-ink')} onClick={onExpand}>
        {onExpand && <Icon name="unfold" />}
      </td>
      <td colSpan={codeCols} className="truncate">
        {onExpand && (
          <button
            type="button"
            onClick={onExpand}
            className="cursor-pointer pl-2 font-sans text-[12px] text-accent-ink hover:underline"
          >
            Expand {count} lines
          </button>
        )}
        {label && <span className="ml-3 whitespace-pre">{label}</span>}
      </td>
    </tr>
  )

  const rows: ReactNode[] = []
  items.forEach((it, idx) => {
    if (it.kind === 'gap') {
      if (expandAll || expanded.has(idx)) {
        it.lines.forEach((o, j) => rows.push(contextRow(o, `g${idx}-${j}`)))
        return
      }
      const next = items[idx + 1]
      rows.push(
        hunkRow(
          `g${idx}`,
          next ? hunkHeader(next.lines, diff.newLines) : null,
          () => setExpanded((s) => new Set(s).add(idx)),
          it.lines.length,
        ),
      )
      return
    }
    if (idx === 0) rows.push(hunkRow('h0', hunkHeader(it.lines, diff.newLines)))

    if (!split) {
      it.lines.forEach((o, j) => {
        if (o.k === ' ') return rows.push(contextRow(o, `${idx}-${j}`))
        const add = o.k === '+'
        rows.push(
          <tr key={`${idx}-${j}`} className={add ? 'bg-diff-add-bg' : 'bg-diff-del-bg'}>
            <td className={cn(NO, 'text-ink', add ? 'bg-diff-add-gutter' : 'bg-diff-del-gutter')}>{o.a}</td>
            <td className={cn(NO, 'text-ink', add ? 'bg-diff-add-gutter' : 'bg-diff-del-gutter')}>{o.b}</td>
            <td className={cn(CODE, 'whitespace-pre')}>
              <Sign k={o.k} />
              <Highlight line={o.t} lang={lang} range={o.r} markClass={add ? ADD_WORD : DEL_WORD} />
            </td>
          </tr>,
        )
      })
      return
    }

    // Split: pair removed lines (left) with added lines (right), row by row.
    const L = it.lines
    let i = 0
    while (i < L.length) {
      if (L[i].k === ' ') {
        rows.push(contextRow(L[i], `${idx}-${i}`))
        i++
        continue
      }
      const dels: DiffOp[] = []
      const adds: DiffOp[] = []
      const startKey = i
      while (i < L.length && L[i].k === '-') dels.push(L[i++])
      while (i < L.length && L[i].k === '+') adds.push(L[i++])
      for (let q = 0; q < Math.max(dels.length, adds.length); q++) {
        const x = dels[q]
        const y = adds[q]
        rows.push(
          <tr key={`${idx}-${startKey}-${q}`}>
            {x ? (
              <>
                <td className={cn(NO, 'bg-diff-del-gutter text-ink')}>{x.a}</td>
                <td className={cn(CODE, 'bg-diff-del-bg wrap-anywhere whitespace-pre-wrap')}>
                  <Sign k="-" />
                  <Highlight line={x.t} lang={lang} range={x.r} markClass={DEL_WORD} />
                </td>
              </>
            ) : (
              <>
                <td className="bg-diff-empty-bg" />
                <td className="bg-diff-empty-bg" />
              </>
            )}
            {y ? (
              <>
                <td className={cn(NO, 'border-l border-border bg-diff-add-gutter text-ink')}>{y.b}</td>
                <td className={cn(CODE, 'bg-diff-add-bg wrap-anywhere whitespace-pre-wrap')}>
                  <Sign k="+" />
                  <Highlight line={y.t} lang={lang} range={y.r} markClass={ADD_WORD} />
                </td>
              </>
            ) : (
              <>
                <td className="border-l border-border bg-diff-empty-bg" />
                <td className="bg-diff-empty-bg" />
              </>
            )}
          </tr>,
        )
      }
    }
  })

  return (
    <table className="w-full table-fixed border-collapse font-mono text-code [font-variant-ligatures:none] [&_td]:h-5">
      <colgroup>
        {split ? (
          <>
            <col className="w-12" />
            <col />
            <col className="w-12" />
            <col />
          </>
        ) : (
          <>
            <col className="w-12" />
            <col className="w-12" />
            <col />
          </>
        )}
      </colgroup>
      <tbody>
        {rows}
      </tbody>
    </table>
  )
}
