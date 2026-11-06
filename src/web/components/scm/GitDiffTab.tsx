import { useEffect, useMemo, useState } from 'react'
import { DiffTable } from '@/components/diff/DiffTable'
import { Segmented } from '@/components/ui/Segmented'
import * as api from '@/lib/api'
import { computeDiff } from '@/lib/diff'
import { langFromPath } from '@/lib/syntax'
import { useApp, type DiffMode } from '@/state/app-context'

/** Diff of one changed file, like VS Code's: staged = HEAD → index, changes = index → working tree. */
export function GitDiffTab({ path, staged }: { path: string; staged: boolean }) {
  const { project, gitTick, diffMode, setDiffMode } = useApp()
  const [file, setFile] = useState<api.FileDiff | null>(null)
  const [error, setError] = useState<string | null>(null)
  const root = project?.root ?? ''

  useEffect(() => {
    let live = true
    api
      .gitDiff(root, path, staged)
      .then((f) => {
        if (!live) return
        setFile(f)
        setError(null)
      })
      .catch((err: unknown) => live && setError(err instanceof Error ? err.message : 'Could not read the diff.'))
    return () => {
      live = false
    }
  }, [root, path, staged, gitTick])

  const diff = useMemo(() => (file ? computeDiff(file.old, file.new) : null), [file])

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-7 flex-none items-center gap-2 border-b border-border pr-3 pl-4 text-ui-small text-ink-muted">
        <span className="truncate font-mono text-ink">{path}</span>
        <span>· {staged ? 'staged (HEAD → index)' : 'working tree (index → file)'}</span>
        {diff && (
          <span className="font-mono">
            <span className="text-success">+{diff.adds}</span> <span className="text-danger">−{diff.dels}</span>
          </span>
        )}
        <span className="flex-1" />
        <Segmented<DiffMode>
          size="sm"
          label="Diff layout"
          value={diffMode}
          onChange={setDiffMode}
          options={[
            { value: 'unified', label: 'Unified' },
            { value: 'split', label: 'Split' },
          ]}
        />
      </div>
      <div className="min-h-0 flex-1 overflow-auto bg-surface-100">
        {error && <p className="p-4 text-ui-small text-danger">{error}</p>}
        {!file && !error && <p className="p-4 text-ui-small text-ink-muted">Loading…</p>}
        {file?.skipped && <p className="p-4 text-ui-small text-ink-muted">Not shown: {file.skipped}.</p>}
        {file && !file.skipped && diff && (diff.adds || diff.dels ? <DiffTable diff={diff} lang={langFromPath(path)} mode={diffMode} /> : <p className="p-4 text-ui-small text-ink-muted">No differences.</p>)}
      </div>
    </div>
  )
}
