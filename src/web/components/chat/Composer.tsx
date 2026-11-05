import { useState } from 'react'
import { Button } from '@/components/ui/Button'
import { Segmented } from '@/components/ui/Segmented'
import type { ContextMode } from '@/state/app-context'
import { useApp } from '@/state/app-context'
import { ContextBar } from './ContextBar'
import { ModelPicker } from './ModelPicker'

/** Context bar, message box, deployment picker and per-message context mode. */
export function Composer() {
  const { contextMode, setContextMode, send, stop, streaming } = useApp()
  const [text, setText] = useState('')

  const submit = () => {
    const t = text.trim()
    if (!t || streaming) return
    send(t)
    setText('')
  }

  return (
    <div className="mx-auto w-full max-w-[868px] flex-none px-6 pb-4">
      <ContextBar />
      <div className="rounded-lg border border-border-strong bg-surface-200 focus-within:border-focus-ring focus-within:shadow-[0_0_0_1px_var(--focus-ring)]">
        <textarea
          aria-label="Message"
          rows={2}
          value={text}
          placeholder="Ask about the selected files…"
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
              e.preventDefault()
              submit()
            }
          }}
          className="block max-h-60 min-h-12 w-full resize-none bg-transparent px-3 pt-2.5 pb-1 text-ui-prose text-ink outline-none [field-sizing:content] placeholder:text-ink-muted"
        />
        <div className="flex items-center gap-2 pt-1 pr-1.5 pb-1.5 pl-2">
          <ModelPicker />
          <Segmented<ContextMode>
            size="sm"
            label="Context to send"
            value={contextMode}
            onChange={setContextMode}
            options={[
              { value: 'full', label: 'Resend full context' },
              { value: 'new', label: 'Only send new message' },
            ]}
          />
          <span className="flex-1" />
          <span className="text-ui-small text-ink-muted">Ctrl+Enter</span>
          {streaming ? (
            <Button size="xs" icon="close" onClick={stop}>
              Stop
            </Button>
          ) : (
            <Button variant="primary" size="icon" icon="send" aria-label="Send" disabled={!text.trim()} onClick={submit} />
          )}
        </div>
      </div>
    </div>
  )
}
