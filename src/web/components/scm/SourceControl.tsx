import { useState } from 'react'
import { Button, IconButton } from '@/components/ui/Button'
import { FileGlyph } from '@/components/ui/FileGlyph'
import { Icon } from '@/components/ui/Icon'
import { Count } from '@/components/ui/Tag'
import * as api from '@/lib/api'
import { splitPath } from '@/lib/format'
import { gitTabId } from '@/lib/git-tabs'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

const LETTER_TONE: Record<api.ChangeLetter, string> = {
  M: 'text-modified',
  R: 'text-modified',
  A: 'text-success',
  U: 'text-success',
  D: 'text-danger',
  C: 'text-danger',
}
const LETTER_TITLE: Record<api.ChangeLetter, string> = {
  M: 'Modified',
  R: 'Renamed',
  A: 'Added',
  U: 'Untracked',
  D: 'Deleted',
  C: 'Conflict',
}

function Row({
  entry,
  staged,
  busy,
  onAction,
}: {
  entry: api.StatusEntry
  staged: boolean
  busy: boolean
  onAction: () => void
}) {
  const { openGitDiff, activeTab } = useApp()
  const { dir, name } = splitPath(entry.path)
  const tab = gitTabId(entry.path, staged)
  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => openGitDiff(entry.path, staged)}
      onKeyDown={(e) => e.key === 'Enter' && openGitDiff(entry.path, staged)}
      title={`${entry.path} · ${LETTER_TITLE[entry.letter]}${staged ? ' (staged)' : ''}`}
      className={cn(
        'group flex h-row cursor-pointer items-center gap-1.5 pr-2 pl-6 text-ui-body hover:bg-surface-hover',
        activeTab === tab && 'bg-surface-selected hover:bg-surface-selected',
      )}
    >
      <FileGlyph name={name} />
      <span className={cn('truncate', entry.letter === 'D' && 'line-through decoration-ink-muted')}>{name}</span>
      <span className="min-w-0 flex-1 truncate text-[11px] text-ink-muted">{dir.replace(/\/$/, '')}</span>
      <IconButton
        icon={staged ? 'minus' : 'plus'}
        label={staged ? `Unstage ${entry.path}` : `Stage ${entry.path}`}
        disabled={busy}
        onClick={(e) => {
          e.stopPropagation()
          onAction()
        }}
        className="hidden group-hover:inline-flex focus-visible:inline-flex"
      />
      <span className={cn('w-3 flex-none text-center font-mono text-[11px] font-semibold', LETTER_TONE[entry.letter])}>{entry.letter}</span>
    </div>
  )
}

function Section({
  title,
  entries,
  staged,
  busy,
  onAll,
  onOne,
}: {
  title: string
  entries: api.StatusEntry[]
  staged: boolean
  busy: boolean
  onAll: () => void
  onOne: (path: string) => void
}) {
  const [open, setOpen] = useState(true)
  if (!entries.length) return null
  return (
    <div>
      <div className="group flex h-row items-center gap-1 pr-2 pl-1.5 text-ui-caption text-ink-muted uppercase">
        <button type="button" className="inline-flex flex-1 cursor-pointer items-center gap-1" onClick={() => setOpen((o) => !o)}>
          <Icon name={open ? 'chevronDown' : 'chevronRight'} className="size-3.5" />
          {title}
        </button>
        <IconButton
          icon={staged ? 'minus' : 'plus'}
          label={staged ? 'Unstage all' : 'Stage all'}
          disabled={busy}
          onClick={onAll}
          className="hidden group-hover:inline-flex focus-visible:inline-flex"
        />
        <Count>{entries.length}</Count>
      </div>
      {open && entries.map((e) => <Row key={`${staged}:${e.path}`} entry={e} staged={staged} busy={busy} onAction={() => onOne(e.path)} />)}
    </div>
  )
}

/** VS Code-style Source Control: stage, unstage, write a message, commit, push. */
export function SourceControl() {
  const { project, git, gitChanged, foundry } = useApp()
  const [message, setMessage] = useState('')
  const [busy, setBusy] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [done, setDone] = useState<string | null>(null)
  const root = project?.root ?? ''

  const act = (what: string, fn: () => Promise<unknown>) => {
    setBusy(what)
    setError(null)
    setDone(null)
    fn()
      .then(() => gitChanged())
      .catch((err: unknown) => setError(err instanceof Error ? err.message : `${what} failed.`))
      .finally(() => setBusy(null))
  }

  if (!project) return null
  if (!git)
    return (
      <div className="flex min-h-0 flex-1 flex-col">
        <div className="flex h-8 flex-none items-center pr-2 pl-3">
          <span className="text-ui-caption text-ink-muted uppercase">Source control</span>
        </div>
        <p className="px-3 text-ui-small text-ink-muted">This folder is not a git repository.</p>
      </div>
    )

  const staged = git.staged
  const changes = git.changes
  const all = !staged.length && changes.length > 0
  const canCommit = !!message.trim() && (staged.length > 0 || changes.length > 0) && !busy
  const commitLabel = all ? 'Commit all' : 'Commit'

  const doCommit = () =>
    act('Commit', async () => {
      const r = await api.gitCommit(root, message, all)
      setMessage('')
      setDone(`Committed ${r.files} ${r.files === 1 ? 'file' : 'files'} as ${r.commit}.`)
    })

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-8 flex-none items-center gap-1.5 pr-2 pl-3">
        <span className="text-ui-caption text-ink-muted uppercase">Source control</span>
        <span className="flex-1" />
        <IconButton icon="refresh" label="Refresh" onClick={gitChanged} />
      </div>

      <div className="flex flex-none flex-col gap-2 px-3 pb-2">
        <div className="flex items-center gap-1.5 text-ui-small text-ink-muted">
          <Icon name="branch" className="size-3.5" />
          <span className="truncate font-mono text-ink">{git.branch ?? '(detached)'}</span>
          {git.upstream && (git.ahead > 0 || git.behind > 0) && (
            <span className="font-mono" title={`${git.ahead} to push, ${git.behind} to pull (${git.upstream})`}>
              {git.ahead > 0 && `↑${git.ahead}`} {git.behind > 0 && `↓${git.behind}`}
            </span>
          )}
          {!git.upstream && git.branch && <span title="This branch is not on the remote yet">· not published</span>}
        </div>

        <div className="relative rounded-md border border-border-strong bg-surface-100 focus-within:border-focus-ring">
          <textarea
            aria-label="Commit message"
            value={message}
            rows={3}
            placeholder={`Message (Ctrl+Enter to commit on ${git.branch ?? 'HEAD'})`}
            onChange={(e) => setMessage(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && (e.ctrlKey || e.metaKey) && canCommit) {
                e.preventDefault()
                doCommit()
              }
            }}
            className="block w-full resize-y bg-transparent py-1.5 pr-7 pl-2 text-ui-body text-ink outline-none placeholder:text-ink-muted"
          />
          {foundry.state === 'ready' && (
            <IconButton
              icon="sparkle"
              label="Draft the message with AI"
              disabled={!!busy || (!staged.length && !changes.length)}
              onClick={() =>
                act('Drafting', async () => {
                  const r = await api.draftCommit(root)
                  setMessage(r.message)
                })
              }
              className="absolute top-1 right-1"
            />
          )}
        </div>

        <Button variant="primary" icon="check" disabled={!canCommit} onClick={doCommit} className="w-full">
          {busy === 'Commit' ? 'Committing…' : busy === 'Drafting' ? 'Drafting…' : commitLabel}
        </Button>
        {all && !busy && <p className="-mt-1 text-[11px] text-ink-muted">Nothing is staged, so all changes will be committed.</p>}

        {git.branch && (git.ahead > 0 || !git.upstream) && (
          <Button icon="cloud" disabled={!!busy} onClick={() => act('Push', () => api.gitPush(root))} className="w-full">
            {busy === 'Push' ? 'Pushing…' : git.upstream ? `Push ${git.ahead} ${git.ahead === 1 ? 'commit' : 'commits'}` : 'Publish branch'}
          </Button>
        )}

        {error && (
          <p role="alert" className="flex items-start gap-1.5 text-ui-small text-danger">
            <Icon name="warning" className="mt-px size-3.5 flex-none" />
            {error}
          </p>
        )}
        {done && !error && (
          <p role="status" className="flex items-start gap-1.5 text-ui-small text-success">
            <Icon name="check" className="mt-px size-3.5 flex-none" />
            {done}
          </p>
        )}
      </div>

      <div className="min-h-0 flex-1 overflow-auto border-t border-border pt-1 pb-2">
        {git.conflicts.length > 0 && (
          <p className="mx-3 my-1 rounded-sm bg-warning-bg px-2 py-1 text-ui-small text-ink">
            {git.conflicts.length} merge {git.conflicts.length === 1 ? 'conflict' : 'conflicts'}: resolve in your editor first.
          </p>
        )}
        <Section
          title="Staged changes"
          entries={staged}
          staged
          busy={!!busy}
          onAll={() => act('Unstage', () => api.gitUnstage(root, staged.map((e) => e.path)))}
          onOne={(p) => act('Unstage', () => api.gitUnstage(root, [p]))}
        />
        <Section
          title="Changes"
          entries={changes}
          staged={false}
          busy={!!busy}
          onAll={() => act('Stage', () => api.gitStage(root, changes.map((e) => e.path)))}
          onOne={(p) => act('Stage', () => api.gitStage(root, [p]))}
        />
        {!staged.length && !changes.length && !git.conflicts.length && (
          <p className="px-3 py-2 text-ui-small text-ink-muted">No changes. The working tree matches the last commit.</p>
        )}
      </div>
    </div>
  )
}
