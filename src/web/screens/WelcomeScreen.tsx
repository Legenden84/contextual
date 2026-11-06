import { useEffect } from 'react'
import { Logo } from '@/components/shell/Logo'
import { Button, LinkButton } from '@/components/ui/Button'
import { Icon } from '@/components/ui/Icon'
import { FolderGlyph } from '@/components/ui/FileGlyph'
import { describeFoundry } from '@/lib/foundry-status'
import { useApp } from '@/state/app-context'

const STEPS = [
  'Tick the files the model should see',
  'Ask for a change in chat',
  'Review the diff, file by file',
  'Save what you want. The model never writes to disk.',
]

function ago(iso: string): string {
  const mins = Math.round((Date.now() - new Date(iso).getTime()) / 60000)
  if (!Number.isFinite(mins)) return ''
  if (mins < 1) return 'just now'
  if (mins < 60) return `${mins} min ago`
  const hours = Math.round(mins / 60)
  if (hours < 24) return `${hours} ${hours === 1 ? 'hour' : 'hours'} ago`
  const days = Math.round(hours / 24)
  if (days === 1) return 'Yesterday'
  if (days < 7) return `${days} days ago`
  return new Date(iso).toLocaleDateString('en-GB', { day: 'numeric', month: 'short' })
}

const ROW = 'flex w-full cursor-pointer items-center gap-3 border-b border-border px-3 py-2 text-left last:border-b-0 hover:bg-surface-hover'

/** Empty state: no project open. */
export function WelcomeScreen() {
  const { openFolderDialog, openFolder, openSettings, foundry, recents, clearRecents } = useApp()
  const label = describeFoundry(foundry)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'o') {
        e.preventDefault()
        openFolderDialog()
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [openFolderDialog])

  return (
    <main className="flex flex-1 items-center justify-center overflow-auto bg-surface-100 p-8">
      <div className="w-[860px]">
        <div className="mb-8 flex items-center gap-4">
          <Logo size={40} />
          <div>
            <h1 className="text-ui-display text-ink-strong">Contextual</h1>
            <p className="mt-0.5 text-ui-prose text-ink-muted">Pick files, chat, review diffs, save what you want.</p>
          </div>
        </div>

        <div className="grid grid-cols-[300px_1fr] gap-14">
          <div>
            <div className="mb-3 text-ui-caption text-ink-muted uppercase">Start</div>
            <Button variant="primary" size="lg" icon="folderOpen" kbd="Ctrl+O" onClick={openFolderDialog}>
              Open folder…
            </Button>
            <ol className="my-6 flex flex-col gap-2">
              {STEPS.map((s, i) => (
                <li key={s} className="flex items-center gap-3">
                  <span className="inline-flex size-5 flex-none items-center justify-center rounded-full border border-border-strong font-mono text-[11px] text-ink-muted">
                    {i + 1}
                  </span>
                  {s}
                </li>
              ))}
            </ol>
            <div className="flex items-center gap-2 rounded-md border border-border px-3 py-2 text-ui-small text-ink-muted">
              <Icon name="cloud" className={label.tone === 'ok' ? 'text-success' : label.tone === 'warn' ? 'text-warning' : ''} />
              <span title={label.detail} className="min-w-0 truncate">
                {foundry.state === 'ready' ? (
                  <>
                    Foundry · <b className="font-medium text-ink">{foundry.health.deployment}</b>
                  </>
                ) : (
                  label.text
                )}
              </span>
              <span className="flex-1" />
              <LinkButton onClick={openSettings}>Settings</LinkButton>
            </div>
          </div>

          <div>
            <div className="mb-3 text-ui-caption text-ink-muted uppercase">Recent</div>
            {recents.length === 0 && (
              <p className="mb-3 rounded-md border border-dashed border-border px-3 py-4 text-ui-small text-ink-muted">
                Folders you open appear here.
              </p>
            )}
            <div className={recents.length ? 'mb-3 overflow-hidden rounded-md border border-border' : 'hidden'}>
              {recents.map((r) => (
                <button key={r.path} type="button" onClick={() => void openFolder(r.path)} className={ROW}>
                  <FolderGlyph open={false} />
                  <span className="flex min-w-0 flex-1 flex-col leading-[18px]">
                    <b className="font-medium text-ink-strong">{r.name}</b>
                    <span className="truncate font-mono text-ui-small text-ink-muted">{r.path}</span>
                  </span>
                  <span className="w-20 text-right text-ui-small text-ink-muted">{ago(r.openedAt)}</span>
                </button>
              ))}
            </div>
            {recents.length > 0 && <LinkButton onClick={clearRecents}>Clear recent</LinkButton>}
          </div>
        </div>
      </div>
    </main>
  )
}
