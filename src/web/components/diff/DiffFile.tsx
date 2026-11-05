import { useMemo, useState } from 'react'
import type { Change } from '@/data/types'
import { Button, IconButton } from '@/components/ui/Button'
import { Checkbox, CheckLabel } from '@/components/ui/Checkbox'
import { Icon } from '@/components/ui/Icon'
import { StatBar, StatusLetter } from '@/components/ui/Status'
import { Tag } from '@/components/ui/Tag'
import { useCopy } from '@/hooks/useCopy'
import { computeDiff, unifiedPatch } from '@/lib/diff'
import { splitPath } from '@/lib/format'
import { langFromPath } from '@/lib/syntax'
import { cn } from '@/lib/utils'
import { DiffTable } from './DiffTable'

export interface DiffFileProps {
  change: Change
  include: boolean
  onToggleInclude: () => void
  mode: 'unified' | 'split'
  defaultCollapsed?: boolean
  expandAll?: boolean
}

/** One file in a GitHub-style diff: header, optional conflict banner, body. */
export function DiffFile({ change, include, onToggleInclude, mode, defaultCollapsed, expandAll }: DiffFileProps) {
  const diff = useMemo(() => computeDiff(change.old, change.new), [change])
  const [collapsedState, setCollapsed] = useState(!!defaultCollapsed)
  const collapsed = expandAll ? false : collapsedState
  const [copiedFile, copyFile] = useCopy()
  const [copiedDiff, copyDiff] = useCopy()
  const { dir, name } = splitPath(change.path)

  return (
    <section
      id={`diff-${change.path}`}
      className={cn(
        'mb-3 scroll-mt-3 overflow-hidden rounded-md border border-border bg-surface-100',
        change.conflict && 'border-warning-border',
      )}
    >
      <header
        className={cn(
          'sticky top-0 z-1 flex h-10 items-center gap-2 bg-surface-200 pr-3 pl-2',
          !collapsed && 'border-b border-border',
        )}
      >
        <IconButton
          icon={collapsed ? 'chevronRight' : 'chevronDown'}
          label={collapsed ? 'Expand file' : 'Collapse file'}
          onClick={() => setCollapsed((c) => !c)}
        />
        <StatusLetter status={change.status} />
        <span className="min-w-0 truncate font-mono text-[12px] font-semibold text-ink-strong">
          <span className="font-normal text-ink-muted">{dir}</span>
          {name}
        </span>
        {change.status === 'A' && <Tag>new file</Tag>}
        {change.status === 'D' && <Tag tone="danger">deleted</Tag>}
        {change.conflict && (
          <Tag tone="warn">
            <Icon name="warning" />
            conflict
          </Tag>
        )}
        <StatBar adds={diff.adds} dels={diff.dels} />
        <span className="flex-1" />
        {change.status !== 'D' && (
          <Button variant="ghost" size="xs" icon="copy" onClick={() => copyFile(change.new)}>
            {copiedFile ? 'Copied' : 'Copy file content'}
          </Button>
        )}
        <Button variant="ghost" size="xs" icon="copy" onClick={() => copyDiff(unifiedPatch(change.path, diff, change.status))}>
          {copiedDiff ? 'Copied' : 'Copy diff'}
        </Button>
        <CheckLabel
          className="text-ui-body"
          onToggle={onToggleInclude}
          control={<Checkbox state={include} onChange={onToggleInclude} label="Include in apply" />}
        >
          Include in apply
        </CheckLabel>
      </header>
      {change.conflict && !collapsed && (
        <div className="flex items-center gap-2 border-b border-warning-border bg-warning-bg px-3 py-1.5 text-ui-small text-ink">
          <Icon name="warning" className="text-warning" />
          <div>
            <b className="font-semibold">Conflict: file modified on disk since context was loaded.</b>{' '}
            <span className="text-ink-muted">{change.conflict}</span>
          </div>
          <span className="flex-1" />
          <Button size="xs" icon="eye">
            Compare with disk
          </Button>
          <Button size="xs" icon="refresh">
            Reload &amp; resend
          </Button>
        </div>
      )}
      {!collapsed && (
        <div className="overflow-x-auto">
          <DiffTable diff={diff} lang={langFromPath(change.path)} mode={mode} expandAll={expandAll} />
        </div>
      )}
    </section>
  )
}
