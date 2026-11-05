import { useEffect, useMemo, useState } from 'react'
import { DiffFile } from '@/components/diff/DiffFile'
import { Button, IconButton } from '@/components/ui/Button'
import { Checkbox } from '@/components/ui/Checkbox'
import { Icon } from '@/components/ui/Icon'
import { Segmented } from '@/components/ui/Segmented'
import { LineCounts, StatBar, StatusLetter } from '@/components/ui/Status'
import { computeDiff } from '@/lib/diff'
import { splitPath } from '@/lib/format'
import { cn } from '@/lib/utils'
import { useApp, type DiffMode } from '@/state/app-context'

/** Sample response: the new command and the deletion start folded, as in the design. */
const START_COLLAPSED = new Set([1, 2, 4])

/** Full-width diff for one response, replacing the chat and Changes columns. */
export function DiffScreen() {
  const { changes, response, include, toggleInclude, diffMode, setDiffMode, backToChat, openApply, diffFocus } = useApp()
  const [expandAll, setExpandAll] = useState(false)
  const [showListPref, setShowList] = useState<boolean | null>(null)
  const showList = showListPref ?? diffMode !== 'split'
  const [active, setActive] = useState(diffFocus ?? changes[0]?.path)

  const stats = useMemo(() => changes.map((c) => computeDiff(c.old, c.new)), [changes])
  const adds = stats.reduce((a, d) => a + d.adds, 0)
  const dels = stats.reduce((a, d) => a + d.dels, 0)
  const selected = changes.filter((c) => include[c.path]).length

  const jump = (path: string) => {
    setActive(path)
    document.getElementById(`diff-${path}`)?.scrollIntoView({ block: 'start', behavior: 'smooth' })
  }

  useEffect(() => {
    if (diffFocus) document.getElementById(`diff-${diffFocus}`)?.scrollIntoView({ block: 'start' })
  }, [diffFocus])

  return (
    <main className="flex min-w-0 flex-1 flex-col bg-surface-100">
      <div className="flex h-11 flex-none items-center gap-2 border-b border-border bg-surface-000 pr-3 pl-2 text-ui-body whitespace-nowrap">
        <Button variant="ghost" size="xs" icon="back" onClick={backToChat}>
          Back to chat
        </Button>
        <IconButton icon="files" label="Toggle file list" active={showList} onClick={() => setShowList(!showList)} />
        <span className="mx-1 h-[18px] w-px bg-border-strong" />
        <b className="font-semibold text-ink-strong">Response #{response?.number}</b>
        <span className="text-ink-muted">
          {response?.model} · {response?.time}
        </span>
        <span className="text-ink-muted">·</span>
        <span>{changes.length} files changed</span>
        <StatBar adds={adds} dels={dels} />
        <span className="flex-1" />
        <Segmented<DiffMode>
          label="Diff layout"
          value={diffMode}
          onChange={(m) => {
            setDiffMode(m)
            setShowList(null)
          }}
          options={[
            { value: 'unified', label: 'Unified', icon: 'unified' },
            { value: 'split', label: 'Split', icon: 'split' },
          ]}
        />
        <Button variant="ghost" size="xs" icon="unfold" aria-pressed={expandAll} onClick={() => setExpandAll((e) => !e)}>
          {expandAll ? 'Collapse unchanged' : 'Expand all'}
        </Button>
        <Button variant="primary" disabled={selected === 0} onClick={openApply}>
          Apply {selected} {selected === 1 ? 'file' : 'files'}…
        </Button>
      </div>

      <div className="flex min-h-0 flex-1">
        {showList && (
          <nav className="w-[236px] flex-none overflow-auto border-r border-border pb-2">
            <div className="flex h-8 items-center pl-3">
              <span className="text-ui-caption text-ink-muted uppercase">Files</span>
            </div>
            {changes.map((c, i) => (
              <div
                key={c.path}
                onClick={() => jump(c.path)}
                title={c.path}
                className={cn(
                  'flex h-7 cursor-pointer items-center gap-1.5 px-3 text-ui-small hover:bg-surface-hover',
                  active === c.path && 'bg-surface-selected hover:bg-surface-selected',
                )}
              >
                <Checkbox state={!!include[c.path]} label={`Include ${c.path}`} onChange={() => toggleInclude(c.path)} />
                <StatusLetter status={c.status} />
                <span className={cn('min-w-0 truncate', include[c.path] ? 'text-ink' : 'text-ink-muted')}>{splitPath(c.path).name}</span>
                {c.conflict && <Icon name="warning" className="size-[13px] text-warning" />}
                <span className="flex-1" />
                <LineCounts adds={stats[i].adds} dels={stats[i].dels} className="font-normal" />
              </div>
            ))}
          </nav>
        )}
        <div className="min-w-0 flex-1 overflow-auto px-4 py-3">
          {changes.map((c, i) => (
            <DiffFile
              key={(response?.id ?? '') + c.path}
              change={c}
              include={!!include[c.path]}
              onToggleInclude={() => toggleInclude(c.path)}
              mode={diffMode}
              defaultCollapsed={
                diffFocus ? c.path !== diffFocus : response?.id === 'a1' ? START_COLLAPSED.has(i) : changes.length > 3 && i > 0
              }
              expandAll={expandAll}
            />
          ))}
        </div>
      </div>
    </main>
  )
}
