import { useMemo } from 'react'
import { Button, IconButton } from '@/components/ui/Button'
import { Checkbox } from '@/components/ui/Checkbox'
import { Icon } from '@/components/ui/Icon'
import { LineCounts, StatusLetter } from '@/components/ui/Status'
import { WarningBanner } from '@/components/ui/Banner'
import { computeDiff } from '@/lib/diff'
import { splitPath } from '@/lib/format'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

/** Right-hand panel: the files a response proposes to add, modify or delete. */
export function ChangesPanel() {
  const { changes, response, include, toggleInclude, setIncludeAll, openDiff, openApply, diffFocus, centerView } = useApp()
  const rows = useMemo(() => changes.map((c) => ({ ...c, ...computeDiff(c.old, c.new) })), [changes])

  const counts = { A: 0, M: 0, D: 0 }
  rows.forEach((r) => counts[r.status]++)
  const adds = rows.reduce((a, r) => a + r.adds, 0)
  const dels = rows.reduce((a, r) => a + r.dels, 0)
  const selected = rows.filter((r) => include[r.path]).length
  const selectable = rows.filter((r) => !r.conflict).length
  const allState = selected === 0 ? 'off' : selected >= selectable ? 'on' : 'mixed'
  const conflicts = rows.filter((r) => r.conflict && !include[r.path])

  return (
    <aside className="flex min-h-0 w-[320px] flex-none flex-col border-l border-border bg-surface-000">
      <div className="flex h-8 flex-none items-center gap-1.5 pr-2 pl-3">
        <span className="text-ui-caption text-ink-muted uppercase">Changes</span>
        {response && (
          <span className="text-ui-small text-ink-muted">
            from response #{response.number} · {response.time}
          </span>
        )}
        <span className="flex-1" />
        <IconButton icon="history" label="Earlier responses" />
      </div>

      <div className="border-b border-border px-3 pt-1 pb-3">
        <div className="text-ui-title text-ink-strong">{rows.length} files changed</div>
        <div className="mt-1 flex gap-3 text-ui-small text-ink-muted">
          {(['A', 'M', 'D'] as const).map((s) => (
            <span key={s} className="inline-flex items-center gap-[5px]">
              <StatusLetter status={s} />
              {counts[s]} {s === 'A' ? 'added' : s === 'M' ? 'modified' : 'deleted'}
            </span>
          ))}
        </div>
        <div className="mt-0.5 font-mono text-[12px] text-ink-muted">
          <span className="font-semibold text-success">+{adds}</span>{' '}
          <span className="font-semibold text-danger">−{dels}</span> lines
        </div>
      </div>

      <div className="flex-1 overflow-auto py-1">
        <div className="flex h-control items-center gap-2 px-3 text-ui-small text-ink-muted">
          <Checkbox state={allState} label="Select all for apply" onChange={() => setIncludeAll(allState !== 'on')} />
          <span>
            {selected} of {rows.length} selected for apply
          </span>
        </div>
        {rows.map((r) => {
          const { dir } = splitPath(r.path)
          const name = r.path.slice(dir.length)
          const on = !!include[r.path]
          return (
            <div
              key={r.path}
              onClick={() => openDiff(r.path)}
              className={cn(
                'flex cursor-pointer items-center gap-2 px-3 py-[5px] hover:bg-surface-hover',
                centerView === 'diff' && diffFocus === r.path && 'bg-surface-selected hover:bg-surface-selected',
              )}
            >
              <Checkbox state={on} label={`Include ${name} in apply`} onChange={() => toggleInclude(r.path)} />
              <StatusLetter status={r.status} />
              <span className="flex min-w-0 flex-1 flex-col leading-4">
                <span className={cn('inline-flex items-center gap-1 truncate text-ui-body leading-4', on ? 'text-ink' : 'text-ink-muted')}>
                  {name}
                  {r.conflict && (
                    <span className="inline-flex text-warning" title="Conflict: modified on disk">
                      <Icon name="warning" className="size-[13px]" />
                    </span>
                  )}
                </span>
                <span className="truncate font-mono text-[11px] text-ink-muted [direction:rtl] text-left">
                  {dir.replace(/^src\//, '').replace(/\/$/, '')}
                </span>
              </span>
              <LineCounts adds={r.adds} dels={r.dels} className="font-normal" />
            </div>
          )
        })}
      </div>

      {conflicts.map((c) => (
        <WarningBanner key={c.path} className="mx-3 mb-3 items-start rounded-md">
          <span>
            <b className="font-semibold">{splitPath(c.path).name}</b> changed on disk after it was sent. Excluded from
            apply until you review it.
          </span>
        </WarningBanner>
      ))}

      <div className="flex gap-2 border-t border-border p-3">
        <Button icon="split" onClick={() => openDiff()}>
          Open diff
        </Button>
        <Button variant="primary" className="flex-1" disabled={selected === 0} onClick={openApply}>
          Apply {selected} {selected === 1 ? 'file' : 'files'}…
        </Button>
      </div>
    </aside>
  )
}
