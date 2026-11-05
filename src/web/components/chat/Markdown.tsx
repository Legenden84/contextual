import { useMemo, type ReactNode } from 'react'
import { CodeBlock } from '@/components/code/CodeBlock'
import { Checkbox } from '@/components/ui/Checkbox'
import { InlineCode } from '@/components/ui/InlineCode'
import { StatusLetter } from '@/components/ui/Status'
import { Tag } from '@/components/ui/Tag'
import type { Change } from '@/data/types'
import { computeDiff } from '@/lib/diff'
import { parseMarkdown, type Block, type Inline, type ListItem } from '@/lib/markdown'
import { cn } from '@/lib/utils'

/** Only web links open; anything else (javascript:, file:, relative) renders as plain text. */
const safeHref = (href: string) => (/^https?:\/\//i.test(href) ? href : null)

export function InlineText({ inline }: { inline: Inline[] }) {
  return (
    <>
      {inline.map((s, i) => {
        switch (s.kind) {
          case 'code':
            return <InlineCode key={i}>{s.text}</InlineCode>
          case 'bold':
            return (
              <b key={i} className="font-semibold text-ink-strong">
                <InlineText inline={s.children} />
              </b>
            )
          case 'italic':
            return (
              <em key={i} className="italic">
                <InlineText inline={s.children} />
              </em>
            )
          case 'strike':
            return (
              <s key={i} className="text-ink-muted">
                <InlineText inline={s.children} />
              </s>
            )
          case 'link': {
            const href = safeHref(s.href)
            return href ? (
              <a key={i} href={href} target="_blank" rel="noreferrer" className="text-accent-ink hover:underline">
                <InlineText inline={s.children} />
              </a>
            ) : (
              <span key={i}>
                <InlineText inline={s.children} />
              </span>
            )
          }
          default:
            return <span key={i}>{s.text}</span>
        }
      })}
    </>
  )
}

function FileBadge({ change }: { change: Change }) {
  const d = useMemo(() => computeDiff(change.old, change.new), [change])
  if (change.status === 'A')
    return (
      <Tag tone="add">
        <StatusLetter status="A" className="size-3 text-[9px] leading-3" />
        new file
      </Tag>
    )
  return (
    <Tag>
      <StatusLetter status={change.status} className="size-3 text-[9px] leading-3" />+{d.adds} −{d.dels}
    </Tag>
  )
}

const HEADING_CLASS: Record<number, string> = {
  1: 'mt-5 mb-2 border-b border-border pb-1 text-ui-title text-ink-strong',
  2: 'mt-5 mb-2 text-ui-title text-ink-strong',
  3: 'mt-4 mb-1.5 text-ui-prose font-semibold text-ink-strong',
  4: 'mt-3 mb-1 text-ui-body font-semibold text-ink-strong',
  5: 'mt-3 mb-1 text-ui-body font-semibold text-ink',
  6: 'mt-3 mb-1 text-ui-small font-semibold text-ink-muted uppercase',
}

/** Lists keep their nesting by indentation; numbered and bullet items can mix. */
function List({ items }: { items: ListItem[] }) {
  return (
    <ul className="mb-3 last:mb-0">
      {items.map((it, j) => (
        <li key={j} className="my-[3px] flex gap-2" style={{ paddingLeft: it.depth * 18 }}>
          <span className="w-4 flex-none text-right text-ink-muted select-none">
            {it.checked != null ? (
              <Checkbox state={it.checked} disabled className="mt-1 cursor-default opacity-100" />
            ) : it.number ? (
              <span className="font-mono text-[12px]">{it.number}</span>
            ) : (
              <span aria-hidden>{it.depth % 2 ? '◦' : '•'}</span>
            )}
          </span>
          <span className="min-w-0 flex-1">
            {it.status && <StatusLetter status={it.status} className="mr-1 inline-flex align-[-2px]" />}
            <InlineText inline={it.inline} />
          </span>
        </li>
      ))}
    </ul>
  )
}

function Blocks({ blocks, byPath }: { blocks: Block[]; byPath: Map<string, Change> }): ReactNode {
  return blocks.map((b, i) => {
    switch (b.kind) {
      case 'p':
        return (
          <p key={i} className="mb-3 last:mb-0">
            <InlineText inline={b.inline} />
          </p>
        )
      case 'heading': {
        // h1 in a reply becomes <h2>: the page already has its own top-level heading.
        const H = `h${Math.min(b.level + 1, 6)}` as 'h2'
        return (
          <H key={i} className={cn(HEADING_CLASS[b.level], 'first:mt-0')}>
            <InlineText inline={b.inline} />
          </H>
        )
      }
      case 'list':
        return <List key={i} items={b.items} />
      case 'quote':
        return (
          <blockquote key={i} className="mb-3 border-l-2 border-border-strong pl-3 text-ink-muted last:mb-0">
            <Blocks blocks={b.blocks} byPath={byPath} />
          </blockquote>
        )
      case 'table':
        return (
          <div key={i} className="mb-3 overflow-x-auto last:mb-0">
            <table className="w-full border-collapse text-ui-body">
              <thead>
                <tr className="bg-surface-200">
                  {b.head.map((c, j) => (
                    <th key={j} style={{ textAlign: b.align[j] ?? 'left' }} className="border border-border px-2.5 py-1 font-semibold text-ink-strong">
                      <InlineText inline={c} />
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {b.rows.map((r, j) => (
                  <tr key={j}>
                    {b.head.map((_, k) => (
                      <td key={k} style={{ textAlign: b.align[k] ?? 'left' }} className="border border-border px-2.5 py-1 align-top">
                        <InlineText inline={r[k] ?? []} />
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )
      case 'hr':
        return <hr key={i} className="my-4 border-t border-border" />
      case 'file':
      case 'delete':
        return (
          <p key={i} className="mt-4 font-mono text-[12px] text-ink-muted">
            ### {b.kind === 'file' ? 'FILE' : 'DELETE'}: {b.path}
          </p>
        )
      case 'code': {
        const change = b.path ? byPath.get(b.path) : undefined
        return (
          <CodeBlock
            key={i}
            code={b.code}
            path={b.path}
            lang={b.lang}
            maxLines={change?.status === 'M' ? 9 : 28}
            badge={change && <FileBadge change={change} />}
          />
        )
      }
    }
  })
}

/** Renders an assistant response; code blocks after `### FILE:` get the file path and status. */
export function Markdown({ source, changes }: { source: string; changes: Change[] }) {
  const blocks = useMemo(() => parseMarkdown(source), [source])
  const byPath = useMemo(() => new Map(changes.map((c) => [c.path, c])), [changes])
  return <Blocks blocks={blocks} byPath={byPath} />
}
