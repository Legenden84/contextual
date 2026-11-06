import { useEffect, useMemo, useRef, useState, type KeyboardEvent, type ReactNode } from 'react'
import { Icon, type IconName } from '@/components/ui/Icon'
import * as api from '@/lib/api'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

type Item =
  | { kind: 'create'; label: string; name: string }
  | { kind: 'section'; label: string }
  | { kind: 'branch'; branch: api.BranchInfo; remote: boolean }

/**
 * VS Code-style branch quick pick: filter, switch to a local or remote branch, or create a new
 * branch from the current one. Opened from the branch name in the status bar.
 */
export function BranchPicker() {
  const { project, git, closeBranchPicker, branchChanged } = useApp()
  const root = project?.root ?? ''
  const [list, setList] = useState<api.BranchList | null>(null)
  const [query, setQuery] = useState('')
  const [mode, setMode] = useState<'pick' | 'create'>('pick')
  const [active, setActive] = useState(0)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const listRef = useRef<HTMLDivElement>(null)
  const current = list?.current ?? git?.branch ?? null

  // Show local data at once, then refresh the remote branches in the background.
  useEffect(() => {
    let live = true
    api.listBranches(root).then((l) => live && setList(l)).catch(() => {})
    api.listBranches(root, true).then((l) => live && setList(l)).catch(() => {})
    return () => {
      live = false
    }
  }, [root])

  const items = useMemo<Item[]>(() => {
    const q = query.trim().toLowerCase()
    const match = (b: api.BranchInfo) => !q || b.name.toLowerCase().includes(q)
    const local = (list?.local ?? []).filter(match)
    const remote = (list?.remote ?? []).filter(match)
    const out: Item[] = [
      q
        ? { kind: 'create', label: `Create new branch “${query.trim()}”`, name: query.trim() }
        : { kind: 'create', label: 'Create new branch…', name: '' },
    ]
    if (local.length) out.push({ kind: 'section', label: 'branches' }, ...local.map((b) => ({ kind: 'branch' as const, branch: b, remote: false })))
    if (remote.length)
      out.push({ kind: 'section', label: 'remote branches' }, ...remote.map((b) => ({ kind: 'branch' as const, branch: b, remote: true })))
    return out
  }, [list, query])

  const selectable = items.map((it, i) => (it.kind === 'section' ? -1 : i)).filter((i) => i >= 0)
  const activeIndex = selectable.includes(active) ? active : (selectable[0] ?? 0)

  const run = (what: () => Promise<unknown>) => {
    setBusy(true)
    setError(null)
    what()
      .then(() => branchChanged())
      .catch((err: unknown) => setError(err instanceof Error ? err.message : 'Git failed.'))
      .finally(() => setBusy(false))
  }

  const choose = (it: Item | undefined) => {
    if (!it || busy) return
    if (it.kind === 'create') {
      if (it.name) run(() => api.createBranch(root, it.name))
      else {
        setMode('create')
        setQuery('')
      }
    } else if (it.kind === 'branch') {
      if (!it.remote && it.branch.name === current) return closeBranchPicker()
      run(() => api.switchBranch(root, it.branch.name))
    }
  }

  const onKey = (e: KeyboardEvent) => {
    if (e.key === 'Escape') {
      e.preventDefault()
      if (mode === 'create') {
        setMode('pick')
        setQuery('')
      } else closeBranchPicker()
    } else if (mode === 'create' && e.key === 'Enter') {
      e.preventDefault()
      if (query.trim()) run(() => api.createBranch(root, query.trim()))
    } else if (mode === 'pick' && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) {
      e.preventDefault()
      const pos = selectable.indexOf(activeIndex)
      const next = selectable[Math.max(0, Math.min(selectable.length - 1, pos + (e.key === 'ArrowDown' ? 1 : -1)))]
      setActive(next)
      listRef.current?.querySelector(`[data-index="${next}"]`)?.scrollIntoView({ block: 'nearest' })
    } else if (mode === 'pick' && e.key === 'Enter') {
      e.preventDefault()
      choose(items[activeIndex])
    }
  }

  const row = (icon: IconName, label: ReactNode, detail: ReactNode, i: number, onClick: () => void) => (
    <div
      key={i}
      data-index={i}
      role="option"
      aria-selected={i === activeIndex}
      onMouseMove={() => setActive(i)}
      onClick={onClick}
      className={cn('flex h-[26px] cursor-pointer items-center gap-2 px-3 text-ui-body', i === activeIndex && 'bg-surface-selected')}
    >
      <Icon name={icon} className="size-3.5 flex-none text-ink-muted" />
      <span className="min-w-0 truncate text-ink">{label}</span>
      <span className="min-w-0 flex-1 truncate text-right text-ui-small text-ink-muted">{detail}</span>
    </div>
  )

  return (
    <div className="absolute inset-0 z-30" onMouseDown={closeBranchPicker}>
      <div
        role="dialog"
        aria-label="Switch branch"
        onMouseDown={(e) => e.stopPropagation()}
        className="absolute top-1.5 left-1/2 w-[620px] max-w-[94%] -translate-x-1/2 overflow-hidden rounded-md border border-border-strong bg-surface-200 shadow-modal"
      >
        <div className="p-1.5">
          <input
            autoFocus
            value={query}
            disabled={busy}
            onChange={(e) => {
              setQuery(e.target.value)
              setActive(0)
            }}
            onKeyDown={onKey}
            placeholder={mode === 'pick' ? 'Select a branch to switch to' : 'Name of the new branch (Enter to create, Esc to go back)'}
            aria-label={mode === 'pick' ? 'Filter branches' : 'New branch name'}
            className="h-control w-full rounded-sm border border-focus-ring bg-surface-100 px-2 text-ui-body text-ink outline-none placeholder:text-ink-muted"
          />
        </div>

        {mode === 'create' ? (
          <p className="px-3 pb-2 text-ui-small text-ink-muted">
            Creates the branch from <span className="font-mono text-ink">{current ?? 'HEAD'}</span> and switches to it. Your uncommitted
            changes come along.
          </p>
        ) : (
          <div ref={listRef} role="listbox" className="max-h-[420px] overflow-auto pb-1">
            {items.map((it, i) =>
              it.kind === 'section' ? (
                <div key={i} className="mt-1 border-t border-border px-3 pt-1 text-right text-ui-caption text-ink-muted">
                  {it.label}
                </div>
              ) : it.kind === 'create' ? (
                row('plus', it.label, it.name ? `from ${current ?? 'HEAD'}` : '', i, () => choose(it))
              ) : (
                row(
                  it.remote ? 'cloud' : 'branch',
                  <>
                    {it.branch.name}
                    {!it.remote && it.branch.name === current && <span className="ml-1.5 text-ui-small text-accent-ink">current</span>}
                  </>,
                  <>
                    <span className="font-mono">{it.branch.sha}</span> · {it.branch.subject} · {it.branch.when}
                  </>,
                  i,
                  () => choose(it),
                )
              ),
            )}
            {!list && <p className="px-3 py-1 text-ui-small text-ink-muted">Loading branches…</p>}
          </div>
        )}

        {(busy || error) && (
          <p role={error ? 'alert' : 'status'} className={cn('border-t border-border px-3 py-1.5 text-ui-small', error ? 'text-danger' : 'text-ink-muted')}>
            {error ?? 'Working…'}
          </p>
        )}
      </div>
    </div>
  )
}
