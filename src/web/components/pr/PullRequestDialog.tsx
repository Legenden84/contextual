import { useEffect, useMemo, useRef, useState } from 'react'
import { DiffFile } from '@/components/diff/DiffFile'
import { Button, IconButton, LinkButton } from '@/components/ui/Button'
import { Checkbox, CheckLabel } from '@/components/ui/Checkbox'
import { Icon } from '@/components/ui/Icon'
import { StatBar } from '@/components/ui/Status'
import { TextInput } from '@/components/ui/TextInput'
import type { Change } from '@/data/types'
import * as api from '@/lib/api'
import { computeDiff } from '@/lib/diff'
import { contentText } from '@/lib/prompt'
import { useApp } from '@/state/app-context'

/** Branch name suggested from the first commit or chat request, e.g. "contextual/add-order-cancellation". */
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

const asChange = (f: api.Comparison['files'][number]): Change => ({
  path: f.path,
  status: f.letter === 'A' ? 'A' : f.letter === 'D' ? 'D' : 'M',
  old: f.old,
  new: f.new,
})

/**
 * Open a pull request, like GitHub's compare view: the commits on this branch that the base
 * doesn't have, and their file diffs. Nothing to compare → says so. Committing happens in
 * Source Control; this dialog only pushes and opens (or updates) the PR.
 */
export function PullRequestDialog() {
  const { project, thread, messages, closePr, recordPr, foundry, setSidebarTab, diffMode } = useApp()
  const [cmp, setCmp] = useState<api.Comparison | null>(null)
  const [loading, setLoading] = useState(true)
  const [loadError, setLoadError] = useState<string | null>(null)
  const [base, setBase] = useState('')
  const [title, setTitle] = useState('')
  const [body, setBody] = useState('')
  const [newBranch, setNewBranch] = useState('')
  const [draft, setDraft] = useState(false)
  const [drafting, setDrafting] = useState(false)
  const [draftError, setDraftError] = useState<string | null>(null)
  const [creating, setCreating] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const root = project?.root ?? ''

  // What the chat asked and the model explained (without code), to inform the PR description.
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
  const firstAsk = messages.find((m) => m.role === 'user')?.text ?? ''

  const draftFor = (baseName: string) => {
    setDrafting(true)
    setDraftError(null)
    api
      .draftPr(root, baseName, notes)
      .then((d) => {
        setTitle(d.title)
        setBody(d.body)
      })
      .catch((err: unknown) => setDraftError(err instanceof Error ? err.message : 'Could not draft the text.'))
      .finally(() => setDrafting(false))
  }
  const runDraft = () => cmp && draftFor(cmp.base)
  // Draft the text once per opening, as soon as there is something to describe.
  const autoDrafted = useRef(false)
  const canDraft = foundry.state === 'ready'

  // Compare once on open and whenever the base changes (debounced while typing).
  const [baseKey, setBaseKey] = useState<string | undefined>(undefined)
  useEffect(() => {
    const req = { live: true }
    api
      .comparePr(root, baseKey)
      .then((c) => {
        if (!req.live) return
        setCmp(c)
        setBase((b) => b || c.base)
        const first = c.commits[c.commits.length - 1]?.subject ?? firstAsk
        if (c.branch && c.branch === c.defaultBranch) setNewBranch((n) => n || suggestBranch(first))
        // Single commit: its subject is a good title (GitHub does the same).
        setTitle((t) => t || (c.commits.length === 1 ? c.commits[0].subject : first.split('\n')[0].slice(0, 72)))
        if (!autoDrafted.current && canDraft && c.commits.length && !c.existingPr) {
          autoDrafted.current = true
          draftFor(c.base)
        }
      })
      .catch((err: unknown) => req.live && setLoadError(err instanceof Error ? err.message : 'Could not compare the branches.'))
      .finally(() => req.live && setLoading(false))
    return () => {
      req.live = false
    }
    // draftFor only reads stable inputs (root, notes); comparing again shouldn't depend on it.
  }, [root, baseKey, firstAsk, canDraft]) // eslint-disable-line react-hooks/exhaustive-deps

  useEffect(() => {
    if (!base || base === (baseKey ?? cmp?.base)) return
    const t = window.setTimeout(() => {
      setLoading(true)
      setLoadError(null)
      setBaseKey(base)
    }, 600)
    return () => window.clearTimeout(t)
  }, [base, baseKey, cmp?.base])

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && !creating && closePr()
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [closePr, creating])

  const files = useMemo(() => cmp?.files ?? [], [cmp])
  const totals = useMemo(
    () =>
      files.reduce(
        (a, f) => {
          const d = computeDiff(f.old, f.new)
          return { adds: a.adds + d.adds, dels: a.dels + d.dels }
        },
        { adds: 0, dels: 0 },
      ),
    [files],
  )

  const onDefault = !!cmp?.branch && cmp.branch === cmp.defaultBranch
  const existing = !onDefault ? cmp?.existingPr ?? null : null
  const nothing = !!cmp && !cmp.problem && cmp.commits.length === 0
  const ready = !!cmp && cmp.github && cmp.commits.length > 0
  const canCreate =
    ready && !creating && !loading && !!base.trim() && (existing ? cmp.unpushed > 0 : !!title.trim()) && (!onDefault || !!newBranch.trim())

  const create = () => {
    if (!cmp) return
    setCreating(true)
    setError(null)
    api
      .createPr(root, { title, body, base: cmp.base, draft, newBranch: onDefault ? newBranch.trim() : undefined })
      .then((r) => recordPr(r))
      .catch((err: unknown) => setError(err instanceof Error ? err.message : 'Could not create the pull request.'))
      .finally(() => setCreating(false))
  }

  const openSourceControl = () => {
    setSidebarTab('scm')
    closePr()
  }

  return (
    <div className="absolute inset-0 z-10 flex items-center justify-center bg-surface-scrim" onClick={() => !creating && closePr()}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="pr-title"
        onClick={(e) => e.stopPropagation()}
        className="flex h-[90%] w-[960px] max-w-[95%] flex-col overflow-hidden rounded-lg border border-border-strong bg-surface-100 shadow-modal"
      >
        <div className="flex items-center pt-3 pr-3 pb-2 pl-4">
          <h2 id="pr-title" className="text-ui-title text-ink-strong">
            {existing ? `Pull request #${existing.number}` : 'Open a pull request'}
          </h2>
          <span className="flex-1" />
          <IconButton icon="close" label="Close" onClick={closePr} disabled={creating} />
        </div>

        {/* base ← compare, as on GitHub */}
        <div className="flex flex-wrap items-center gap-2 border-b border-border bg-surface-000 px-4 py-2 text-ui-small text-ink-muted">
          <Icon name="branch" className="size-3.5" />
          <label className="inline-flex items-center gap-1.5">
            base:
            <TextInput mono size="sm" boxClassName="w-[150px]" aria-label="Base branch" value={base} onChange={(e) => setBase(e.target.value)} />
          </label>
          <span aria-hidden>←</span>
          {onDefault ? (
            <label className="inline-flex items-center gap-1.5">
              new branch:
              <TextInput mono size="sm" boxClassName="w-[300px]" aria-label="New branch name" value={newBranch} onChange={(e) => setNewBranch(e.target.value)} />
            </label>
          ) : (
            <span>
              compare: <span className="font-mono text-ink">{cmp?.branch ?? '…'}</span>
            </span>
          )}
          <span className="flex-1" />
          {loading && <span>Comparing…</span>}
          {cmp && !loading && cmp.commits.length > 0 && (
            <span className="inline-flex items-center gap-3">
              <span>
                {cmp.commits.length} {cmp.commits.length === 1 ? 'commit' : 'commits'} · {files.length} {files.length === 1 ? 'file' : 'files'}
              </span>
              <StatBar adds={totals.adds} dels={totals.dels} />
            </span>
          )}
        </div>

        <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-auto px-4 py-3">
          {loadError && <p className="text-danger">{loadError}</p>}
          {cmp?.problem && (
            <p className="flex items-start gap-1.5 rounded-md border border-warning-border bg-warning-bg px-3 py-2 text-ui-small text-ink">
              <Icon name="warning" className="mt-px size-3.5 text-warning" />
              {cmp.problem}
            </p>
          )}

          {onDefault && ready && (
            <p className="text-ui-small text-ink-muted">
              You're on <span className="font-mono text-ink">{cmp.branch}</span>, the default branch. Your {cmp.commits.length}{' '}
              {cmp.commits.length === 1 ? 'commit moves' : 'commits move'} to the new branch, which is pushed and used for the pull request.
            </p>
          )}

          {nothing && (
            <div className="rounded-md border border-border bg-surface-200 px-6 py-8 text-center">
              <p className="mb-1 text-ui-title text-ink-strong">There isn't anything to compare.</p>
              <p className="text-ink-muted">
                <span className="font-mono">{cmp.base}</span> is up to date with all commits from{' '}
                <span className="font-mono">{cmp.branch}</span>.
              </p>
              {cmp.uncommitted > 0 && (
                <p className="mt-3 text-ui-small text-ink-muted">
                  You have {cmp.uncommitted} uncommitted {cmp.uncommitted === 1 ? 'change' : 'changes'}. Commit them first, then open the
                  pull request.{' '}
                  <LinkButton onClick={openSourceControl} className="inline-flex">
                    Open Source Control
                  </LinkButton>
                </p>
              )}
            </div>
          )}

          {ready && existing && (
            <p className="flex items-center gap-1.5 text-ui-small text-ink">
              <Icon name="check" className="size-3.5 text-success" />
              This branch already has an open pull request:{' '}
              <a href={existing.url} target="_blank" rel="noreferrer" className="text-accent-ink hover:underline">
                #{existing.number}
              </a>
              . {cmp.unpushed > 0 ? `Push ${cmp.unpushed} new ${cmp.unpushed === 1 ? 'commit' : 'commits'} to update it.` : 'It is up to date.'}
            </p>
          )}

          {ready && !existing && (
            <>
              <label className="flex flex-col gap-1">
                <span className="flex items-center gap-2 text-ui-small font-medium text-ink">
                  Title
                  <span className="flex-1" />
                  {foundry.state === 'ready' && (
                    <LinkButton disabled={drafting} onClick={runDraft}>
                      <Icon name="sparkle" />
                      {drafting ? 'Drafting…' : 'Draft with AI'}
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
                  rows={7}
                  placeholder={drafting ? 'Drafting from the commits, the diff and the chat…' : 'Markdown'}
                  className="w-full resize-y rounded-md border border-border-strong bg-surface-100 px-2 py-1.5 font-mono text-[12px] leading-[18px] text-ink outline-none focus:border-focus-ring disabled:opacity-60"
                />
              </label>
              {draftError && <p className="text-ui-small text-warning">Couldn't draft the text: {draftError}</p>}
            </>
          )}

          {ready && cmp.uncommitted > 0 && (
            <p className="text-ui-small text-ink-muted">
              {cmp.uncommitted} uncommitted {cmp.uncommitted === 1 ? 'change is' : 'changes are'} not part of this pull request.{' '}
              <LinkButton onClick={openSourceControl} className="inline-flex">
                Commit in Source Control
              </LinkButton>
            </p>
          )}

          {ready && (
            <>
              <div>
                <div className="mb-1 text-ui-caption text-ink-muted uppercase">Commits</div>
                <div className="overflow-hidden rounded-md border border-border">
                  {cmp.commits.map((c) => (
                    <div key={c.sha} className="flex h-7 items-center gap-3 border-b border-border px-3 text-ui-small last:border-b-0">
                      <span className="font-mono text-accent-ink">{c.sha}</span>
                      <span className="min-w-0 flex-1 truncate text-ink">{c.subject}</span>
                      <span className="flex-none text-ink-muted">
                        {c.author} · {c.when}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
              <div>
                <div className="mb-1 text-ui-caption text-ink-muted uppercase">Files changed</div>
                {files.map((f, i) =>
                  f.skipped ? (
                    <p key={f.path} className="mb-3 rounded-md border border-border px-3 py-2 font-mono text-[12px] text-ink-muted">
                      {f.path} · not shown ({f.skipped})
                    </p>
                  ) : (
                    <DiffFile key={f.path} change={asChange(f)} mode={diffMode} defaultCollapsed={files.length > 3 && i > 0} />
                  ),
                )}
              </div>
            </>
          )}

          {error && (
            <p role="alert" className="flex items-start gap-1.5 text-ui-small text-danger">
              <Icon name="warning" className="mt-px size-3.5 flex-none" />
              {error}
            </p>
          )}
        </div>

        <div className="flex items-center gap-3 border-t border-border bg-surface-000 px-4 py-3">
          {ready && !existing && (
            <CheckLabel className="text-ui-small" onToggle={() => setDraft((d) => !d)} control={<Checkbox state={draft} onChange={() => setDraft((d) => !d)} />}>
              Open as draft
            </CheckLabel>
          )}
          {cmp?.remote && (
            <span title={cmp.remote} className="min-w-0 truncate text-ui-small text-ink-muted">
              Pushes to <span className="font-mono">{cmp.remote}</span>
            </span>
          )}
          <span className="flex-1" />
          <Button onClick={closePr} disabled={creating}>
            {nothing || !ready ? 'Close' : 'Cancel'}
          </Button>
          {ready && (
            <Button variant="primary" disabled={!canCreate} onClick={create}>
              {creating ? 'Working…' : existing ? (cmp.unpushed > 0 ? `Push to #${existing.number}` : 'Up to date') : draft ? 'Create draft pull request' : 'Create pull request'}
            </Button>
          )}
        </div>
      </div>
    </div>
  )
}
