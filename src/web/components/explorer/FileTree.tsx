import type { TreeNode } from '@/data/sample'
import { IconButton, LinkButton } from '@/components/ui/Button'
import { Checkbox, CheckLabel } from '@/components/ui/Checkbox'
import { FileGlyph, FolderGlyph } from '@/components/ui/FileGlyph'
import { Icon } from '@/components/ui/Icon'
import { Switch } from '@/components/ui/Switch'
import { Tag } from '@/components/ui/Tag'
import { TextInput } from '@/components/ui/TextInput'
import { fmtK } from '@/lib/format'
import { checkState, filterTree, isOverridable, nodeStats, sortNodes } from '@/lib/tree'
import { cn } from '@/lib/utils'
import { useApp } from '@/state/app-context'

/** Files above this many tokens are flagged in the tree when selected. */
const BIG_FILE = 10000

function TreeRow({ node, depth, forceOpen }: { node: TreeNode; depth: number; forceOpen: boolean }) {
  const app = useApp()
  const isDir = node.kind === 'dir'
  const open = isDir && (forceOpen || app.open.has(node.path)) && !!node.children?.length
  const stats = nodeStats(node, app.selected)
  const state = checkState(node, app.selected)
  const fileSelected = !isDir && app.selected.has(node.path)
  const active = !isDir && app.centerView === 'chat' && app.activeTab === node.path
  // Too large for the size limit, but can be ticked on its own.
  const optIn = isOverridable(node)
  const blocked = !!node.excluded && !optIn

  const tokens = blocked ? '' : isDir ? (stats.selectedTokens ? fmtK(stats.selectedTokens) : '') : fmtK(node.tokens ?? 0)
  const tokenTone =
    (isDir && stats.selectedTokens) || fileSelected
      ? fileSelected && (node.tokens ?? 0) > BIG_FILE
        ? 'font-semibold text-warning'
        : 'text-ink-muted'
      : 'text-ink-faint'

  const onRowClick = () => {
    if (isDir) app.toggleOpen(node.path)
    else if (!blocked) app.openPreview(node.path)
  }

  return (
    <>
      <div
        role="treeitem"
        aria-expanded={isDir ? open : undefined}
        aria-selected={active}
        title={node.excluded ? `Excluded — ${node.excluded}` : node.path}
        onClick={onRowClick}
        style={{ paddingLeft: 6 + depth * 12 }}
        className={cn(
          'group relative flex h-row cursor-default items-center gap-[5px] pr-3 text-ui-body whitespace-nowrap text-ink hover:bg-surface-hover',
          (blocked || (optIn && !fileSelected)) && 'text-ink-faint',
          active && 'bg-surface-selected shadow-[inset_0_0_0_1px_var(--focus-ring)] hover:bg-surface-selected',
        )}
      >
        {depth > 0 && (
          <span
            className="pointer-events-none absolute inset-y-0 left-1.5 bg-[linear-gradient(to_right,var(--editor-indent-guide)_1px,transparent_1px)] bg-[length:12px_100%] bg-[position:7px_0]"
            style={{ width: depth * 12 }}
          />
        )}
        <span className="inline-flex w-3.5 flex-none text-ink-muted">
          {isDir && <Icon name={open ? 'chevronDown' : 'chevronRight'} className="size-3.5" />}
        </span>
        <Checkbox
          state={state}
          disabled={blocked}
          label={optIn ? `Include ${node.name} anyway (over the size limit)` : `Include ${node.name}`}
          onChange={() => (isDir ? app.toggleFolder(node) : app.toggleFile(node.path))}
        />
        {isDir ? (
          <FolderGlyph open={open} className={cn(node.excluded && 'opacity-45')} />
        ) : (
          <FileGlyph name={node.name} className={cn(blocked && 'opacity-45')} />
        )}
        <span className="truncate">{node.name}</span>
        {node.changedOnDisk && (
          <span className="inline-flex text-warning" title="Changed on disk since it was loaded into context">
            <Icon name="refresh" className="size-3" />
          </span>
        )}
        {node.image && !blocked && (
          <span className="inline-flex text-ink-muted" title="Sent to the model as a picture">
            <Icon name="eye" className="size-3" />
          </span>
        )}
        {node.condensedFrom && (
          <Tag title={`Sent condensed: embedded images, fonts and minified code are left out (~${fmtK(node.condensedFrom)} tokens in full). Read-only for the model.`}>
            condensed
          </Tag>
        )}
        {blocked && <Tag tone="excluded">excluded</Tag>}
        {optIn && (
          <Tag tone="warn" title={`${node.excluded}. Over Max file size in Settings; tick to include anyway.`}>
            too large
          </Tag>
        )}
        <span className="flex-1" />
        {blocked && (
          <span className="pointer-events-none absolute -top-[26px] right-16 z-3 hidden rounded-sm border border-border-strong bg-surface-300 px-2 py-0.5 text-[11px] text-ink shadow-popover group-hover:block">
            {node.excluded}
          </span>
        )}
        <span className={cn('flex-none font-mono text-[11px]', tokenTone)}>{tokens}</span>
      </div>
      {open &&
        sortNodes(node.children!).map((c) => <TreeRow key={c.path} node={c} depth={depth + 1} forceOpen={forceOpen} />)}
    </>
  )
}

/** Explorer: tri-state checkboxes, token counts, exclusions and changed-on-disk badges. */
export function FileTree() {
  const app = useApp()
  const nodes = filterTree(app.tree, app.filter)
  const filtering = !!app.filter.trim()
  let files = 0
  let tokens = 0
  for (const n of app.tree) {
    const s = nodeStats(n, app.selected)
    files += s.selected
    tokens += s.selectedTokens
  }

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex h-8 flex-none items-center gap-1.5 pr-2 pl-3">
        <span className="text-ui-caption text-ink-muted uppercase">Explorer</span>
        <span className="flex-1" />
        <IconButton icon="refresh" label="Rescan folder" onClick={app.rescan} />
        <IconButton icon="dots" label="More" />
      </div>
      <div className="flex flex-col gap-1.5 px-3 pb-1.5">
        <TextInput
          size="sm"
          icon="search"
          placeholder="Filter files"
          value={app.filter}
          onChange={(e) => app.setFilter(e.target.value)}
        />
        <div className="flex items-center gap-3">
          <LinkButton onClick={app.selectAll}>Select all</LinkButton>
          <LinkButton onClick={app.clearSelection}>Clear</LinkButton>
          <span className="flex-1" />
          <CheckLabel
            className="text-ui-small"
            onToggle={() => app.updateSettings({ respectGitignore: !app.settings.respectGitignore })}
            control={
              <Switch
                on={app.settings.respectGitignore}
                label="Respect .gitignore"
                onChange={() => app.updateSettings({ respectGitignore: !app.settings.respectGitignore })}
              />
            }
          >
            Respect .gitignore
          </CheckLabel>
        </div>
      </div>
      <div className="flex h-row items-center gap-1 border-t border-border pr-3 pl-1.5 text-[11px] tracking-[0.04em] text-ink uppercase">
        <span className="inline-flex w-3.5 text-ink-muted">
          <Icon name="chevronDown" className="size-3.5" />
        </span>
        <b className="truncate font-semibold" title={app.project?.root}>
          {app.project?.name}
        </b>
        <span className="flex-1" />
        <span className="font-mono text-[11px] tracking-normal text-ink-muted normal-case">
          {files} files · {fmtK(tokens)}
        </span>
      </div>
      {app.stale.size > 0 && (
        <div className="mx-2 mt-0.5 mb-1 flex items-center gap-1.5 rounded-sm bg-warning-bg px-2 py-[3px] text-ui-small text-ink">
          <Icon name="refresh" className="size-3.5 text-warning" />
          <span>
            {app.stale.size} {app.stale.size === 1 ? 'file' : 'files'} changed on disk
          </span>
          <span className="flex-1" />
          <LinkButton onClick={app.refreshStale}>Refresh</LinkButton>
        </div>
      )}
      {app.project?.truncated && (
        <p className="mx-2 mb-1 rounded-sm bg-warning-bg px-2 py-[3px] text-ui-small text-ink">
          Large folder: only the first 20,000 entries are listed. Add exclusions in Settings.
        </p>
      )}
      <div role="tree" aria-label="Project files" className="min-h-0 flex-1 overflow-auto pb-1.5">
        {sortNodes(nodes).map((n) => (
          <TreeRow key={n.path} node={n} depth={0} forceOpen={filtering} />
        ))}
        {filtering && nodes.length === 0 && (
          <p className="px-4 py-2 text-ui-small text-ink-muted">No files match “{app.filter}”.</p>
        )}
      </div>
    </div>
  )
}
