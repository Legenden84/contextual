import type { ReactNode } from 'react'
import { Icon } from '@/components/ui/Icon'
import { describeFoundry } from '@/lib/foundry-status'
import { useApp } from '@/state/app-context'
import { UsagePopover } from './UsagePopover'

function Item({ children, className }: { children: ReactNode; className?: string }) {
  return <span className={`inline-flex items-center gap-1 [&_svg]:size-3.5 ${className ?? ''}`}>{children}</span>
}

export function StatusBar() {
  const { screen, settings, deployment, openSettings, foundry, project, scanning } = useApp()
  const label = describeFoundry(foundry)
  const tone = label.tone === 'ok' ? '[&_svg]:text-success' : label.tone === 'warn' ? 'text-warning' : ''
  const dep = settings.deployments[deployment] ?? settings.deployments[0]

  return (
    <footer className="flex h-6 flex-none items-center gap-4 border-t border-border bg-surface-000 px-3 text-ui-small text-ink-muted">
      {screen === 'welcome' ? (
        <>
          <span title={label.detail} className={`inline-flex items-center gap-1 [&_svg]:size-3.5 ${tone}`}>
            <Icon name="cloud" />
            {label.text}
          </span>
          <span className="flex-1" />
          <Item>No project</Item>
        </>
      ) : (
        <>
          <Item>
            <Icon name="branch" />
            {project?.branch ?? 'no git'}
          </Item>
          <span title={label.detail} className={`inline-flex items-center gap-1 [&_svg]:size-3.5 ${tone}`}>
            <Icon name="cloud" />
            {foundry.state === 'ready' ? `Foundry · ${dep.name}` : label.text}
          </span>
          <Item className="[&_svg]:text-success">
            <Icon name="check" />
            Read-only: nothing is written until you apply
          </Item>
          <span className="flex-1" />
          {scanning && (
            <Item>
              <Icon name="refresh" className="animate-spin" />
              Scanning…
            </Item>
          )}
          {screen === 'settings' ? (
            <Item>Saved</Item>
          ) : (
            <>
              <UsagePopover />
              <button type="button" onClick={openSettings} className="inline-flex cursor-pointer items-center gap-1 hover:text-ink [&_svg]:size-3.5">
                <Icon name="gear" />
                Settings
              </button>
            </>
          )}
        </>
      )}
    </footer>
  )
}
