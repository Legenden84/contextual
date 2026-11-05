import { Fragment, useEffect, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { useCopy } from '@/hooks/useCopy'
import type { FileContent } from '@/lib/api'
import { fmtN } from '@/lib/format'
import { LANG_LABEL, langFromPath } from '@/lib/syntax'
import { findNode } from '@/lib/tree'
import { useApp } from '@/state/app-context'
import { CodeView } from './CodeView'

/** Read-only preview tab opened from the explorer. */
export function CodePreview({ path }: { path: string }) {
  const { tree, selected, loadFile, project } = useApp()
  const [wrap, setWrap] = useState(false)
  const [current, setCurrent] = useState<number | undefined>(project?.kind === 'sample' ? 34 : undefined)
  const [file, setFile] = useState<FileContent | null>(null)
  const [copied, copy] = useCopy()
  const node = findNode(tree, path)
  const parts = path.split('/')
  const lang = langFromPath(path)
  const content = file?.content

  // Reload when the scan reports a new modified time for this file.
  const mtime = node?.mtimeMs
  useEffect(() => {
    let live = true
    void loadFile(path).then((f) => live && setFile(f))
    return () => {
      live = false
    }
  }, [loadFile, path, mtime])

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-7 flex-none items-center gap-1.5 border-b border-border pr-3 pl-4 text-ui-small text-ink-muted">
        {parts.map((p, i) => (
          <Fragment key={i}>
            {i > 0 && <span>›</span>}
            {i === parts.length - 1 ? <b className="font-medium text-ink">{p}</b> : <span>{p}</span>}
          </Fragment>
        ))}
        <span className="flex-1" />
        <span>
          {LANG_LABEL[lang]} · UTF-8 · ~{fmtN(node?.tokens ?? 0)} tokens · {selected.has(path) ? 'in context' : 'not in context'}
        </span>
        <Button variant="ghost" size="xs" icon="copy" disabled={content == null} onClick={() => content != null && copy(content)}>
          {copied ? 'Copied' : 'Copy'}
        </Button>
        <Button variant="ghost" size="xs" icon="wrap" aria-pressed={wrap} onClick={() => setWrap((w) => !w)}>
          Wrap lines
        </Button>
      </div>
      {content != null ? (
        <CodeView
          code={content}
          path={path}
          wrap={wrap}
          currentLine={current}
          activeGuide={1}
          onLineClick={setCurrent}
          className="min-h-0 flex-1"
        />
      ) : (
        <div className="flex flex-1 items-center justify-center bg-surface-100 text-ui-small text-ink-muted">
          {file ? `Cannot preview this file: ${file.error}` : 'Loading…'}
        </div>
      )}
    </div>
  )
}
