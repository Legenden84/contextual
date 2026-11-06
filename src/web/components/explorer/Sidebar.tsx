import { SourceControl } from '@/components/scm/SourceControl'
import { Icon } from '@/components/ui/Icon'
import { Count } from '@/components/ui/Tag'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'
import { FileTree } from './FileTree'
import { RulesPanel } from './RulesPanel'

/** Left column: Files (explorer + Skills & Rules), Skills & Rules on its own, or Source Control. */
export function Sidebar() {
  const { rules, rulesOn, sidebarTab: tab, setSidebarTab: setTab, git } = useApp()
  const onCount = rules.filter((r) => rulesOn.has(r.path)).length
  const changeCount = git ? new Set([...git.staged, ...git.changes, ...git.conflicts].map((e) => e.path)).size : 0

  const tabClass = (active: boolean) =>
    cn(
      'inline-flex h-[30px] cursor-pointer items-center gap-1.5 border-b-2 border-transparent px-2 text-ui-small whitespace-nowrap text-ink-muted',
      active && 'border-accent text-ink-strong',
    )

  return (
    <aside className="flex min-h-0 w-[312px] flex-none flex-col border-r border-border bg-surface-000">
      <div role="tablist" className="flex gap-0.5 border-b border-border px-2 pt-1.5">
        <button type="button" role="tab" aria-selected={tab === 'files'} className={tabClass(tab === 'files')} onClick={() => setTab('files')}>
          <Icon name="files" />
          Files
        </button>
        <button type="button" role="tab" aria-selected={tab === 'rules'} className={tabClass(tab === 'rules')} onClick={() => setTab('rules')}>
          <Icon name="book" />
          Rules
          <Count>{onCount}</Count>
        </button>
        <button type="button" role="tab" aria-selected={tab === 'scm'} className={tabClass(tab === 'scm')} onClick={() => setTab('scm')}>
          <Icon name="branch" />
          Source control
          {changeCount > 0 && <Count>{changeCount}</Count>}
        </button>
      </div>
      {tab === 'files' ? (
        <>
          <FileTree />
          <RulesPanel />
        </>
      ) : tab === 'rules' ? (
        <RulesPanel fill />
      ) : (
        <SourceControl />
      )}
    </aside>
  )
}
