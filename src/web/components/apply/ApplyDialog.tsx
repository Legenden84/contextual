import { useEffect, useMemo, useState } from 'react'
import { Button, IconButton } from '@/components/ui/Button'
import { Checkbox, CheckLabel } from '@/components/ui/Checkbox'
import { Icon } from '@/components/ui/Icon'
import { LineCounts } from '@/components/ui/Status'
import { computeDiff } from '@/lib/diff'
import { splitPath } from '@/lib/format'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

const VERB = {
  A: ['create', 'text-success'],
  M: ['overwrite', 'text-modified'],
  D: ['delete', 'text-danger'],
} as const

/** Lists what will happen to each selected file before anything is written. */
export function ApplyDialog() {
  const { changes, include, closeApply, apply, project } = useApp()
  const [backup, setBackup] = useState(true)
  const [reveal, setReveal] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const run = () => {
    setBusy(true)
    setError(null)
    void apply(backup).then((err) => {
      setBusy(false)
      if (err) setError(err)
    })
  }

  const picked = useMemo(
    () => changes.filter((c) => include[c.path]).map((c) => ({ ...c, ...computeDiff(c.old, c.new) })),
    [changes, include],
  )
  const skipped = changes.filter((c) => !include[c.path])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && closeApply()
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [closeApply])

  return (
    <div className="absolute inset-0 z-10 flex items-center justify-center bg-surface-scrim" onClick={closeApply}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="apply-title"
        onClick={(e) => e.stopPropagation()}
        className="w-[640px] overflow-hidden rounded-lg border border-border-strong bg-surface-100 shadow-modal"
      >
        <div className="flex items-center pt-3 pr-3 pb-2 pl-4">
          <h2 id="apply-title" className="text-ui-title text-ink-strong">
            Apply changes
          </h2>
          <span className="flex-1" />
          <IconButton icon="close" label="Close" onClick={closeApply} />
        </div>

        <div className="flex flex-col gap-3 px-4 pb-4">
          <p className="text-ink-muted">
            Saved in place in <span className="font-mono text-[12px]">{project?.root}</span>
            {project?.branch && (
              <>
                {' '}
                on <span className="font-mono text-[12px]">{project.branch}</span>
              </>
            )}
            , like Save in your editor: nothing is staged or committed, and each file keeps its line endings.
          </p>

          <div className="overflow-hidden rounded-md border border-border">
            {picked.map((c) => (
              <div key={c.path} className="flex h-[34px] items-center gap-3 border-b border-border px-3 last:border-b-0">
                <span className={cn('w-[72px] flex-none text-[11px] font-semibold tracking-[0.04em] uppercase', VERB[c.status][1])}>
                  {VERB[c.status][0]}
                </span>
                <span className="min-w-0 truncate font-mono text-[12px] text-ink [direction:rtl] text-left">{c.path}</span>
                <span className="flex-1" />
                <LineCounts adds={c.adds} dels={c.dels} className="font-normal" />
              </div>
            ))}
          </div>

          {skipped.length > 0 && (
            <div className="flex items-center gap-2 text-ui-small text-ink-muted">
              <Icon name="warning" className="size-3.5 text-warning" />
              <span>
                Skipped:{' '}
                {skipped.map((c, i) => (
                  <span key={c.path}>
                    {i > 0 && ', '}
                    <span className="font-mono">{splitPath(c.path).name}</span> ({c.conflict ? 'conflict, ' : ''}not selected)
                  </span>
                ))}
              </span>
            </div>
          )}

          <CheckLabel onToggle={() => setBackup((b) => !b)} control={<Checkbox state={backup} onChange={() => setBackup((b) => !b)} />}>
            Create backup before overwriting
            <span className="text-ui-small text-ink-muted">kept outside the project, for Undo</span>
          </CheckLabel>
          <CheckLabel onToggle={() => setReveal((r) => !r)} control={<Checkbox state={reveal} onChange={() => setReveal((r) => !r)} />}>
            Show in explorer after apply
          </CheckLabel>
          {!backup && (
            <p className="text-ui-small text-warning">Without a backup, this save cannot be undone from Contextual (git can still restore committed files).</p>
          )}
          {error && (
            <p role="alert" className="flex items-start gap-1.5 text-ui-small text-danger">
              <Icon name="warning" className="mt-px size-3.5" />
              {error}
            </p>
          )}
        </div>

        <div className="flex items-center gap-2 border-t border-border bg-surface-000 px-4 py-3">
          <span className="text-ui-small text-ink-muted">Saves the exact file contents from the response. No merging.</span>
          <span className="flex-1" />
          <Button onClick={closeApply}>Cancel</Button>
          <Button variant="primary" disabled={picked.length === 0 || busy} onClick={run} autoFocus>
            {busy ? 'Saving…' : `Apply ${picked.length} ${picked.length === 1 ? 'file' : 'files'}`}
          </Button>
        </div>
      </div>
    </div>
  )
}
