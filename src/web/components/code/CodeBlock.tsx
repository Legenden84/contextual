import { useState, type ReactNode } from 'react'
import { Button } from '@/components/ui/Button'
import { FileGlyph } from '@/components/ui/FileGlyph'
import { Icon } from '@/components/ui/Icon'
import { useCopy } from '@/hooks/useCopy'
import { LANG_LABEL, langFromPath, type Lang } from '@/lib/syntax'
import { CodeView } from './CodeView'

export interface CodeBlockProps {
  code: string
  path?: string
  lang?: Lang
  /** Show at most this many lines, then "Show N more lines". */
  maxLines?: number
  /** Extra element after the path, e.g. a "new file" tag. */
  badge?: ReactNode
}

/** Code block in chat: header bar with path or language, Wrap lines and Copy. */
export function CodeBlock({ code, path, lang, maxLines, badge }: CodeBlockProps) {
  const language = lang ?? langFromPath(path)
  const [wrap, setWrap] = useState(false)
  const [expanded, setExpanded] = useState(false)
  const [copied, copy] = useCopy()

  const lines = code.split('\n')
  const cut = !expanded && maxLines != null && lines.length > maxLines
  const shown = cut ? lines.slice(0, maxLines).join('\n') : code

  return (
    <div className="mt-2 mb-3 overflow-hidden rounded-md border border-border bg-surface-100">
      <div className="flex h-8 items-center gap-2 border-b border-border bg-surface-200 pr-2 pl-3">
        {path ? (
          <>
            <FileGlyph name={path} />
            <span className="truncate font-mono text-[12px] text-ink">{path}</span>
            {badge}
          </>
        ) : (
          <span className="text-[11px] text-ink-muted">{LANG_LABEL[language]}</span>
        )}
        <span className="flex-1" />
        {path && <span className="text-[11px] text-ink-muted">{LANG_LABEL[language]}</span>}
        <Button variant="ghost" size="xs" icon="wrap" aria-pressed={wrap} onClick={() => setWrap((w) => !w)}>
          Wrap lines
        </Button>
        <Button variant="ghost" size="xs" icon="copy" onClick={() => copy(code)}>
          {copied ? 'Copied' : 'Copy'}
        </Button>
      </div>
      <CodeView code={shown} lang={language} wrap={wrap} />
      {cut && (
        <button
          type="button"
          onClick={() => setExpanded(true)}
          className="flex h-7 w-full cursor-pointer items-center justify-center gap-1.5 border-t border-border bg-surface-200 text-ui-small text-accent-ink"
        >
          <Icon name="unfold" />
          Show {lines.length - maxLines!} more lines
        </button>
      )}
    </div>
  )
}
