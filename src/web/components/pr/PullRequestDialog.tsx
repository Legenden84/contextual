import { useCallback, useEffect, useMemo, useState } from 'react'
import { Button, IconButton, LinkButton } from '@/components/ui/Button'
import { Checkbox, CheckLabel } from '@/components/ui/Checkbox'
import { Icon } from '@/components/ui/Icon'
import { StatusLetter } from '@/components/ui/Status'
import { TextInput } from '@/components/ui/TextInput'
import * as api from '@/lib/api'
import { contentText } from '@/lib/prompt'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

/** Branch name suggested from the chat's first request, e.g. "contextual/add-order-cancellation". */
function suggestBranch(text: string): string {
  const slug = text
    .toLowerCase()
    .replace(/`[^`]*`/g, ' ')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-+|-+$/g, '')
    .split('-')
    .slice(0, 6)
    .join('-')
  return `contextual/${slug || 'changes'}`
}

/**
 * Create a pull request from the files this chat applied: commit them on the current branch
 * (or a new one when on the default branch), push, and open the PR with gh.
 */
export function PullRequestDialog() {
  const { project, thread, messages, closePr, recordPr, foundry } = useApp()
  const [info, setInfo] = useState<api.GitInfo | null>(null)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set())
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [base, setBase] = useState('')
  const [newBranch, setNewBranch] = useState('')
  const [draft, setDraft] = useState(false)
  const [drafting, setDrafting] = useState(false)
  const [draftError, setDraftError] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [error, setError] = useState<string | null>(null)

  const root = project?.root ?? ''
  const toggle = (path: string) =>
    setPicked((s) => {
      const next = new Set(s)
      if (next.has(path)) next.delete(path)
      else next.add(path)
      return next
    })
  const firstAsk = messages.find((m) => m.role === 'user')?.text ?? ''

  // What the chat asked and the model explained (without code), for the PR description.
  const notes = useMemo(
    () =>
      thread.transcript
        .map((m) => {
          const text = contentText(m.content).replace(/```[\s\S]*?```/g, '[code]').slice(0, 4000)
          return `${m.role === 'user' ? 'Request' : 'Reply'}: ${m.role === 'user' ? text.split('# Request').pop()!.trim() : text}`
        })
        .join('\n\n'),
    [thread.transcript],
  )

  const draftText = useCallback(
    (paths: string[]) => {
      if (foundry.state !== 'ready' || !paths.length) return
      setDrafting(true)
      setDraftError(null)
      api
        .draftPr(root, paths, notes)
        .then((d) => {
          setTitle(d.title)
          setBody(d.body)
        })
        .catch((err: unknown) => setDraftError(err instanceof Error ? err.message : 'Could not draft the text.'))
        .finally(() => setDrafting(false))
    },
    [root, notes, foundry.state],
  )

  // Read the repository once per opening; later edits are the user's.
  const [start] = useState(() => ({ root, applied: thread.applied, firstAsk, draftText }))
  useEffect(() => {
    let live = true
    const { root, applied, firstAsk, draftText } = start
    api
      .gitInfo(root, applied)
      .then((i) => {
        if (!live) return
        setInfo(i)
        const changed = i.files.filter((f) => f.status).map((f) => f.path)
        setPicked(new Set(changed))
        setBase(i.defaultBranch ?? 'main')
        if (i.branch && i.branch === i.defaultBranch) setNewBranch(suggestBranch(firstAsk))
        setTitle(firstAsk.split('\n')[0].replace(/`/g, '').slice(0, 72))
        draftText(changed)
      })
      .catch((err: unknown) => live && setLoadError(err instanceof Error ? err.message : 'Could not read the repository.'))
    return () => {
      live = false
    }
  }, [start])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && !creating && closePr()
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [closePr, creating])

  const onDefault = !!info?.branch && info.branch === info.defaultBranch
  const files = info?.files ?? []
  const changedFiles = files.filter((f) => f.status)
  const paths = changedFiles.filter((f) => picked.has(f.path)).map((f) => f.path)
  const blocked = !info?.isRepo || !info.github
  const canCreate = !blocked && paths.length > 0 && !!title.trim() && !!base.trim() && (!onDefault || !!newBranch.trim()) && !creating
  const existing = info?.existingPr && !onDefault

  const create = () => {
    setCreating(true)
    setError(null)
    api
      .createPr(root, { paths, title, body, base, draft, newBranch: onDefault ? newBranch.trim() : undefined })
      .then((r) => recordPr(r, paths))
      .catch((err: unknown) => setError(err instanceof Error ? err.message : 'Could not create the pull request.'))
      .finally(() => setCreating(false))
  }

  return (
    <div className="absolute inset-0 z-10 flex items-center justify-center bg-surface-scrim" onClick={() => !creating && closePr()}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="pr-title"
        onClick={(e) => e.stopPropagation()}
        className="flex max-h-[90%] w-[680px] flex-col overflow-hidden rounded-lg border border-border-strong bg-surface-100 shadow-modal"
      >
        <div className="flex items-center pt-3 pr-3 pb-2 pl-4">
          <h2 id="pr-title" className="text-ui-title text-ink-strong">
            {existing ? 'Update pull request' : 'Create pull request'}
          </h2>
          <span className="flex-1" />
          <IconButton icon="close" label="Close" onClick={closePr} disabled={creating} />
        </div>

        <div className="flex min-h-0 flex-col gap-3 overflow-auto px-4 pb-4">
          {!info && !loadError && <p className="text-ink-muted">Reading the repository…</p>}
          {loadError && <p className="text-danger">{loadError}</p>}
          {info && blocked && (
            <p className="flex items-start gap-1.5 rounded-md border border-warning-border bg-warning-bg px-3 py-2 text-ui-small text-ink">
              <Icon name="warning" className="mt-px size-3.5 text-warning" />
              {info.problem}
            </p>
          )}

          {info && !blocked && (
            <>
              <div className="flex items-center gap-2 text-ui-small text-ink-muted">
                <Icon name="branch" className="size-3.5" />
                {onDefault ? (
                  <>
                    You're on <span className="font-mono text-ink">{info.branch}</span>, the default branch. The commit goes to a new
                    branch:
                  </>
                ) : (
                  <>
                    Commit on <span className="font-mono text-ink">{info.branch}</span>, push, and{' '}
                    {existing ? (
                      <>
                        update{' '}
                        <a href={info.existingPr!.url} target="_blank" rel="noreferrer" className="text-accent-ink hover:underline">
                          #{info.existingPr!.number}
                        </a>
                      </>
                    ) : (
                      <>
                        open a PR into <span className="font-mono text-ink">{base}</span>
                      </>
                    )}
                    .
                  </>
                )}
              </div>
              {onDefault && (
                <TextInput mono aria-label="New branch name" value={newBranch} onChange={(e) => setNewBranch(e.target.value)} />
              )}

              <div className="overflow-hidden rounded-md border border-border">
                {changedFiles.map((f) => (
                  <div
                    key={f.path}
                    onClick={() => toggle(f.path)}
                    className="flex h-[30px] cursor-pointer items-center gap-2 border-b border-border px-3 last:border-b-0 hover:bg-surface-hover"
                  >
                    <Checkbox state={picked.has(f.path)} label={`Include ${f.path}`} onChange={() => toggle(f.path)} />
                    {f.status && <StatusLetter status={f.status} />}
                    <span className="min-w-0 truncate font-mono text-[12px] text-ink [direction:rtl] text-left">{f.path}</span>
                  </div>
                ))}
                {!changedFiles.length && (
                  <p className="px-3 py-2 text-ui-small text-ink-muted">
                    No files applied in this chat have uncommitted changes{files.length ? ' (they were committed or undone)' : ''}.
                  </p>
                )}
              </div>
              <p className="-mt-1.5 text-ui-small text-ink-muted">
                Only these files are committed. Other changes in your working tree stay as they are.
              </p>

              <label className="flex flex-col gap-1">
                <span className="flex items-center gap-2 text-ui-small font-medium text-ink">
                  Title
                  <span className="flex-1" />
                  {foundry.state === 'ready' && (
                    <LinkButton disabled={drafting || !paths.length} onClick={() => draftText(paths)}>
                      <Icon name="sparkle" />
                      {drafting ? 'Drafting…' : 'Redraft with AI'}
                    </LinkButton>
                  )}
                </span>
                <TextInput value={title} disabled={drafting} onChange={(e) => setTitle(e.target.value)} />
              </label>
              <label className="flex flex-col gap-1">
                <span className="text-ui-small font-medium text-ink">Description</span>
                <textarea
                  value={body}
                  disabled={drafting}
                  onChange={(e) => setBody(e.target.value)}
                  rows={8}
                  placeholder={drafting ? 'Drafting from the chat and the diff…' : 'Markdown'}
                  className="w-full resize-y rounded-md border border-border-strong bg-surface-100 px-2 py-1.5 font-mono text-[12px] leading-[18px] text-ink outline-none focus:border-focus-ring disabled:opacity-60"
                />
              </label>
              {draftError && <p className="text-ui-small text-warning">Couldn't draft the text: {draftError}</p>}

              <div className="flex items-center gap-4">
                {!existing && (
                  <label className="flex items-center gap-2 text-ui-small text-ink">
                    Base
                    <TextInput mono size="sm" boxClassName="w-[160px]" value={base} onChange={(e) => setBase(e.target.value)} />
                  </label>
                )}
                {!existing && (
                  <CheckLabel className="text-ui-small" onToggle={() => setDraft((d) => !d)} control={<Checkbox state={draft} onChange={() => setDraft((d) => !d)} />}>
                    Open as draft
                  </CheckLabel>
                )}
              </div>
            </>
          )}

          {error && (
            <p role="alert" className="flex items-start gap-1.5 text-ui-small text-danger">
              <Icon name="warning" className="mt-px size-3.5" />
              {error}
            </p>
          )}
        </div>

        <div className="flex items-center gap-2 border-t border-border bg-surface-000 px-4 py-3">
          <span title={info?.remote ?? ''} className={cn('min-w-0 truncate text-ui-small text-ink-muted', !info?.remote && 'invisible')}>
            Pushes to <span className="font-mono">{info?.remote}</span>
          </span>
          <span className="flex-1" />
          <Button onClick={closePr} disabled={creating}>
            Cancel
          </Button>
          <Button variant="primary" disabled={!canCreate} onClick={create}>
            {creating ? 'Working…' : existing ? `Commit & push ${paths.length} ${paths.length === 1 ? 'file' : 'files'}` : draft ? 'Create draft PR' : 'Create pull request'}
          </Button>
        </div>
      </div>
    </div>
  )
}
