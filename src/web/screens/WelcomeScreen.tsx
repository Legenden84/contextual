import { useEffect } from 'react'
import { Logo } from '@/components/shell/Logo'
import { Button, LinkButton } from '@/components/ui/Button'
import { Icon } from '@/components/ui/Icon'
import { FolderGlyph } from '@/components/ui/FileGlyph'
import { Tag } from '@/components/ui/Tag'
import { recentProjects } from '@/data/sample'
import { describeFoundry } from '@/lib/foundry-status'
import { useApp } from '@/state/app-context'

const STEPS = [
  'Tick the files the model should see',
  'Ask for a change in chat',
  'Review the diff, file by file',
  'Apply what you want. The model never writes to disk.',
]

/** Empty state: no project open. */
export function WelcomeScreen() {
  const { openProject, openSettings, foundry } = useApp()
  const label = describeFoundry(foundry)

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'o') {
        e.preventDefault()
        openProject()
      }
    }
    document.addEventListener('keydown', onKey)
    return () => document.removeEventListener('keydown', onKey)
  }, [openProject])

  return (
    <main className="flex flex-1 items-center justify-center overflow-auto bg-surface-100 p-8">
      <div className="w-[860px]">
        <div className="mb-8 flex items-center gap-4">
          <Logo size={40} />
          <div>
            <h1 className="text-ui-display text-ink-strong">Contextual</h1>
            <p className="mt-0.5 text-ui-prose text-ink-muted">Pick files, chat, review diffs, apply what you want.</p>
          </div>
        </div>

        <div className="grid grid-cols-[300px_1fr] gap-14">
          <div>
            <div className="mb-3 text-ui-caption text-ink-muted uppercase">Start</div>
            <Button variant="primary" size="lg" icon="folderOpen" kbd="Ctrl+O" onClick={openProject}>
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
            <div className="mb-3 overflow-hidden rounded-md border border-border">
              {recentProjects.map((r) => (
                <button
                  key={r.path}
                  type="button"
                  onClick={openProject}
                  className="flex w-full cursor-pointer items-center gap-3 border-b border-border px-3 py-2 text-left last:border-b-0 hover:bg-surface-hover"
                >
                  <FolderGlyph open={false} />
                  <span className="flex flex-1 flex-col leading-[18px]">
                    <b className="font-medium text-ink-strong">{r.name}</b>
                    <span className="font-mono text-ui-small text-ink-muted">{r.path}</span>
                  </span>
                  <Tag>{r.stack}</Tag>
                  <span className="w-20 text-right text-ui-small text-ink-muted">{r.when}</span>
                </button>
              ))}
            </div>
            <LinkButton>Clear recent</LinkButton>
          </div>
        </div>
      </div>
    </main>
  )
}
