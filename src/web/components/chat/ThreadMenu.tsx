import { useEffect, useRef, useState } from 'react'
import { Button } from '@/components/ui/Button'
import { ChatHistory } from './ChatHistory'
import { useApp } from '@/state/app-context'

/** Pull request, New chat, and the Chats manager (saved conversations of the open folder). */
export function ThreadMenu() {
  const { newChat, streaming, project, messages, openPr } = useApp()
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
      {project && (
        <Button
          variant="ghost"
          size="xs"
          icon="branch"
          disabled={streaming}
          onClick={openPr}
          title="Compare this branch with its base and open a pull request"
        >
          Pull request
        </Button>
      )}
      <Button variant="ghost" size="xs" icon="plus" disabled={streaming || !messages.length} onClick={newChat}>
        New chat
      </Button>
      {project && (
        <Button
          variant="ghost"
          size="xs"
          icon="history"
          aria-haspopup="dialog"
          aria-expanded={open}
          disabled={streaming}
          onClick={() => setOpen((o) => !o)}
        >
          Chats
        </Button>
      )}
      {open && <ChatHistory onClose={() => setOpen(false)} />}
    </div>
  )
}
