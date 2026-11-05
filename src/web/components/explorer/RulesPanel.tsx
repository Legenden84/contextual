import { useRef, useState } from 'react'
import { CodeView } from '@/components/code/CodeView'
import { Button, IconButton } from '@/components/ui/Button'
import { FileGlyph } from '@/components/ui/FileGlyph'
import { Icon } from '@/components/ui/Icon'
import { Switch } from '@/components/ui/Switch'
import { Count, Tag } from '@/components/ui/Tag'
import { fmtK, splitPath } from '@/lib/format'
import { langFromPath } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

const PREVIEW_LINES = 9

/** Skills & Rules: auto-detected instruction files, each toggled into the system instructions. */
export function RulesPanel({ fill }: { fill?: boolean }) {
  const app = useApp()
  const [collapsed, setCollapsed] = useState(false)
  const fileInput = useRef<HTMLInputElement>(null)
  const onCount = app.rules.filter((r) => app.rulesOn.has(r.path)).length
  const preview = app.rules.find((r) => r.path === app.rulePreview)

  return (
    <div className={cn('border-t border-border pb-1.5', fill ? 'min-h-0 flex-1 overflow-auto border-t-0' : 'flex-none')}>
      <div className="flex h-8 items-center gap-1.5 pr-2 pl-1.5">
        <button
          type="button"
          className="inline-flex w-3.5 cursor-pointer text-ink-muted"
          aria-label={collapsed ? 'Expand Skills & Rules' : 'Collapse Skills & Rules'}
          onClick={() => setCollapsed((c) => !c)}
        >
          <Icon name={collapsed ? 'chevronRight' : 'chevronDown'} className="size-3.5" />
        </button>
        <span className="text-ui-caption text-ink-muted uppercase">Skills &amp; Rules</span>
        <Count>{onCount}</Count>
        <span className="flex-1" />
        <IconButton icon="plus" label="Import from file…" onClick={() => fileInput.current?.click()} />
      </div>

      {!collapsed && (
        <>
          {app.rules.map((r) => {
            const on = app.rulesOn.has(r.path)
            const { dir, name } = splitPath(r.path)
            return (
              <div
                key={r.path}
                onClick={() => app.previewRule(app.rulePreview === r.path ? null : r.path)}
                className={cn(
                  'flex h-6 cursor-pointer items-center gap-1.5 pr-3 pl-[22px] text-ui-small hover:bg-surface-hover',
                  app.rulePreview === r.path && 'bg-surface-selected hover:bg-surface-selected',
                )}
              >
                <FileGlyph name={r.path} />
                <span title={r.path} className={cn('min-w-0 flex-1 truncate font-mono text-[12px]', !on && 'text-ink-muted')}>
                  {name}
                  {dir && <span className="text-[11px] text-ink-muted"> {dir}</span>}
                </span>
                <Tag tone={r.type === 'Skill' ? 'skill' : 'default'}>{r.type}</Tag>
                <span className="flex-none font-mono text-[11px] text-ink-muted">{fmtK(r.tokens)}</span>
                <Switch on={on} label={`Include ${r.path}`} onChange={() => app.toggleRule(r.path)} />
              </div>
            )
          })}

          {preview && (
            <div className="mt-1 mr-3 mb-1.5 ml-[22px] overflow-hidden rounded-md border border-border">
              <CodeView
                compact
                guides={false}
                lang={langFromPath(preview.path) === 'plain' ? 'markdown' : langFromPath(preview.path)}
                code={preview.content.split('\n').slice(0, PREVIEW_LINES).join('\n')}
              />
            </div>
          )}

          <div className="flex items-center gap-2 pt-1.5 pr-3 pl-[22px]">
            <Button size="xs" icon="files" onClick={() => fileInput.current?.click()}>
              Import from file…
            </Button>
            <span className="truncate text-[11px] text-ink-muted">
              Detected: AGENTS.md, CLAUDE.md, .github, .cursor/rules, .claude/skills
            </span>
          </div>
        </>
      )}

      <input
        ref={fileInput}
        type="file"
        accept=".md,.mdc,.txt"
        className="hidden"
        onChange={(e) => {
          const f = e.target.files?.[0]
          if (f) app.importRule(f)
          e.target.value = ''
        }}
      />
    </div>
  )
}
