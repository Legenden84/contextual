import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Icon } from '@/components/ui/Icon'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

function when(iso: string): string {
  const d = new Date(iso)
  const sameDay = d.toDateString() === new Date().toDateString()
  return sameDay
    ? d.toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' })
    : d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })
}

/** "New chat" and the saved conversations of the open folder. */
export function ThreadMenu() {
  const { threads, thread, newChat, openThread, streaming, project, messages } = useApp()
  const [open, setOpen] = useState(false)
  const root = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => !root.current?.contains(e.target as Node) && setOpen(false)
    const esc = (e: KeyboardEvent) => e.key === 'Escape' && setOpen(false)
    document.addEventListener('mousedown', close)
    document.addEventListener('keydown', esc)
    return () => {
      document.removeEventListener('mousedown', close)
      document.removeEventListener('keydown', esc)
    }
  }, [open])

  return (
    <div ref={root} className="relative flex items-center gap-1 pr-2">
      <Button variant="ghost" size="xs" icon="plus" disabled={streaming || !messages.length} onClick={newChat}>
        New chat
      </Button>
      {project?.kind === 'folder' && (
        <Button
          variant="ghost"
          size="xs"
          icon="history"
          aria-haspopup="menu"
          aria-expanded={open}
          disabled={streaming || !threads.length}
          onClick={() => setOpen((o) => !o)}
        >
          Earlier chats
        </Button>
      )}
      {open && (
        <div role="menu" className="absolute top-full right-2 z-20 mt-1 max-h-80 w-[340px] overflow-auto rounded-md border border-border-strong bg-surface-200 p-1 shadow-popover">
          {threads.map((t) => (
            <button
              key={t.id}
              type="button"
              role="menuitem"
              onClick={() => {
                openThread(t.id)
                setOpen(false)
              }}
              className={cn(
                'flex w-full cursor-pointer items-center gap-2 rounded-sm px-2 py-1.5 text-left text-ui-body hover:bg-surface-hover',
                t.id === thread.id && 'bg-surface-selected hover:bg-surface-selected',
              )}
            >
              <Icon name="sparkle" className="size-3.5 text-ink-muted" />
              <span className="min-w-0 flex-1 truncate">{t.title}</span>
              <span className="flex-none text-ui-small text-ink-muted">
                {t.turns} · {when(t.updatedAt)}
              </span>
            </button>
          ))}
        </div>
      )}
    </div>
  )
}
