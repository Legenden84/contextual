import { useMemo, useState } from 'react'
import { Button, IconButton, LinkButton } from '@/components/ui/Button'
import { Icon } from '@/components/ui/Icon'
import type { ThreadSummary } from '@/lib/api'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

function when(iso: string): string {
  const d = new Date(iso)
  return d.toDateString() === new Date().toDateString()
    ? d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })
}

const DAY = 86_400_000

/** Today / Yesterday / Previous 7 days / Older, like other chat apps. */
function groupOf(iso: string): string {
  const startOfToday = new Date().setHours(0, 0, 0, 0)
  const t = new Date(iso).getTime()
  if (t >= startOfToday) return 'Today'
  if (t >= startOfToday - DAY) return 'Yesterday'
  if (t >= startOfToday - 7 * DAY) return 'Previous 7 days'
  return 'Older'
}

type Confirm = { kind: 'one'; id: string } | { kind: 'all' } | null

/** The saved chats of the open folder: search, open, rename, delete. */
export function ChatHistory({ onClose }: { onClose: () => void }) {
  const { threads, thread, openThread, renameThread, deleteThread, deleteAllThreads } = useApp()
  const [query, setQuery] = useState('')
  const [editing, setEditing] = useState<{ id: string; title: string } | null>(null)
  const [confirm, setConfirm] = useState<Confirm>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const groups = useMemo(() => {
    const q = query.trim().toLowerCase()
    const shown = threads.filter((t) => !q || t.title.toLowerCase().includes(q))
    const out: { label: string; items: ThreadSummary[] }[] = []
    for (const t of shown) {
      const label = groupOf(t.updatedAt)
      const g = out.find((x) => x.label === label)
      if (g) g.items.push(t)
      else out.push({ label, items: [t] })
    }
    return out
  }, [threads, query])

  const act = (fn: () => Promise<string | null>, after?: () => void) => {
    setBusy(true)
    setError(null)
    void fn().then((err) => {
      setBusy(false)
      if (err) setError(err)
      else after?.()
    })
  }

  const saveRename = () => {
    if (!editing) return
    const original = threads.find((t) => t.id === editing.id)?.title
    if (!editing.title.trim() || editing.title.trim() === original) return setEditing(null)
    act(() => renameThread(editing.id, editing.title), () => setEditing(null))
  }

  return (
    <div
      role="dialog"
      aria-label="Chats"
      className="absolute top-full right-2 z-20 mt-1 flex max-h-[480px] w-[400px] flex-col overflow-hidden rounded-md border border-border-strong bg-surface-200 shadow-popover"
    >
      <div className="flex items-center gap-2 border-b border-border p-2">
        <label className="flex h-6 flex-1 items-center gap-1.5 rounded-sm border border-border-strong bg-surface-100 px-2 focus-within:border-focus-ring">
          <Icon name="search" className="size-3.5 text-ink-muted" />
          <input
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search chats"
            aria-label="Search chats"
            className="w-full bg-transparent text-ui-small text-ink outline-none placeholder:text-ink-muted"
          />
        </label>
      </div>

      <div className="min-h-0 flex-1 overflow-auto py-1">
        {!threads.length && <p className="px-3 py-3 text-ui-small text-ink-muted">No saved chats for this folder yet.</p>}
        {!!threads.length && !groups.length && <p className="px-3 py-3 text-ui-small text-ink-muted">No chats match “{query}”.</p>}
        {groups.map((g) => (
          <div key={g.label}>
            <div className="px-3 pt-2 pb-1 text-ui-caption text-ink-muted uppercase">{g.label}</div>
            {g.items.map((t) => {
              const current = t.id === thread.id
              if (confirm?.kind === 'one' && confirm.id === t.id)
                return (
                  <div key={t.id} className="mx-1 flex items-center gap-2 rounded-sm bg-surface-hover px-2 py-1.5 text-ui-small">
                    <span className="min-w-0 flex-1 truncate text-ink">
                      Delete “{t.title}”?{current && ' A new chat starts.'}
                    </span>
                    <Button size="xs" disabled={busy} onClick={() => setConfirm(null)}>
                      Cancel
                    </Button>
                    <Button
                      size="xs"
                      disabled={busy}
                      className="border-danger bg-danger text-on-accent hover:bg-danger"
                      onClick={() => act(() => deleteThread(t.id), () => setConfirm(null))}
                    >
                      Delete
                    </Button>
                  </div>
                )
              if (editing?.id === t.id)
                return (
                  <div key={t.id} className="mx-1 px-1 py-1">
                    <input
                      autoFocus
                      value={editing.title}
                      disabled={busy}
                      aria-label="Chat name"
                      onChange={(e) => setEditing({ id: t.id, title: e.target.value })}
                      onFocus={(e) => e.target.select()}
                      onBlur={saveRename}
                      onKeyDown={(e) => {
                        if (e.key === 'Enter') saveRename()
                        if (e.key === 'Escape') {
                          e.stopPropagation()
                          setEditing(null)
                        }
                      }}
                      className="h-6 w-full rounded-sm border border-focus-ring bg-surface-100 px-2 text-ui-body text-ink outline-none"
                    />
                  </div>
                )
              return (
                <div
                  key={t.id}
                  role="button"
                  tabIndex={0}
                  onClick={() => {
                    if (!current) openThread(t.id)
                    onClose()
                  }}
                  onKeyDown={(e) => {
                    if (e.key !== 'Enter') return
                    if (!current) openThread(t.id)
                    onClose()
                  }}
                  className={cn(
                    'group mx-1 flex h-7 cursor-pointer items-center gap-2 rounded-sm px-2 text-ui-body hover:bg-surface-hover',
                    current && 'bg-surface-selected hover:bg-surface-selected',
                  )}
                >
                  <span className="min-w-0 flex-1 truncate text-ink">{t.title}</span>
                  <span className="flex-none text-ui-small text-ink-muted group-hover:hidden group-focus-within:hidden">
                    {t.turns} {t.turns === 1 ? 'turn' : 'turns'} · {when(t.updatedAt)}
                  </span>
                  <span className="hidden flex-none items-center group-hover:inline-flex group-focus-within:inline-flex">
                    <IconButton
                      icon="edit"
                      label={`Rename “${t.title}”`}
                      onClick={(e) => {
                        e.stopPropagation()
                        setConfirm(null)
                        setEditing({ id: t.id, title: t.title })
                      }}
                    />
                    <IconButton
                      icon="trash"
                      label={`Delete “${t.title}”`}
                      className="hover:text-danger"
                      onClick={(e) => {
                        e.stopPropagation()
                        setEditing(null)
                        setConfirm({ kind: 'one', id: t.id })
                      }}
                    />
                  </span>
                </div>
              )
            })}
          </div>
        ))}
      </div>

      {error && (
        <p role="alert" className="border-t border-border px-3 py-1.5 text-ui-small text-danger">
          {error}
        </p>
      )}

      {threads.length > 0 && (
        <div className="flex items-center gap-2 border-t border-border bg-surface-000 px-3 py-2 text-ui-small">
          {confirm?.kind === 'all' ? (
            <>
              <span className="min-w-0 flex-1 text-ink">
                Delete all {threads.length} {threads.length === 1 ? 'chat' : 'chats'} for this folder? This can't be undone.
              </span>
              <Button size="xs" disabled={busy} onClick={() => setConfirm(null)}>
                Cancel
              </Button>
              <Button
                size="xs"
                disabled={busy}
                className="border-danger bg-danger text-on-accent hover:bg-danger"
                onClick={() => act(deleteAllThreads, () => setConfirm(null))}
              >
                Delete all
              </Button>
            </>
          ) : (
            <>
              <span className="flex-1 text-ink-muted">
                {threads.length} {threads.length === 1 ? 'chat' : 'chats'}
              </span>
              <LinkButton className="text-danger" onClick={() => setConfirm({ kind: 'all' })}>
                <Icon name="trash" />
                Delete all chats…
              </LinkButton>
            </>
          )}
        </div>
      )}
    </div>
  )
}
