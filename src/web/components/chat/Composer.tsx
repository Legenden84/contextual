import { useRef, useState, type ClipboardEvent, type DragEvent } from 'react'
import { Button, IconButton } from '@/components/ui/Button'
import { Icon } from '@/components/ui/Icon'
import { Segmented } from '@/components/ui/Segmented'
import { fmtK } from '@/lib/format'
import { IMAGE_TYPES } from '@/lib/images'
import { cn } from '@/lib/utils'
import type { ContextMode } from '@/state/app-context'
import { useApp } from '@/state/app-context'
import { ContextBar } from './ContextBar'
import { ModelPicker } from './ModelPicker'

const imagesIn = (list: FileList | DataTransferItemList | null | undefined): File[] => {
  if (!list) return []
  const out: File[] = []
  for (const item of Array.from(list as ArrayLike<File | DataTransferItem>)) {
    const f = item instanceof File ? item : item.kind === 'file' ? item.getAsFile() : null
    if (f && IMAGE_TYPES.includes(f.type)) out.push(f)
  }
  return out
}

/** Context bar, message box with picture attachments, deployment picker and per-message context mode. */
export function Composer() {
  const { contextMode, setContextMode, send, stop, streaming, attachments, addAttachments, removeAttachment } = useApp()
  const [text, setText] = useState('')
  const [dragging, setDragging] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const picker = useRef<HTMLInputElement>(null)

  const attach = (files: File[]) => {
    if (!files.length) return
    setError(null)
    void addAttachments(files).then(setError)
  }

  const submit = () => {
    const t = text.trim() || (attachments.length ? 'See the attached picture(s).' : '')
    if (!t || streaming) return
    send(t)
    setText('')
    setError(null)
  }

  const onPaste = (e: ClipboardEvent) => {
    const files = imagesIn(e.clipboardData?.items)
    if (files.length) {
      e.preventDefault()
      attach(files)
    }
  }

  const onDrop = (e: DragEvent) => {
    e.preventDefault()
    setDragging(false)
    attach(imagesIn(e.dataTransfer?.files))
  }

  return (
    <div className="mx-auto w-full max-w-[868px] flex-none px-6 pb-4">
      <ContextBar />
      <div
        onDragOver={(e) => {
          if (Array.from(e.dataTransfer.items).some((i) => i.kind === 'file')) {
            e.preventDefault()
            setDragging(true)
          }
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={onDrop}
        className={cn(
          'rounded-lg border border-border-strong bg-surface-200 focus-within:border-focus-ring focus-within:shadow-[0_0_0_1px_var(--focus-ring)]',
          dragging && 'border-focus-ring bg-surface-hover',
        )}
      >
        {attachments.length > 0 && (
          <div className="flex flex-wrap gap-2 px-3 pt-2.5">
            {attachments.map((a) => (
              <figure
                key={a.id}
                title={`${a.name} · ${a.width}×${a.height} · ~${fmtK(a.tokens)} tokens`}
                className="group relative h-14 overflow-hidden rounded-md border border-border bg-surface-100"
              >
                <img src={a.url} alt={a.name} className="h-full w-auto max-w-[160px] object-cover" />
                <figcaption className="absolute inset-x-0 bottom-0 truncate bg-surface-scrim px-1 font-mono text-[10px] text-ink">
                  ~{fmtK(a.tokens)} tokens
                </figcaption>
                <IconButton
                  icon="close"
                  label={`Remove ${a.name}`}
                  onClick={() => removeAttachment(a.id)}
                  className="absolute top-0.5 right-0.5 bg-surface-200 opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
                />
              </figure>
            ))}
          </div>
        )}
        <textarea
          aria-label="Message"
          rows={2}
          value={text}
          placeholder={dragging ? 'Drop pictures to attach them…' : 'Ask about the selected files… (paste or drop screenshots)'}
          onChange={(e) => setText(e.target.value)}
          onPaste={onPaste}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && (e.ctrlKey || e.metaKey)) {
              e.preventDefault()
              submit()
            }
          }}
          className="block max-h-60 min-h-12 w-full resize-none bg-transparent px-3 pt-2.5 pb-1 text-ui-prose text-ink outline-none [field-sizing:content] placeholder:text-ink-muted"
        />
        {error && (
          <p role="alert" className="flex items-start gap-1.5 px-3 pb-1 text-ui-small text-danger">
            <Icon name="warning" className="mt-px size-3.5" />
            {error}
          </p>
        )}
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
          <IconButton icon="image" label="Attach pictures" onClick={() => picker.current?.click()} />
          <input
            ref={picker}
            type="file"
            accept={IMAGE_TYPES.join(',')}
            multiple
            className="hidden"
            onChange={(e) => {
              attach(imagesIn(e.target.files))
              e.target.value = ''
            }}
          />
          <span className="flex-1" />
          <span className="text-ui-small text-ink-muted">Ctrl+Enter</span>
          {streaming ? (
            <Button size="xs" icon="close" onClick={stop}>
              Stop
            </Button>
          ) : (
            <Button
              variant="primary"
              size="icon"
              icon="send"
              aria-label="Send"
              disabled={!text.trim() && !attachments.length}
              onClick={submit}
            />
          )}
        </div>
      </div>
    </div>
  )
}
