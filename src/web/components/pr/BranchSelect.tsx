import { useEffect, useRef, useState } from 'react'
import { Icon } from '@/components/ui/Icon'
import { cn } from '@/lib/utils'

/** GitHub-style branch dropdown: "base: main ▾" opening a filterable list. */
export function BranchSelect({
  label,
  value,
  options,
  onChange,
  disabled,
}: {
  label: string
  value: string
  options: string[]
  onChange: (branch: string) => void
  disabled?: boolean
}) {
  const [open, setOpen] = useState(false)
  const [query, setQuery] = useState('')
  const root = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const close = (e: MouseEvent) => !root.current?.contains(e.target as Node) && setOpen(false)
    document.addEventListener('mousedown', close)
    return () => document.removeEventListener('mousedown', close)
  }, [open])

  const q = query.trim().toLowerCase()
  const shown = options.filter((o) => !q || o.toLowerCase().includes(q))

  const pick = (b: string) => {
    setOpen(false)
    setQuery('')
    if (b !== value) onChange(b)
  }

  return (
    <div ref={root} className="relative">
      <button
        type="button"
        disabled={disabled}
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((o) => !o)}
        className="inline-flex h-6 max-w-[320px] cursor-pointer items-center gap-1 rounded-md border border-border-strong bg-surface-300 px-2 text-ui-small text-ink hover:bg-surface-hover disabled:cursor-default disabled:opacity-60"
      >
        <span className="text-ink-muted">{label}:</span>
        <span className="truncate font-mono">{value || '…'}</span>
        <Icon name="chevronDown" className="size-3 flex-none text-ink-muted" />
      </button>
      {open && (
        <div className="absolute top-full left-0 z-20 mt-1 w-[320px] overflow-hidden rounded-md border border-border-strong bg-surface-200 shadow-popover">
          <div className="border-b border-border px-2 py-1.5 text-ui-small font-medium text-ink">Choose a {label} branch</div>
          <div className="p-1.5">
            <input
              autoFocus
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === 'Escape') setOpen(false)
                if (e.key === 'Enter' && shown[0]) pick(shown[0])
              }}
              placeholder="Find a branch"
              aria-label={`Find a ${label} branch`}
              className="h-6 w-full rounded-sm border border-border-strong bg-surface-100 px-2 text-ui-small text-ink outline-none focus:border-focus-ring placeholder:text-ink-muted"
            />
          </div>
          <div role="listbox" className="max-h-64 overflow-auto pb-1">
            {shown.map((b) => (
              <button
                key={b}
                type="button"
                role="option"
                aria-selected={b === value}
                onClick={() => pick(b)}
                className={cn('flex h-[26px] w-full cursor-pointer items-center gap-2 px-2 text-left text-ui-small hover:bg-surface-hover', b === value && 'text-ink-strong')}
              >
                <span className="w-3.5 flex-none">{b === value && <Icon name="check" className="size-3.5 text-accent-ink" />}</span>
                <span className="truncate font-mono">{b}</span>
              </button>
            ))}
            {!shown.length && <p className="px-2 py-1 text-ui-small text-ink-muted">Nothing to show</p>}
          </div>
        </div>
      )}
    </div>
  )
}
