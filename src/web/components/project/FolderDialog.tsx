import { useCallback, useEffect, useRef, useState } from 'react'
import { Button, IconButton } from '@/components/ui/Button'
import { CheckLabel } from '@/components/ui/Checkbox'
import { FolderGlyph } from '@/components/ui/FileGlyph'
import { Icon } from '@/components/ui/Icon'
import { Switch } from '@/components/ui/Switch'
import { Tag } from '@/components/ui/Tag'
import { listDirs, type DirListing } from '@/lib/api'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

/**
 * Folder picker backed by the local server, so it browses the machine the server runs on
 * (WSL paths, and Windows drives under /mnt/c). Accepts typed paths like C:\src\app too.
 */
export function FolderDialog() {
  const { closeFolderDialog, openFolder, lastFolder } = useApp()
  const [listing, setListing] = useState<DirListing | null>(null)
  const [input, setInput] = useState('')
  const [hidden, setHidden] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [opening, setOpening] = useState(false)
  const [selected, setSelected] = useState<string | null>(null)
  const listRef = useRef<HTMLDivElement>(null)

  const go = useCallback(
    (path: string | null, showHidden = hidden) => {
      setLoading(true)
      setError(null)
      listDirs(path, showHidden)
        .then((l) => {
          setListing(l)
          setInput(l.path)
          setSelected(null)
          listRef.current?.scrollTo({ top: 0 })
        })
        .catch((err: unknown) => setError(err instanceof Error ? err.message : 'Could not read folder.'))
        .finally(() => setLoading(false))
    },
    [hidden],
  )

  // Start in the folder of the last opened project, or the home folder.
  const [start] = useState(lastFolder)
  const started = useRef(false)
  useEffect(() => {
    if (started.current) return
    started.current = true
    go(start)
  }, [go, start])

  const toggleHidden = () => {
    const next = !hidden
    setHidden(next)
    go(listing?.path ?? start, next)
  }

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === 'Escape' && closeFolderDialog()
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [closeFolderDialog])

  const target = selected ?? listing?.path ?? null
  const open = () => {
    if (!target) return
    setOpening(true)
    setError(null)
    void openFolder(target).then((err) => {
      setOpening(false)
      if (err) setError(err)
    })
  }

  const crumbs = listing ? listing.path.split('/').filter(Boolean) : []

  return (
    <div className="absolute inset-0 z-10 flex items-center justify-center bg-surface-scrim" onClick={closeFolderDialog}>
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="folder-title"
        onClick={(e) => e.stopPropagation()}
        className="flex h-[560px] w-[680px] flex-col overflow-hidden rounded-lg border border-border-strong bg-surface-100 shadow-modal"
      >
        <div className="flex items-center pt-3 pr-3 pb-2 pl-4">
          <h2 id="folder-title" className="text-ui-title text-ink-strong">
            Open folder
          </h2>
          <span className="flex-1" />
          <IconButton icon="close" label="Close" onClick={closeFolderDialog} />
        </div>

        <form
          className="flex gap-2 px-4"
          onSubmit={(e) => {
            e.preventDefault()
            go(input)
          }}
        >
          <label className="flex h-control flex-1 items-center gap-1.5 rounded-md border border-border-strong bg-surface-100 px-2 focus-within:border-focus-ring">
            <Icon name="folder" className="size-3.5 text-ink-muted" />
            <input
              aria-label="Folder path"
              value={input}
              spellCheck={false}
              onChange={(e) => setInput(e.target.value)}
              placeholder="/home/you/project or C:\src\project"
              className="w-full bg-transparent font-mono text-ui-small text-ink outline-none placeholder:text-ink-muted"
            />
          </label>
          <Button type="submit">Go</Button>
        </form>

        <div className="flex items-center gap-1 px-4 pt-2 text-ui-small">
          {listing?.places.map((p) => (
            <button
              key={p.path}
              type="button"
              onClick={() => go(p.path)}
              className={cn(
                'inline-flex h-row cursor-pointer items-center rounded-sm px-2 text-ink-muted hover:bg-surface-hover hover:text-ink',
                listing.path === p.path && 'bg-surface-300 text-ink',
              )}
            >
              {p.label}
            </button>
          ))}
          <span className="flex-1" />
          <CheckLabel className="text-ui-small" onToggle={toggleHidden} control={<Switch on={hidden} onChange={toggleHidden} />}>
            Show hidden
          </CheckLabel>
        </div>

        <div className="flex h-7 items-center gap-1 border-b border-border px-4 font-mono text-[12px] text-ink-muted">
          <IconButton icon="back" label="Parent folder" disabled={!listing?.parent} onClick={() => listing?.parent && go(listing.parent)} />
          <button type="button" className="cursor-pointer hover:text-ink" onClick={() => go('/')}>
            /
          </button>
          {crumbs.map((c, i) => (
            <span key={i} className="inline-flex items-center gap-1">
              {i > 0 && <span>/</span>}
              <button
                type="button"
                className={cn('cursor-pointer hover:text-ink', i === crumbs.length - 1 && 'text-ink')}
                onClick={() => go('/' + crumbs.slice(0, i + 1).join('/'))}
              >
                {c}
              </button>
            </span>
          ))}
        </div>

        <div ref={listRef} role="listbox" aria-label="Folders" className={cn('min-h-0 flex-1 overflow-auto py-1', loading && 'opacity-60')}>
          {listing?.dirs.map((d) => (
            <div
              key={d.path}
              role="option"
              aria-selected={selected === d.path}
              onClick={() => setSelected(d.path)}
              onDoubleClick={() => go(d.path)}
              className={cn(
                'flex h-7 cursor-pointer items-center gap-2 px-4 text-ui-body hover:bg-surface-hover',
                selected === d.path && 'bg-surface-selected hover:bg-surface-selected',
              )}
            >
              <FolderGlyph open={false} />
              <span className="truncate">{d.name}</span>
              {d.project && <Tag tone="skill">project</Tag>}
              <span className="flex-1" />
              <button
                type="button"
                aria-label={`Open ${d.name}`}
                onClick={(e) => {
                  e.stopPropagation()
                  go(d.path)
                }}
                className="inline-flex cursor-pointer rounded-sm text-ink-muted hover:bg-surface-hover hover:text-ink"
              >
                <Icon name="chevronRight" className="size-3.5" />
              </button>
            </div>
          ))}
          {listing && listing.dirs.length === 0 && <p className="px-4 py-2 text-ui-small text-ink-muted">No sub-folders.</p>}
          {listing?.truncated && <p className="px-4 py-2 text-ui-small text-ink-muted">Only the first 500 folders are listed. Type a path above.</p>}
        </div>

        {error && (
          <p role="alert" className="flex items-start gap-1.5 border-t border-border px-4 py-2 text-ui-small text-danger">
            <Icon name="warning" className="mt-px size-3.5" />
            {error}
          </p>
        )}

        <div className="flex items-center gap-2 border-t border-border bg-surface-000 px-4 py-3">
          <span className="min-w-0 truncate font-mono text-ui-small text-ink-muted" title={target ?? ''}>
            {target}
          </span>
          <span className="flex-1" />
          <Button onClick={closeFolderDialog}>Cancel</Button>
          <Button variant="primary" disabled={!target || opening} onClick={open}>
            {opening ? 'Opening…' : selected ? 'Open selected folder' : 'Open this folder'}
          </Button>
        </div>
      </div>
    </div>
  )
}
