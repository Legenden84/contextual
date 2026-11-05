import { useEffect, useMemo, useRef, useState, type ReactNode } from 'react'
import { DiffFile } from '@/components/diff/DiffFile'
import { Button } from '@/components/ui/Button'
import { Icon } from '@/components/ui/Icon'
import { Segmented } from '@/components/ui/Segmented'
import { Chip, Tag } from '@/components/ui/Tag'
import { computeDiff } from '@/lib/diff'
import { parseInline } from '@/lib/markdown'
import { useApp, type ChatMessage } from '@/state/app-context'
import { InlineText, Markdown } from './Markdown'

const PROSE = 'pl-[30px] text-ui-prose text-ink'

function Avatar({ ai }: { ai?: boolean }) {
  return ai ? (
    <span className="inline-flex size-[22px] items-center justify-center rounded-full bg-accent text-on-accent">
      <Icon name="sparkle" className="size-[13px]" />
    </span>
  ) : (
    <span className="inline-flex size-[22px] items-center justify-center rounded-full bg-surface-300 text-[10px] font-semibold text-ink">
      TL
    </span>
  )
}

function MessageHead({ children }: { children: ReactNode }) {
  return <div className="mb-1.5 flex h-[26px] items-center gap-2 text-ui-body">{children}</div>
}

function UserMessage({ msg }: { msg: ChatMessage }) {
  return (
    <div className="mb-6">
      <MessageHead>
        <Avatar />
        <b className="font-semibold text-ink-strong">You</b>
        <span className="text-ui-small text-ink-muted">{msg.time}</span>
        <span className="flex-1" />
        <Chip>
          <Icon name="files" />
          {msg.meta}
        </Chip>
      </MessageHead>
      <div className={`${PROSE} ml-[30px] rounded-lg border border-border bg-surface-200 px-4 py-3`}>
        <p>
          <InlineText inline={parseInline(msg.text)} />
        </p>
        {!!msg.images?.length && (
          <div className="mt-2 flex flex-wrap gap-2">
            {msg.images.map((img, i) => (
              <a key={i} href={img.url} target="_blank" rel="noreferrer" title={img.name}>
                <img
                  src={img.url}
                  alt={img.name}
                  className="h-24 w-auto max-w-[240px] rounded-md border border-border object-cover"
                />
              </a>
            ))}
          </div>
        )}
      </div>
    </div>
  )
}

function AssistantMessage({ msg }: { msg: ChatMessage }) {
  const { include, toggleInclude, diffMode, openDiff, response } = useApp()
  const [view, setView] = useState<'response' | 'diff'>('response')
  const changes = useMemo(() => msg.changes ?? [], [msg.changes])
  const current = response?.id === msg.id
  const sum = useMemo(
    () =>
      changes.reduce(
        (a, c) => {
          const d = computeDiff(c.old, c.new)
          return { adds: a.adds + d.adds, dels: a.dels + d.dels }
        },
        { adds: 0, dels: 0 },
      ),
    [changes],
  )

  return (
    <div className="mb-6">
      <MessageHead>
        <Avatar ai />
        <b className="font-semibold text-ink-strong">{msg.model}</b>
        <span className="text-ui-small text-ink-muted">
          Foundry · {msg.time}
          {msg.meta && ` · ${msg.meta}`}
        </span>
        {msg.api === 'responses' && (
          <Tag title="Azure stores this conversation (Responses API), so the next message only uploads what's new.">context kept by Azure</Tag>
        )}
        <span className="flex-1" />
        {changes.length > 0 && (
          <Segmented
            size="sm"
            label="Show response as"
            value={view}
            onChange={setView}
            options={[
              { value: 'response', label: 'Response' },
              { value: 'diff', label: 'Diff' },
            ]}
          />
        )}
      </MessageHead>
      <div className={PROSE}>
        {msg.streaming && !msg.text ? (
          <p className="inline-flex items-center gap-2 text-ink-muted">
            <span className="size-2 animate-pulse rounded-full bg-accent" />
            Waiting for {msg.model}…
          </p>
        ) : view === 'response' || !changes.length ? (
          <>
            <Markdown source={msg.text} changes={changes} />
            {msg.streaming && <span className="ml-0.5 inline-block h-4 w-px animate-pulse bg-ink-strong align-[-3px]" />}
          </>
        ) : (
          <div className="pt-1">
            {changes.map((c, i) => (
              <DiffFile
                key={msg.id + c.path}
                change={c}
                include={current && !!include[c.path]}
                onToggleInclude={() => current && toggleInclude(c.path)}
                mode={diffMode}
                defaultCollapsed={i > 0}
              />
            ))}
          </div>
        )}
      </div>
      {changes.length > 0 && (
        <div className="mt-3 ml-[30px] flex items-center gap-2 rounded-md border border-border bg-surface-200 py-1.5 pr-1.5 pl-3 text-ui-body">
          <Icon name="files" className="text-ink-muted" />
          <b className="font-semibold">
            {changes.length} {changes.length === 1 ? 'file' : 'files'} changed
          </b>
          <span className="font-mono text-[12px] font-semibold text-success">+{sum.adds}</span>
          <span className="font-mono text-[12px] font-semibold text-danger">−{sum.dels}</span>
          <span className="flex-1" />
          {current ? (
            <Button size="xs" icon="split" onClick={() => openDiff()}>
              Review diff
            </Button>
          ) : (
            <span className="pr-1.5 text-ui-small text-ink-muted">Superseded by a later response</span>
          )}
        </div>
      )}
    </div>
  )
}

function NoteMessage({ msg }: { msg: ChatMessage }) {
  const error = msg.tone === 'error'
  return (
    <div role="status" className="mb-6 ml-[30px] flex items-start gap-2 text-ui-small text-ink-muted">
      <Icon name="warning" className={error ? 'mt-px size-3.5 text-danger' : 'mt-px size-3.5 text-warning'} />
      <span className={error ? 'text-ink' : undefined}>{msg.text}</span>
    </div>
  )
}

export function ChatThread() {
  const { messages, totals, project } = useApp()
  const end = useRef<HTMLDivElement>(null)
  // Messages present when the thread opened (the sample conversation) don't trigger scrolling.
  const [initialCount] = useState(messages.length)
  const last = messages[messages.length - 1]
  const follow = messages.length > initialCount ? `${messages.length}:${last?.text.length ?? 0}` : ''

  // Keep the newest message in view while it streams.
  useEffect(() => {
    if (follow) end.current?.scrollIntoView({ block: 'end' })
  }, [follow])

  if (!messages.length)
    return (
      <div className="flex min-h-0 flex-1 items-center justify-center px-6">
        <div className="max-w-[420px] text-center text-ui-prose text-ink-muted">
          <Icon name="sparkle" className="mx-auto mb-3 size-6 text-accent-ink" />
          <p className="mb-2 text-ink">Ask for a change in {project?.name}.</p>
          <p className="text-ui-body">
            {totals.files
              ? `${totals.files} ${totals.files === 1 ? 'file is' : 'files are'} ticked and will be sent with your message.`
              : 'Tick the files the model should see in the explorer, then describe the change below.'}{' '}
            The model only proposes edits; nothing is written until you apply.
          </p>
        </div>
      </div>
    )

  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <div className="mx-auto max-w-[820px] px-6 pt-4 pb-6">
        <div className="mb-3 text-center text-[11px] text-ink-muted">Today</div>
        {messages.map((m) =>
          m.role === 'user' ? (
            <UserMessage key={m.id} msg={m} />
          ) : m.role === 'assistant' ? (
            <AssistantMessage key={m.id} msg={m} />
          ) : (
            <NoteMessage key={m.id} msg={m} />
          ),
        )}
        <div ref={end} />
      </div>
    </div>
  )
}
