import { FileGlyph } from '@/components/ui/FileGlyph'
import { Icon } from '@/components/ui/Icon'
import { Tag } from '@/components/ui/Tag'
import { splitPath } from '@/lib/format'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

const TAB = 'relative flex cursor-pointer items-center gap-1.5 border-r border-border px-3 text-ui-body text-ink-muted'
const ACTIVE =
  'bg-surface-100 text-ink-strong before:absolute before:inset-x-0 before:top-0 before:h-0.5 before:bg-accent after:absolute after:inset-x-0 after:-bottom-px after:h-px after:bg-surface-100'

/** Centre tabs: Chat plus read-only file previews. */
export function Tabs() {
  const { previewTabs, activeTab, setActiveTab, closeTab } = useApp()
  return (
    <div role="tablist" className="flex h-9 flex-none border-b border-border bg-surface-000">
      <div role="tab" aria-selected={activeTab === 'chat'} onClick={() => setActiveTab('chat')} className={cn(TAB, activeTab === 'chat' && ACTIVE)}>
        <Icon name="sparkle" className="size-3.5 text-accent-ink" />
        <span>Chat</span>
      </div>
      {previewTabs.map((path) => {
        const name = splitPath(path).name
        const active = activeTab === path
        return (
          <div key={path} role="tab" aria-selected={active} title={path} onClick={() => setActiveTab(path)} className={cn(TAB, active && ACTIVE)}>
            <FileGlyph name={name} />
            <span>{name}</span>
            <Tag>read-only</Tag>
            <button
              type="button"
              aria-label={`Close ${name}`}
              onClick={(e) => {
                e.stopPropagation()
                closeTab(path)
              }}
              className="inline-flex cursor-pointer rounded-sm text-ink-muted hover:bg-surface-hover hover:text-ink"
            >
              <Icon name="close" className="size-3.5" />
            </button>
          </div>
        )
      })}
      <span className="flex-1" />
    </div>
  )
}
