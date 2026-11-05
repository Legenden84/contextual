import { useMemo } from 'react'
import { CodeBlock } from '@/components/code/CodeBlock'
import { InlineCode } from '@/components/ui/InlineCode'
import { StatusLetter } from '@/components/ui/Status'
import { Tag } from '@/components/ui/Tag'
import type { Change } from '@/data/types'
import { computeDiff } from '@/lib/diff'
import { parseMarkdown, type Inline } from '@/lib/markdown'

export function InlineText({ inline }: { inline: Inline[] }) {
  return (
    <>
      {inline.map((s, i) =>
        s.kind === 'code' ? (
          <InlineCode key={i}>{s.text}</InlineCode>
        ) : s.kind === 'bold' ? (
          <b key={i} className="font-semibold text-ink-strong">
            {s.text}
          </b>
        ) : (
          <span key={i}>{s.text}</span>
        ),
      )}
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

/** Renders an assistant response; code blocks after `### FILE:` get the file path and status. */
export function Markdown({ source, changes }: { source: string; changes: Change[] }) {
  const blocks = useMemo(() => parseMarkdown(source), [source])
  const byPath = new Map(changes.map((c) => [c.path, c]))

  return (
    <>
      {blocks.map((b, i) => {
        switch (b.kind) {
          case 'p':
            return (
              <p key={i} className="mb-3 last:mb-0">
                <InlineText inline={b.inline} />
              </p>
            )
          case 'ul':
            return (
              <ul key={i} className="mb-3 list-disc pl-[18px]">
                {b.items.map((it, j) => (
                  <li key={j} className="my-[3px]">
                    {it.status && <StatusLetter status={it.status} className="mr-1 inline-flex align-[-2px]" />}
                    <InlineText inline={it.inline} />
                  </li>
                ))}
              </ul>
            )
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
      })}
    </>
  )
}
